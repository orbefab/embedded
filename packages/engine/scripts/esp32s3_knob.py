# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a knob run, write refs from captures, and write closings into a netlist.

    uv run packages/engine/scripts/esp32s3_knob.py panel [--shape] RUN REFS
    uv run packages/engine/scripts/esp32s3_knob.py marks RUN REFS
    uv run packages/engine/scripts/esp32s3_knob.py events RUN REFS
    uv run packages/engine/scripts/esp32s3_knob.py --check RUN REFS
    uv run packages/engine/scripts/esp32s3_knob.py refs --write [--start LINE] CAP CAP CAP REFS
    uv run packages/engine/scripts/esp32s3_knob.py script REFS NETLIST

`panel` compares transfers and flushes. A transfer is its kind, command, length, hash and SPI
transactions. A flush is its rectangle, length and hash. `traces/display.csv` must list those
flushes, and `traces/display-lcd1.rgb565` must be the frame they leave. `--shape` drops colour
hashes and the frame. `panel` does not read a mark line.

`marks` compares every line from the start line through the done line that is not a transfer, a
K line or a T line. `events` compares the K and T lines. `--check` runs all three. It also wants
every diagnostic to be `chip.unmodeled_register`, `KNOB READY` before 2.0 s, and the backlight
duties 0 then 4096 of 4096 steps at 5 kHz. The summary names that ready time.

