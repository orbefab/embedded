# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Decode the I2C bus in a run folder's logic.vcd, apart from the engine.

    uv run packages/engine/scripts/i2c_decode.py RUN_DIR [--part NAME] [--timing] [--timing-s3 HZ]

Each `atmega328p` part in RUN_DIR/netlist.json has a view of the bus: its own `PC5` as SCL
and `PC4` as SDA, the levels its inputs read. For each such view the script writes one line per
phase, as `traces/i2c.csv` names them, and compares the decoding of each master's view with
that master's rows in `traces/i2c.csv`. A row matches when its phase, address, read, byte and
ack are equal and its time is within one CPU cycle. The script prints how many rows match and
how many of those match to the picosecond. Every view must decode to the same phases, though
their times may differ by the swing between the parts' thresholds. `--part` limits the
comparison to one part.

Each `cst816` part is a view of its own `SCL` and `SDA`. Each `esp32s3` is a view of its GPIO
wires on the nets one `cst816`'s `SCL` and `SDA` sit on, one view per controller. The script
opens `traces/i2c.csv` only when an `atmega328p` view is decoded. Views with no rows are
compared with each other. When an `atmega328p` view is decoded, another view that decodes to
the same phases adds no line, and one that does not is printed and fails the run, so an
ATmega run that already agrees prints what it printed before.

A phase starts at the first bus change after the end of the phase before it. START and a
repeated START end at their SCL fall, an address or data byte at its ninth SCL fall, and STOP
at its SDA rise. A byte that follows a START is in progress from its first SCL high, so an
SDA change on that high is a bus error, as is one on any later high of the byte. The SCL high
after a finished byte is not a bit. SDA rising there is STOP and SDA falling there is a
repeated START. The decoder cannot separate bit 0 of the next data byte from those two. A
bus error in that high makes it exit 1. An arbitration loss on that bit makes it exit 1 only
when the winner changes SDA during that high.

A master that lost arbitration has an `arbitration_lost` row and no rows after it until its
next START. That row matches the byte that starts at its time, an address, a data byte, or
a bus error. An address or a data byte matches on that time, which is the byte's first bus
change. A loss needs a low read, so a bus error with none cannot be one. The read is an SCL
rise on bits 7 to 0 of an address, on bits 7 to 0 of a write data byte, or on the ACK bit
of a read data byte. A low read does not show a loss. A decoded phase before the row is a
mismatch. A START or a STOP at that time does not match. The decoded phases up to the next
START are the winner's.

`--timing` lists, per view, each interval of DS40002061B Table 29-14 with its count, its least
value, and every value below the table's minimum for the measured rate. The rate is the
100 kHz column when the median SCL period is 10 us or longer and the 400 kHz column otherwise.
The rise and fall rows are not checked. tHD;DAT's maximum holds only on an SCL low no longer
than 1.5 times the median low, because a master that holds SCL low, as one does while `TWINT`
is set, need not meet it.

`--timing-s3 HZ` checks each `esp32s3` and `cst816` view against the lengths an ESP-IDF master
writes at `HZ`, 100000 or 400000, to the picosecond. tLOW, tHIGH, tSU;STA, tHD;STA and tSU;STO
are those lengths. Each tHD;DAT is the master's SDA hold or the `cst816`'s 300 ns, because both
parts' edges are on the wires the view reads, and each tSU;DAT is tLOW minus that hold. At
400 kHz the two holds are both 300 ns. An empty interval is not a mismatch, and tBUF is not
checked. The ATmega table is not applied to these views. Another `HZ` exits 1. The lengths
read IDF's register values as the model does, five of them as their register plus one, so
`--timing-s3` checks the wire against that reading and is not an independent check of it.

Exits 1 when a view's phases differ from another view's, or from `traces/i2c.csv`, or when
`--timing` finds an interval out of the table, or when `--timing-s3` finds an interval off
its length.
"""

import argparse
import csv
import json
import statistics
import sys
from pathlib import Path

# DS40002061B Table 29-14, in picoseconds: (100 kHz column, 400 kHz column).
TABLE = {
    "tHD;STA": (4_000_000, 600_000),
    "tLOW": (4_700_000, 1_300_000),
    "tHIGH": (4_000_000, 600_000),
    "tSU;STA": (4_700_000, 600_000),
    "tHD;DAT": (0, 0),
    "tSU;DAT": (250_000, 100_000),
    "tSU;STO": (4_000_000, 600_000),
    "tBUF": (4_700_000, 1_300_000),
}
HOLD_MAX = (3_450_000, 900_000)


def read_vcd(path):
    """`{(part, pin): [(t_ps, level)]}`, the initial level at time 0 first."""
    tokens = Path(path).read_text().split()
    names = {}
    out = {}
    scope = None
    at = 0
    k = 0
    while k < len(tokens):
        token = tokens[k]
        if token == "$scope":
            scope = tokens[k + 2]
            k += 4
        elif token == "$upscope":
            k += 2
        elif token == "$var":
            names[tokens[k + 3]] = (scope, tokens[k + 4])
            k += 6
        elif token == "$timescale":
            assert tokens[k + 1 : k + 3] == ["1", "ps"], "the timescale is 1 ps"
            k = tokens.index("$end", k) + 1
        elif token in ("$enddefinitions", "$dumpvars", "$end"):
            k += 1
        elif token.startswith("#"):
            at = int(token[1:])
            k += 1
        else:
            out.setdefault(names[token[1:]], []).append((at, token[0] == "1"))
            k += 1
    return out


def merged(scl, sda):
    """Every edge of both lines in time order: `(t_ps, line, high)`, SDA first at one time."""
    edges = [(t, "scl", high) for t, high in scl[1:]] + [(t, "sda", high) for t, high in sda[1:]]
    edges.sort(key=lambda edge: (edge[0], edge[1] != "sda"))
    return scl[0][1], sda[0][1], edges


def decode(scl, sda):
    """The phases of one view: dicts with `t_ps`, `end`, `phase`, and the byte fields."""
    scl_high, sda_high, edges = merged(scl, sda)
    phases = []
    ended = None
    first = None
    in_frame = False
    pending = None  # "start" or "repeated_start" until its SCL fall
    follows_start = False  # the next SCL rise opens the byte a START requires
    clocked = False  # SCL rose after a finished byte; the high is STOP, repeated START, or bit 0
    clocked_at = None  # that rise, the first SCL rise of the data byte if the fall keeps it
    held = False  # SDA at that rise
    byte = None  # [bits, value, ack], set only once the rise is a bit
    byte_rise = None  # first SCL rise of the open byte
    byte_low = False  # SDA read low on a bit the master sends
    open_byte = False
    address = None  # the frame's (address, read)

    def on_a_written_bit(index):
        if address is None or address[1] == 0:
            return index < 8
        return index == 8

    def emit(at, phase, **fields):
        nonlocal ended, first, byte_rise, byte_low
        row = {"t_ps": first if first is not None else at, "end": at, "phase": phase, **fields}
        if phase in ("address", "data", "bus_error"):
            row["rise"] = byte_rise
            row["read_low"] = byte_low
            byte_rise = None
            byte_low = False
        phases.append(row)
        ended = at
        first = None

    for at, line, high in edges:
        if (line == "scl" and high == scl_high) or (line == "sda" and high == sda_high):
            continue
        if first is None and (ended is None or at > ended):
            first = at
        if line == "sda":
            sda_high = high
            if not scl_high:
                continue
            if open_byte:
                emit(at, "bus_error")
                byte = None
                open_byte = False
                follows_start = False
                clocked = False
                in_frame = False
                address = None
                pending = None
                continue
            if clocked:
                # The rise after a finished byte is not bit 0 yet.
                clocked = False
                if high:
                    emit(at, "stop")
                    in_frame = False
                    address = None
                else:
                    pending = "repeated_start"
                    in_frame = True
                    address = None
                continue
            if high:
                emit(at, "stop")
                in_frame = False
                address = None
            else:
                pending = "repeated_start" if in_frame else "start"
                in_frame = True
                address = None
            continue
        scl_high = high
        if high:
            if not in_frame or pending:
                continue
            if follows_start or open_byte:
                if byte is None:
                    byte = [0, 0, False]
                    byte_rise = at
                    byte_low = False
                if not sda_high and on_a_written_bit(byte[0]):
                    byte_low = True
                if byte[0] < 8:
                    byte[1] = (byte[1] << 1) | int(sda_high)
                else:
                    byte[2] = not sda_high
                byte[0] += 1
                open_byte = True
                follows_start = False
                continue
            clocked = True
            clocked_at = at
            held = sda_high
            continue
        if pending:
            emit(at, pending)
            pending = None
            follows_start = True
            continue
        if clocked:
            byte = [1, int(held), False]
            byte_rise = clocked_at
            byte_low = (not held) and on_a_written_bit(0)
            open_byte = True
            clocked = False
            continue
        if byte is not None and byte[0] == 9:
            value, ack = byte[1], byte[2]
            if address is None:
                address = (value >> 1, value & 1)
                emit(at, "address", address=value >> 1, read=value & 1, byte="", ack=int(ack))
            else:
                emit(at, "data", address="", read=address[1], byte=value, ack=int(ack))
            byte = None
            open_byte = False
    return phases


def fields(row):
    def cell(value):
        return "" if value in ("", None) else int(value)

    return (row["phase"], cell(row.get("address")), cell(row.get("read")), cell(row.get("byte")), cell(row.get("ack")))


def compare(decoded, rows, cycle_ps):
    """`(matched, exact, problems)` of a master's rows against its view."""
    matched = exact = 0
    problems = []
    k = 0
    for n, row in enumerate(rows):
        t = int(row["t_ps"])
        if row["phase"] == "arbitration_lost":
            if k == len(decoded):
                problems.append(f"row {n + 1}: no decoded phase starts at the arbitration loss at {t}")
                return matched, exact, problems
            phase = decoded[k]
            if phase["t_ps"] + cycle_ps < t:
                problems.append(f"row {n + 1}: arbitration loss at {t} skips {phase['phase']} at {phase['t_ps']}")
                return matched, exact, problems
            if abs(phase["t_ps"] - t) > cycle_ps:
                problems.append(f"row {n + 1}: no decoded phase starts at the arbitration loss at {t}")
                return matched, exact, problems
            if phase["phase"] not in ("address", "data", "bus_error"):
                problems.append(f"row {n + 1}: arbitration loss at {t} matches a {phase['phase']}, not a byte")
                return matched, exact, problems
            if phase["phase"] == "bus_error" and not phase["read_low"]:
                problems.append(
                    f"row {n + 1}: arbitration loss at {t} matches a bus error with no low read "
                    "on bits 7 to 0 of an address, on bits 7 to 0 of a write data byte, "
                    "or on the ACK bit of a read data byte"
                )
                return matched, exact, problems
            matched += 1
            exact += phase["t_ps"] == t
            k += 1
            while k < len(decoded) and decoded[k]["phase"] not in ("start", "stop"):
                k += 1
            if k < len(decoded) and decoded[k]["phase"] == "stop":
                k += 1
            continue
        if k == len(decoded):
            problems.append(f"row {n + 1}: {row['phase']} at {t} has no decoded phase")
            return matched, exact, problems
        got = decoded[k]
        if fields(got) != fields(row) or abs(got["t_ps"] - t) > cycle_ps:
            problems.append(f"row {n + 1}: i2c.csv has {fields(row)} at {t}, the view {fields(got)} at {got['t_ps']}")
            return matched, exact, problems
        matched += 1
        exact += got["t_ps"] == t
        k += 1
    if k != len(decoded):
        problems.append(f"{len(decoded) - k} decoded phases after the last row of i2c.csv")
    return matched, exact, problems