`refs --write` keeps one capture's lists only when exactly three different captures agree.
The destination must not exist yet and must not be one of the captures. Two paths that resolve
to the same file are refused. A 12-hex address token on a mark line, a K line or a T line is
refused. `--start` defaults to `KNOB START`. The done line is that line's first word plus
` DONE`. `script` writes closings, touches and the `x` byte into a netlist. The compare
of that write with the committed netlist catches a moved or removed closing or touch, while
a later stop time or an extra console byte other than `x` still matches.
"""

import argparse
import base64
import csv
import hashlib
import io
import json
import re
import sys
from pathlib import Path

PANEL = 360
QIO = 1 << 1
CS_KEEP = 1 << 8
KNOWN_FLAGS = QIO | CS_KEEP
PANEL_WORDS = {"P", "C", "B", "F", "S"}
SKIP = PANEL_WORDS | {"K", "T"}
READY_BEFORE_PS = 2_000_000_000_000
CLOSE_FROM_PS = 2_000_000_000_000
CLOSE_LEN_PS = 80_000_000_000
CLOSE_APART_PS = 300_000_000_000
TOUCH_HOLD_PS = 300_000_000_000
TOUCH_APART_PS = 400_000_000_000
AFTER_PS = 500_000_000_000
MIN_GAP_PS = 2_000
DOTS = ((180, 80), (93, 230), (267, 230))
LINK = ["part", "pin", "kind", "cs", "phases", "bytes", "hash", "lanes"]
DUTIES = [
    ("u1", "GPIO21", "duty", "", "0/4096@5000", "", "", ""),
    ("u1", "GPIO21", "duty", "", "4096/4096@5000", "", "", ""),
]
DEFAULT_START = "KNOB START"
ADDRESS_TOKEN = re.compile(
    r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{12}(?![0-9A-Fa-f])"
    r"|(?<![0-9A-Fa-f:])(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}(?![0-9A-Fa-f:])"
)


def h16(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:16]


def done_line(start: str) -> str:
    return f"{start.split(' ', 1)[0]} DONE"


def section(text: str, start: str) -> list[str]:
    lines = text.splitlines()
    try:
        begin = lines.index(start)
    except ValueError:
        raise ValueError(f"no {start!r} line") from None
    done = done_line(start)
    try:
        end = lines.index(done, begin)
    except ValueError:
        raise ValueError(f"no {done!r} line") from None
    return lines[begin : end + 1]


def parse_transfers(lines: list[str]) -> tuple[list[dict], list[dict]]:
    transfers, flushes = [], []
    for line in lines:
        f = line.split(" ")
        if f[0] not in {"P", "C", "B", "F", "S"}:
            continue
        try:
            if f[0] == "P":
                data = bytes.fromhex(f[4]) if int(f[3]) else b""
                transfers.append(
                    {
                        "kind": "P",
                        "cmd": f"{int(f[2], 16):08x}",
                        "length": int(f[3]),
                        "hash": h16(data),
                        "data": data,
                        "qio": False,
                        "spi": [],
                        "seq": int(f[1]),
                    }
                )
            elif f[0] == "C":
                transfers.append(
                    {
                        "kind": "C",
                        "cmd": f"{int(f[2], 16):08x}",
                        "length": int(f[3]),
                        "hash": f[4],
                        "data": b"",
                        "qio": True,
                        "spi": [],
                        "seq": int(f[1]),
                    }
                )
            elif f[0] == "B":
                transfers[-1]["data"] += base64.b64decode(f[2], validate=True)
            elif f[0] == "S":
                if f[4] != "-" or f[6] != "-" or int(f[5], 16) or int(f[7], 16):
                    raise ValueError("a transaction has a command or an address")
                transfers[-1]["spi"].append([f[2], int(f[3], 16), int(f[8])])
            else:
                flushes.append(
                    {
                        "seq": int(f[1]),
                        "x0": int(f[2]),
                        "y0": int(f[3]),
                        "x1": int(f[4]),
                        "y1": int(f[5]),
                        "length": int(f[6]),
                        "hash": f[7],
                    }
                )
            if f[0] in {"B", "S"} and int(f[1]) != transfers[-1]["seq"]:
                raise ValueError(f"an {f[0]} line of transfer {f[1]} follows transfer {transfers[-1]['seq']}")
        except (IndexError, ValueError) as error:
            raise ValueError(f"{line[:80]}: {error}") from None
    for t in transfers:
        if h16(t["data"]) != t["hash"] or len(t["data"]) != t["length"]:
            if t["kind"] == "C" and not t["data"]:
                raise ValueError(f"transfer {t['seq']}: a colour write after the dump ended")
            raise ValueError(f"transfer {t['seq']}: its bytes do not give its length and hash")
        if not t["spi"] or t["spi"][0][2] != 32 or t["spi"][0][1] & QIO:
            raise ValueError(f"transfer {t['seq']}: its first transaction is not the command word on one lane")
        if sum(bits for _, _, bits in t["spi"][1:]) != 8 * len(t["data"]):
            raise ValueError(f"transfer {t['seq']}: its transactions do not carry its bytes")
        for i, (_, flags, _) in enumerate(t["spi"]):
            if flags & ~KNOWN_FLAGS:
                raise ValueError(f"transfer {t['seq']}: transaction {i} has flags {flags:#x}")
            if bool(flags & CS_KEEP) == (i == len(t["spi"]) - 1) or (i and bool(flags & QIO) != t["qio"]):
                raise ValueError(f"transfer {t['seq']}: transaction {i} has flags {flags:#x}")
    by_seq = {t["seq"]: t for t in transfers}
    if sorted([*by_seq, *(fl["seq"] for fl in flushes)]) != list(range(1, len(transfers) + len(flushes) + 1)):
        raise ValueError("the transfers and flushes are not numbered 1 to n")
    for fl in flushes:
        caset, raset, colour = (by_seq.get(fl["seq"] + k) for k in (1, 2, 3))
        if not (caset and raset and colour and colour["qio"]):
            raise ValueError(f"flush {fl['seq']}: not followed by CASET, RASET and a colour write")
        x0, y0, x1, y1 = fl["x0"], fl["y0"], fl["x1"], fl["y1"]
        if caset["data"] != bytes([x0 >> 8, x0 & 255, x1 >> 8, x1 & 255]) or raset["data"] != bytes(
            [y0 >> 8, y0 & 255, y1 >> 8, y1 & 255]
        ):
            raise ValueError(f"flush {fl['seq']}: CASET or RASET does not give its rectangle")
        if (colour["hash"], len(colour["data"])) != (fl["hash"], fl["length"]):
            raise ValueError(f"flush {fl['seq']}: its colour write has other bytes")
        fl["data"] = colour["data"]
    return transfers, flushes


def frame_of(flushes: list[dict]) -> bytes:
    memory = bytearray(PANEL * PANEL * 2)
    for fl in flushes:
        width = 2 * (fl["x1"] - fl["x0"] + 1)
        for row in range(fl["y1"] - fl["y0"] + 1):
            at = 2 * ((fl["y0"] + row) * PANEL + fl["x0"])
            memory[at : at + width] = fl["data"][row * width : (row + 1) * width]
    return bytes(memory)


def capture_of(text: str, start: str) -> dict:
    lines = section(text, start)
    transfers, flushes = parse_transfers(lines)
    image = frame_of(flushes)
    return {
        "start": start,
        "transfers": [{k: t[k] for k in ("kind", "cmd", "length", "hash", "spi")} for t in transfers],
        "flushes": [{k: fl[k] for k in ("x0", "y0", "x1", "y1", "length", "hash")} for fl in flushes],
        "marks": [line for line in lines if line.split(" ", 1)[0] not in SKIP],
        "events": [line for line in lines if line.split(" ", 1)[0] in {"K", "T"}],
        "frame_sha256": hashlib.sha256(image).hexdigest(),
        "frame_b64": base64.b64encode(image).decode("ascii"),
    }


def transfer_key(row: dict, shape: bool) -> tuple:
    spi = tuple(tuple(part) for part in row["spi"])
    if shape and row["kind"] == "C":
        return (row["kind"], row["cmd"], row["length"], spi)
    return (row["kind"], row["cmd"], row["length"], row["hash"], spi)


def flush_key(row: dict, shape: bool) -> tuple:
    rect = (row["x0"], row["y0"], row["x1"], row["y1"], row["length"])
    return rect if shape else rect + (row["hash"],)


def display_key(row: dict, shape: bool) -> tuple:
    base = ("lcd1", str(row["x0"]), str(row["y0"]), str(row["x1"]), str(row["y1"]), str(row["length"]))
    return base if shape else base + (row["hash"],)


def differ(name: str, got: list, want: list) -> None:
    if got == want:
        return
    at = next(
        (i for i, pair in enumerate(zip(got, want, strict=False)) if pair[0] != pair[1]), min(len(got), len(want))
    )
    raise ValueError(f"{name} differs at item {at + 1} ({len(got)} items, the refs {len(want)})")


def console_text(serial_csv: str) -> str:
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    known = {"USB_SERIAL_JTAG", "UART0", "UART1", "UART2"}
    if any(row["part"] != "u1" or row["usart"] not in known for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG or a UART")
    rows = [row for row in rows if row["usart"] == "USB_SERIAL_JTAG"]
    return bytes(int(row["byte"]) for row in rows).decode("ascii")


def load_refs(path: Path) -> dict:
    return json.loads(path.read_text())


def serial_path(run: Path) -> str:
    return (run / "traces" / "serial.csv").read_text()


def panel(run: Path, refs_path: Path, shape: bool) -> str:
    refs = load_refs(refs_path)
    transfers, flushes = parse_transfers(console_text(serial_path(run)).splitlines())
    got_t = [{k: t[k] for k in ("kind", "cmd", "length", "hash", "spi")} for t in transfers]
    got_f = [{k: fl[k] for k in ("x0", "y0", "x1", "y1", "length", "hash")} for fl in flushes]
    differ(
        "transfers",
        [transfer_key(row, shape) for row in got_t],
        [transfer_key(row, shape) for row in refs["transfers"]],
    )
    differ("flushes", [flush_key(row, shape) for row in got_f], [flush_key(row, shape) for row in refs["flushes"]])
    rows = list(csv.DictReader(io.StringIO((run / "traces" / "display.csv").read_text())))
    got_rows = [
        tuple(row[c] for c in ("part", "x0", "y0", "x1", "y1", "bytes")) + (() if shape else (row["hash"],))
        for row in rows
    ]
    differ("display.csv", got_rows, [display_key(row, shape) for row in refs["flushes"]])
    if not shape:
        image = base64.b64decode(refs["frame_b64"], validate=True)
        got = (run / "traces" / "display-lcd1.rgb565").read_bytes()
        if got != image:
            raise ValueError(
                f"display-lcd1.rgb565 has SHA-256 {hashlib.sha256(got).hexdigest()}, "
                f"the refs {hashlib.sha256(image).hexdigest()}"
            )
    noun = "shape" if shape else "frame"
    return f"knob panel: {len(got_t)} transfers and {len(got_f)} flushes, {noun}"


def marks(run: Path, refs_path: Path) -> str:
    refs = load_refs(refs_path)
    got = capture_of(console_text(serial_path(run)), refs["start"])
    differ("marks", got["marks"], refs["marks"])
    return f"knob marks: {len(got['marks'])} lines"


def events(run: Path, refs_path: Path) -> str:
    refs = load_refs(refs_path)
    got = capture_of(console_text(serial_path(run)), refs["start"])
    differ("events", got["events"], refs["events"])
    return f"knob events: {len(got['events'])} lines"


def ready_ps(serial_csv: str) -> int:
    rows = [row for row in csv.DictReader(io.StringIO(serial_csv)) if row["usart"] == "USB_SERIAL_JTAG"]
    buf = ""
    for row in rows:
        buf += chr(int(row["byte"]))
        if buf.endswith("KNOB READY\n") or buf.endswith("KNOB READY\r\n"):
            return int(row["t_ps"])
    raise ValueError("the console has no KNOB READY line")


def check_ready(serial_csv: str) -> int:
    at = ready_ps(serial_csv)
    if at >= READY_BEFORE_PS:
        raise ValueError(f"KNOB READY is at {at} ps, which is not before {READY_BEFORE_PS} ps")
    return at


def check_diagnostics(items: list[dict]) -> None:
    bad = [item["code"] for item in items if item["code"] != "chip.unmodeled_register"]
    if bad:
        raise ValueError(f"the run holds {bad}")


def check_duties(run: Path) -> None:
    rows = list(csv.DictReader(io.StringIO((run / "traces" / "link.csv").read_text())))
    got = [tuple(row[c] for c in LINK) for row in rows if row["kind"] == "duty"]
    differ("duties", got, DUTIES)


def check(run: Path, refs_path: Path) -> str:
    serial = serial_path(run)
    text = " ".join((panel(run, refs_path, False), marks(run, refs_path), events(run, refs_path)))
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    at = check_ready(serial)
    check_duties(run)
    return f"knob check: {text}, ready at {at} ps"


def same_lists(a: dict, b: dict) -> str | None:
    for key in ("transfers", "flushes", "marks", "events"):
        if a[key] != b[key]:
            return key
    return None


def read_capture(path: Path, start: str) -> str:
    raw = path.read_bytes()
    at = raw.find(start.encode("ascii"))
    chunk = raw if at < 0 else raw[at:]
    try:
        return chunk.decode("ascii")
    except UnicodeDecodeError:
        where = "capture" if at < 0 else "capture section"
        raise ValueError(f"the {where} is not ASCII") from None


def refuse_address(lines: list[str]) -> None:
    kept = []
    for line in lines:
        word = line.split(" ", 1)[0] if line else ""
        if word not in PANEL_WORDS:
            kept.append(line)
    if ADDRESS_TOKEN.search("\n".join(kept)):
        raise ValueError("the section holds a 12-hex address token")


def same_capture(captures: list[Path]) -> str | None:
    resolved = [path.resolve() for path in captures]
    for index, path in enumerate(resolved):
        earlier = resolved.index(path)
        if earlier != index:
            return f"captures {earlier + 1} and {index + 1} are the same file"
    return None


def write_refs(captures: list[Path], dest: Path, start: str) -> str:
    if len(captures) != 3:
        raise ValueError(f"refs --write needs exactly three captures, got {len(captures)}")
    same = same_capture(captures)
    if same:
        raise ValueError(same)
    if any(path.resolve() == dest.resolve() for path in captures):
        raise ValueError("the refs path is one of the captures")
    if dest.exists():
        raise ValueError("the refs path already exists")
    caps = []
    for path in captures:
        text = read_capture(path, start)
        refuse_address(section(text, start))
        caps.append(capture_of(text, start))
    for index, cap in enumerate(caps[1:], 2):
        key = same_lists(caps[0], cap)
        if key:
            raise ValueError(f"capture {index} {key} differ")
    dest.write_text(json.dumps(caps[0], indent=2) + "\n")
    image = hashlib.sha256(base64.b64decode(caps[0]["frame_b64"])).hexdigest()
    return (
        f"refs written: {len(caps)} captures, {len(caps[0]['transfers'])} transfers, "
        f"{len(caps[0]['flushes'])} flushes, frame {image}"
    )


def counts_of(line: str) -> tuple[int, int]:
    parts = line.split(" ")
    if len(parts) != 3 or parts[0] != "K" or not parts[1].startswith("a=") or not parts[2].startswith("b="):
        raise ValueError(f"not a K line: {line}")
    return int(parts[1][2:]), int(parts[2][2:])


def closings_of(events: list[str]) -> tuple[list[tuple[str, int]], int | None]:
    previous = (0, 0)
    schedule = []
    at = CLOSE_FROM_PS
    for line in events:
        if not line.startswith("K "):
            continue
        count = counts_of(line)
        delta = (count[0] - previous[0], count[1] - previous[1])
        if delta not in {(1, 0), (0, 1)}:
            raise ValueError(f"{line} does not count one contact")
        contact = "a" if delta[0] else "b"
        schedule.append((contact, at))
        previous = count
        at += CLOSE_APART_PS
    last_open = None if not schedule else schedule[-1][1] + CLOSE_LEN_PS
    return schedule, last_open


def touches_of(events: list[str], first: int) -> list[tuple[int, int, int]]:
    out = []
    at = first
    for line in events:
        if line == "T up" or line.startswith("K "):
            continue
        parts = line.split(" ")
        if len(parts) != 3 or parts[0] != "T" or parts[1] != "down" or parts[2] not in {"1", "2", "3"}:
            raise ValueError(f"not a T line: {line}")
        dot = int(parts[2])
        x, y = DOTS[dot - 1]
        out.append((at, x, y))
        at += TOUCH_APART_PS
    return out


def apart(events: list[dict], name: str) -> None:
    times = [event["t_ps"] for event in events]
    if any(b < a + MIN_GAP_PS for a, b in zip(times, times[1:], strict=False)):
        raise ValueError(f"{name} events are less than {MIN_GAP_PS} ps apart")


def script(refs_path: Path, netlist_path: Path) -> str:
    refs = load_refs(refs_path)
    net = json.loads(netlist_path.read_text())
    schedule, last_open = closings_of(refs["events"])
    first_touch = CLOSE_FROM_PS if last_open is None else last_open + TOUCH_APART_PS
    touches = touches_of(refs["events"], first_touch)
    if touches:
        x_ps = touches[-1][0] + AFTER_PS
    elif last_open is not None:
        x_ps = last_open + AFTER_PS
    else:
        x_ps = CLOSE_FROM_PS + AFTER_PS
    by_name = {part["name"]: part for part in net["parts"]}
    for name, contact in (("sw_a", "a"), ("sw_b", "b")):
        events = []
        for which, at in schedule:
            if which == contact:
                events.append({"t_ps": at, "state": "closed"})
                events.append({"t_ps": at + CLOSE_LEN_PS, "state": "open"})
        by_name[name]["events"] = events
    chip = next(part for part in net["parts"] if part["kind"] == "cst816")
    chip["touches"] = [
        item
        for at, x, y in touches
        for item in (
            {"t_ps": at, "x": x, "y": y, "down": True},
            {"t_ps": at + TOUCH_HOLD_PS, "x": x, "y": y, "down": False},
        )
    ]
    core = next(part for part in net["parts"] if part["kind"] == "esp32s3")
    console = [event for event in core.get("console", []) if event.get("bytes") != [120]]
    console.append({"t_ps": x_ps, "bytes": [120]})
    console.sort(key=lambda event: event["t_ps"])
    core["console"] = console
    apart(console, "console")
    for name in ("sw_a", "sw_b"):
        apart(by_name[name]["events"], name)
    apart(chip["touches"], "touches")
    stop = x_ps + AFTER_PS
    if net["analysis"]["transient"]["stop_ps"] < stop:
        net["analysis"]["transient"]["stop_ps"] = stop
    netlist_path.write_text(json.dumps(net, indent=2) + "\n")
    return f"script: {len(schedule)} closings, {len(touches)} touches, x at {x_ps} ps, stop at least {stop} ps"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", nargs=2, metavar=("RUN", "REFS"))
    commands = parser.add_subparsers(dest="cmd")
    panel_cmd = commands.add_parser("panel")
    panel_cmd.add_argument("--shape", action="store_true")
    panel_cmd.add_argument("run", type=Path)
    panel_cmd.add_argument("refs", type=Path)
    for name in ("marks", "events"):
        cmd = commands.add_parser(name)
        cmd.add_argument("run", type=Path)
        cmd.add_argument("refs", type=Path)
    refs_cmd = commands.add_parser("refs")
    refs_cmd.add_argument("--write", action="store_true")
    refs_cmd.add_argument("--start", default=DEFAULT_START)
    refs_cmd.add_argument("paths", nargs="+", type=Path)
    script_cmd = commands.add_parser("script")
    script_cmd.add_argument("refs", type=Path)
    script_cmd.add_argument("netlist", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.check:
            print(check(Path(args.check[0]), Path(args.check[1])))
            return 0
        if args.cmd == "panel":
            print(panel(args.run, args.refs, args.shape))
        elif args.cmd == "marks":
            print(marks(args.run, args.refs))
        elif args.cmd == "events":
            print(events(args.run, args.refs))
        elif args.cmd == "refs":
            if not args.write or len(args.paths) < 2:
                parser.error("refs needs --write, captures, and a destination")
            print(write_refs(args.paths[:-1], args.paths[-1], args.start))
        elif args.cmd == "script":
            print(script(args.refs, args.netlist))
        else:
            parser.error("a command is required")
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"knob: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