def timing(scl, sda):
    """`(column, {name: [ps]}, holds)`, `holds` the tHD;DAT values on unstretched lows."""
    scl_high, sda_high, edges = merged(scl, sda)
    rises = [t for t, high in scl[1:] if high]
    periods = [b - a for a, b in zip(rises, rises[1:], strict=False)]
    column = 0 if periods and statistics.median(periods) >= 10_000_000 else 1
    seen = {name: [] for name in TABLE}
    lows = []
    rise = fall = sda_fall = stop = None
    held = []  # SDA changes in the current SCL low, as hold times
    for at, line, high in edges:
        if line == "scl":
            if high == scl_high:
                continue
            scl_high = high
            if high:
                if fall is not None:
                    seen["tLOW"].append(at - fall)
                    lows.append((at - fall, list(held)))
                for change in held:
                    seen["tSU;DAT"].append(at - (fall + change))
                held = []
                rise = at
            else:
                if sda_fall is not None:
                    seen["tHD;STA"].append(at - sda_fall)
                elif rise is not None:
                    seen["tHIGH"].append(at - rise)
                sda_fall = None
                fall = at
            continue
        if high == sda_high:
            continue
        sda_high = high
        if not scl_high:
            if fall is not None:
                held.append(at - fall)
                seen["tHD;DAT"].append(at - fall)
            continue
        if high:
            if rise is not None:
                seen["tSU;STO"].append(at - rise)
            stop = at
        else:
            if stop is not None:
                seen["tBUF"].append(at - stop)
            elif rise is not None:
                seen["tSU;STA"].append(at - rise)
            stop = None
            sda_fall = at
    median_low = statistics.median([low for low, _ in lows]) if lows else 0
    holds = [h for low, hs in lows if low <= 1.5 * median_low for h in hs]
    return column, seen, holds


# The master's SDA hold at each rate, and the cst816's hold, which is the same at 400 kHz.
S3_HZ = {
    100_000: {"edge": 5_000_000, "hold": 1_250_000},
    400_000: {"edge": 1_250_000, "hold": 300_000},
}
CST816_HOLD_PS = 300_000
S3_EDGES = ("tLOW", "tHIGH", "tSU;STA", "tHD;STA", "tSU;STO")


def gpio_on(part, net):
    """The `GPIO<n>` pin of `part` that sits on `net`, or `None`."""
    for pin, named in part["pins"].items():
        if named == net and pin.startswith("GPIO"):
            return pin
    return None


def bus_views(netlist):
    """`(atmega, extra)` wire keys. `extra` is `(label, scl, sda)` for each cst816 and S3 view."""
    atmega = []
    for part in netlist["parts"]:
        if part["kind"] == "atmega328p":
            name = part["name"]
            atmega.append((part, (name, "PC5"), (name, "PC4")))
    controllers = [part for part in netlist["parts"] if part["kind"] == "cst816"]
    chips = [part for part in netlist["parts"] if part["kind"] == "esp32s3"]
    extra = [(part["name"], (part["name"], "SCL"), (part["name"], "SDA")) for part in controllers]
    pairs = []
    for chip in chips:
        for touch in controllers:
            scl = gpio_on(chip, touch["pins"]["SCL"])
            sda = gpio_on(chip, touch["pins"]["SDA"])
            if scl and sda:
                pairs.append((chip["name"], touch["name"], scl, sda))
    for chip, touch, scl, sda in pairs:
        label = chip if len(pairs) == 1 else f"{chip}/{touch}"
        extra.append((label, (chip, scl), (chip, sda)))
    return atmega, extra


def s3_lengths(hz):
    """The picosecond lengths at `hz`, or `None` for a rate the table does not name."""
    row = S3_HZ.get(hz)
    if row is None:
        return None
    holds = {row["hold"], CST816_HOLD_PS}
    return {"edge": row["edge"], "holds": holds, "setups": {row["edge"] - hold for hold in holds}}


def s3_problems(seen, hz):
    """Interval names whose values are not the lengths at `hz`. tBUF is not one of them."""
    lengths = s3_lengths(hz)
    if lengths is None:
        return ["hz"]
    off = []
    for name in S3_EDGES:
        if any(value != lengths["edge"] for value in seen[name]):
            off.append(name)
    if any(value not in lengths["holds"] for value in seen["tHD;DAT"]):
        off.append("tHD;DAT")
    if any(value not in lengths["setups"] for value in seen["tSU;DAT"]):
        off.append("tSU;DAT")
    return off


def report_s3(name, scl, sda, hz):
    """Print one view's `--timing-s3` rows. Returns whether any interval is off."""
    _column, seen, _holds = timing(scl, sda)
    lengths = s3_lengths(hz)
    print(f"{name}: timing-s3 at {hz} Hz")
    failed = False
    for row in (*S3_EDGES, "tHD;DAT", "tSU;DAT"):
        values = seen[row]
        least = min(values) if values else None
        if row in S3_EDGES:
            want = f"want={lengths['edge']}"
        elif row == "tHD;DAT":
            want = f"holds={sorted(lengths['holds'])}"
        else:
            want = f"setups={sorted(lengths['setups'])}"
        off = row in s3_problems(seen, hz)
        print(f"{name}:   {row} n={len(values)} least={least} ps {want} ps off={int(off)}")
        failed |= off
    return failed


def same_phases(views, left, right):
    return [fields(phase) for phase in views[left]] == [fields(phase) for phase in views[right]]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("run", nargs="?")
    parser.add_argument("--part")
    parser.add_argument("--timing", action="store_true")
    parser.add_argument(
        "--timing-s3",
        type=int,
        metavar="HZ",
        help="check the esp32s3 and cst816 views against IDF's lengths at HZ, read from its "
        "registers as the model reads them, so not an independent check of that reading",
    )
    args = parser.parse_args()
    if not args.run:
        parser.error("RUN_DIR is required")
    if args.timing_s3 is not None and s3_lengths(args.timing_s3) is None:
        print(f"--timing-s3 takes 100000 or 400000, not {args.timing_s3}", file=sys.stderr)
        sys.exit(1)
    run = Path(args.run)
    netlist = json.loads((run / "netlist.json").read_text())
    vcd = read_vcd(run / "traces" / "logic.vcd")
    atmega, extra = bus_views(netlist)
    rows = []
    if atmega:
        with open(run / "traces" / "i2c.csv", newline="") as f:
            rows = list(csv.DictReader(f))
    failed = False
    views = {}
    order = []
    for chip, scl, sda in atmega:
        name = chip["name"]
        cycle_ps = -(-(10**12) // chip["clock_hz"]["value"])
        views[name] = decode(vcd[scl], vcd[sda])
        order.append(name)
        mine = [row for row in rows if row["part"] == name]
        print(f"{name}: {len(views[name])} decoded phases, {len(mine)} rows in i2c.csv")
        if args.part and args.part != name:
            continue
        matched, exact, problems = compare(views[name], mine, cycle_ps)
        print(f"{name}: {matched} of {len(mine)} rows match, {exact} to the picosecond")
        for problem in problems:
            print(f"{name}: {problem}")
            failed = True
    for label, scl, sda in extra:
        views[label] = decode(vcd[scl], vcd[sda])
        order.append(label)
    atmega_names = [chip["name"] for chip, _scl, _sda in atmega]
    for other in atmega_names[1:]:
        same = same_phases(views, atmega_names[0], other)
        print(f"{atmega_names[0]} and {other} decode the same phases: {same}")
        failed |= not same
    if atmega_names:
        for label, _scl, _sda in extra:
            if same_phases(views, atmega_names[0], label):
                continue
            print(f"{atmega_names[0]} and {label} decode the same phases: False")
            failed = True
    elif order:
        for other in order[1:]:
            same = same_phases(views, order[0], other)
            print(f"{order[0]} and {other} decode the same phases: {same}")
            failed |= not same
    if args.timing:
        for chip, scl, sda in atmega:
            name = chip["name"]
            column, seen, holds = timing(vcd[scl], vcd[sda])
            rate = ("100 kHz", "400 kHz")[column]
            print(f"{name}: timing against the {rate} column")
            for row, mins in TABLE.items():
                values = seen[row]
                low = [v for v in values if v < mins[column]]
                least = min(values) if values else None
                print(f"{name}:   {row} n={len(values)} least={least} ps min={mins[column]} ps below={len(low)}")
                failed |= bool(low)
            over = [h for h in holds if h > HOLD_MAX[column]]
            print(f"{name}:   tHD;DAT max={HOLD_MAX[column]} ps on {len(holds)} unstretched holds, over={len(over)}")
            failed |= bool(over)
    if args.timing_s3 is not None:
        for label, scl, sda in extra:
            failed |= report_s3(label, vcd[scl], vcd[sda], args.timing_s3)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
