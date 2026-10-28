# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Render an `st77916` panel from a replay list or a pattern, apart from the engine.

    uv run scripts/render_st77916.py EVENTS [--part NAME] [--out DIR]
    uv run scripts/render_st77916.py EVENTS --check RUN_DIR [--part NAME]
    uv run scripts/render_st77916.py --pattern PATTERN [--part NAME] [--sender NAME] [--out DIR]
    uv run scripts/render_st77916.py --pattern PATTERN --check RUN_DIR [--part NAME] [--sender NAME]
    uv run scripts/render_st77916.py --self-test

EVENTS is a replay list, a JSON array of `spi_replay` events, such as
`examples/st77916-replay/events.json`. The script plays it into one panel, as a `spi_replay`
part on the panel's pins sends it, and never reads a run's `link.csv` or `display.csv` to do
so. `--out` writes `display.csv`, the rows the run must write without `t_ps`, and
`display-<part>.rgb565`, the frame memory it must end with. `--part` names the panel, `lcd1`
by default.

PATTERN is `firmware/esp32s3/pattern.json`. The script turns it into the SPI2 transfers
`firmware/esp32s3/pattern.c` sends, written from that program's source:

- Each init command, then CASET, RASET and RAMWR for each rectangle, is the 32-bit word as an
  8-bit command phase and a 24-bit address phase on one lane.
- A command with parameters keeps CS after its word and sends the parameters on one lane.
- RAMWR keeps CS after its word and sends the rectangle's pixels, RGB565 high byte first, on
  four lanes in transfers of 64 bytes, keeping CS on all but the last.
- The fill rules: `bars` takes color `dx * len(colors) // width`, `checker` takes color
  `(dx // cell + dy // cell) & 1`, `gradient` is `rgb(dx * 31 // (width - 1), dy * 63 //
  (height - 1), blue)`, and `xor` is `rgb(v >> 3, v >> 2, 31 - (v >> 3))` for `v = (x ^ y) &
  0xff`, with `dx` and `dy` from the rectangle's corner and `x` and `y` on the panel.

It plays those into the panel as the replay list is played, and refuses a pattern with a
rectangle whose pixels are all one value. `--out` also writes `link.csv`, the rows the run
must write without `t_ps` and `start_ps`, with `--sender` naming the chip, `u1` by default.

`--check` compares RUN_DIR with them. Each row of `traces/display.csv` must equal its
expected row on `part`, `x0`, `y0`, `x1`, `y1`, `bytes` and `hash`, in order, and `t_ps` is
not compared. `traces/display-<part>.rgb565` must equal the expected image byte for byte.
`observations.json`'s `displays` entry must name that file and its SHA-256, and its `registers`,
`flags`, `madctl` and `colmod` must equal the panel's. The check fails when the expected rows
are none, and when the expected image is a blank one. For a pattern, each row of
`traces/link.csv` must also equal its expected row on `part`, `pin`, `kind`, `cs`, `phases`,
`bytes` and `hash`, in order.

The panel model, written from the MIPI DCS command set and the ST77916 QSPI framing:

- A command is a 32-bit word in the phases before the data, `opcode << 24 | command << 8`.
- `RAMWR` and `RAMWRC` take opcode 0x32, and their data uses four lanes. Every other
  command takes opcode 0x02, and its data uses one lane. The opcode is checked on every
  transaction. The data lanes are checked only when the transaction carries bytes.
- A transaction with no phase continues the command a transaction ending with CS held left
  open. A command closes when CS is released or when the next command comes. A continuation
  whose data lanes do not match drops that transaction's bytes, and a release still closes
  the open command.
- A register command runs when it closes: SWRESET, SLPIN and SLPOUT, INVOFF and INVON,
  DISPOFF and DISPON, CASET and RASET (start and end, both ends included, big-endian),
  MADCTL and COLMOD. Every other command keeps its last bytes, and the model does not run it.
- RAMWR starts at the window's corner and RAMWRC goes on from the last pixel. Each two bytes
  are one pixel when COLMOD's low three bits are 5. A pixel goes along its row, then to the
  next row at the window's right edge, and back to the top row after the bottom one.
  MADCTL's MX mirrors the column, MY mirrors the row, and then MV exchanges them.
- A pixel write that took bytes is one row when it closes, with its window and the first 16
  hex digits of the SHA-256 of its bytes.
- A reset sets COLMOD to 0x66, MADCTL to 0, and the window to the whole panel.
"""

import argparse
import csv
import hashlib
import io
import json
import sys
from pathlib import Path
from typing import NamedTuple

WIDTH = 360
HEIGHT = 360
HEADER = ["part", "x0", "y0", "x1", "y1", "bytes", "hash"]
LINK_HEADER = ["t_ps", "start_ps", "part", "pin", "kind", "cs", "phases", "bytes", "hash"]
BUFFER = 64
CS_PIN = "GPIO12"
PATTERN = Path(__file__).resolve().parent.parent / "firmware/esp32s3/pattern.json"


class Panel:
    def __init__(self):
        self.frame = bytearray(WIDTH * HEIGHT * 2)
        self.rows = []
        self.stored = {}
        self.reset()
        self.open = None

    def reset(self):
        self.madctl = 0
        self.colmod = 0x66
        self.window = [0, 0, WIDTH - 1, HEIGHT - 1]
        self.cursor = (0, 0)
        self.sleep = True
        self.display_on = False
        self.inverted = False
        self.reset_pin = False
        self.stored = {}

    def transaction(self, event):
        phases = event["phases"]
        lanes = event["lanes"]
        data = bytes(event["bytes"])
        if phases:
            self.close()
            bits = sum(phase["bits"] for phase in phases)
            word = 0
            for phase in phases:
                word = (word << phase["bits"]) | phase["value"]
            opcode, command = word >> 24, (word >> 8) & 0xFF
            if bits == 32 and self.accepted(opcode, command, lanes, bool(data)):
                self.begin(command)
        elif self.open is not None and data and not self.lanes_match(lanes):
            if event["cs"] == "released":
                self.close()
            return
        if self.open is not None:
            self.feed(data)
        if event["cs"] == "released":
            self.close()

    @staticmethod
    def accepted(opcode, command, lanes, has_data):
        if command in (0x2C, 0x3C):
            want_opcode, want_lanes = 0x32, 4
        else:
            want_opcode, want_lanes = 0x02, 1
        return opcode == want_opcode and (not has_data or lanes == want_lanes)

    def lanes_match(self, lanes):
        kind = self.open["kind"]
        if kind == "pixels":
            return lanes == 4
        if kind == "register":
            return lanes == 1
        return True

    def begin(self, command):
        if command in (0x2C, 0x3C):
            if self.colmod & 0x07 != 0x05:
                self.open = {"kind": "dropped"}
                return
            if command == 0x2C:
                self.cursor = (self.window[0], self.window[1])
            self.open = {"kind": "pixels", "window": list(self.window), "data": bytearray(), "half": None}
        else:
            self.open = {"kind": "register", "command": command, "data": bytearray()}

    def feed(self, data):
        write = self.open
        if write["kind"] == "register":
            write["data"] += data
        elif write["kind"] == "pixels":
            write["data"] += data
            for byte in data:
                if write["half"] is None:
                    write["half"] = byte
                    continue
                x, y = self.place(*self.cursor)
                at = (y * WIDTH + x) * 2
                self.frame[at : at + 2] = bytes([write["half"], byte])
                write["half"] = None
                self.cursor = self.advance(write["window"], *self.cursor)

    def place(self, column, row):
        if self.madctl & 0x40:
            column = WIDTH - 1 - column
        if self.madctl & 0x80:
            row = HEIGHT - 1 - row
        return (row, column) if self.madctl & 0x20 else (column, row)

    @staticmethod
    def advance(window, column, row):
        x0, y0, x1, y1 = window
        if column < x1:
            return column + 1, row
        return x0, row + 1 if row < y1 else y0

    def close(self):
        write, self.open = self.open, None
        if write is None:
            return
        if write["kind"] == "pixels" and write["data"]:
            digest = hashlib.sha256(write["data"]).hexdigest()[:16]
            self.rows.append([*write["window"], len(write["data"]), digest])
        elif write["kind"] == "register":
            self.run(write["command"], bytes(write["data"]))

    def run(self, command, data):
        if command == 0x01:
            self.reset()
        elif command == 0x10:
            self.sleep = True
        elif command == 0x11:
            self.sleep = False
        elif command == 0x20:
            self.inverted = False
        elif command == 0x21:
            self.inverted = True
        elif command == 0x28:
            self.display_on = False
        elif command == 0x29:
            self.display_on = True
        elif command in (0x2A, 0x2B):
            if len(data) >= 4:
                start, end = data[0] << 8 | data[1], data[2] << 8 | data[3]
                limit = WIDTH if command == 0x2A else HEIGHT
                if start <= end < limit:
                    k = 0 if command == 0x2A else 1
                    self.window[k], self.window[k + 2] = start, end
        elif command == 0x36:
            if data:
                self.madctl = data[0]
        elif command == 0x3A:
            if data:
                self.colmod = data[0]
        else:
            self.stored[command] = bytes(data)

    def state(self):
        return {
            "flags": {
                "display_on": self.display_on,
                "inverted": self.inverted,
                "reset": self.reset_pin,
                "sleep": self.sleep,
            },
            "madctl": self.madctl,
            "colmod": self.colmod,
            "registers": len(self.stored),
        }


def rgb(r, g, b):
    return (r & 31) << 11 | (g & 63) << 5 | (b & 31)


def pixel(rect, x, y):
    width, height = rect["x1"] - rect["x0"] + 1, rect["y1"] - rect["y0"] + 1
    dx, dy = x - rect["x0"], y - rect["y0"]
    fill = rect["fill"]
    if fill == "bars":
        colors = rect["colors"]
        return colors[dx * len(colors) // width]
    if fill == "checker":
        return rect["colors"][(dx // rect["cell"] + dy // rect["cell"]) & 1]
    if fill == "gradient":
        return rgb(dx * 31 // (width - 1), dy * 63 // (height - 1), rect["blue"])
    if fill == "xor":
        v = (x ^ y) & 0xFF
        return rgb(v >> 3, v >> 2, 31 - (v >> 3))
    raise ValueError(f"unknown fill {fill}")


def rect_pixels(rect):
    out = bytearray()
    for y in range(rect["y0"], rect["y1"] + 1):
        for x in range(rect["x0"], rect["x1"] + 1):
            value = pixel(rect, x, y)
            out += bytes([value >> 8, value & 0xFF])
    return bytes(out)


def pattern_events(doc):
    """The transfers pattern.c sends for `doc`, in order, as replay events."""
    events = []

    def word(opcode, command, held):
        phases = [{"bits": 8, "lanes": 1, "value": opcode}, {"bits": 24, "lanes": 1, "value": command << 8}]
        events.append({"phases": phases, "lanes": 1, "bytes": [], "cs": "held" if held else "released"})

    def data(payload, lanes, held):
        events.append({"phases": [], "lanes": lanes, "bytes": list(payload), "cs": "held" if held else "released"})

    def register(command, params):
        word(0x02, command, bool(params))
        if params:
            data(params, 1, False)

    for command in doc["init"]:
        register(command["cmd"], command["data"])
    for rect in doc["rects"]:
        pixels = rect_pixels(rect)
        if len(set(zip(pixels[::2], pixels[1::2]))) < 2:
            raise ValueError(f"rectangle {rect} is one value")
        for command, start, end in [(0x2A, rect["x0"], rect["x1"]), (0x2B, rect["y0"], rect["y1"])]:
            register(command, [start >> 8, start & 0xFF, end >> 8, end & 0xFF])
        word(0x32, 0x2C, True)
        for at in range(0, len(pixels), BUFFER):
            data(pixels[at : at + BUFFER], 4, at + BUFFER < len(pixels))
    return events


def link_rows(sender, pin, events):
    """The `link.csv` cells after `t_ps` and `start_ps` that `events` make, one list per row."""
    rows = []
    for event in events:
        phases = " ".join(f"{p['bits']}/{p['lanes']}:{p['value']:0{-(-p['bits'] // 4)}x}" for p in event["phases"])
        payload = bytes(event["bytes"])
        rows.append([sender, pin, "spi", event["cs"], phases, str(len(payload)), hashlib.sha256(payload).hexdigest()[:16]])
    return rows


def compare_link(rows, run_csv):
    lines = run_csv.splitlines()
    if not lines or lines[0] != ",".join(LINK_HEADER):
        return ["link.csv has the wrong header"]
    got = [line.split(",")[2:] for line in lines[1:]]
    if got == rows:
        return []
    at = next((k for k, (g, r) in enumerate(zip(got, rows)) if g != r), min(len(got), len(rows)))
    have = ",".join(got[at]) if at < len(got) else "no row"
    want = ",".join(rows[at]) if at < len(rows) else "no row"
    return [f"link.csv has {len(got)} rows for {len(rows)} expected, and row {at} is {have}, not {want}"]


class Rendered(NamedTuple):
    rows: list
    image: bytes
    state: dict


def render(events):
    panel = Panel()
    for event in events:
        panel.transaction(event)
    return Rendered(panel.rows, bytes(panel.frame), panel.state())


def expected_csv(part, rows):
    text = io.StringIO()
    writer = csv.writer(text, lineterminator="\n")
    writer.writerow(HEADER)
    for row in rows:
        writer.writerow([part, *row])
    return text.getvalue()


def compare(part, rows, image, state, run_csv, run_image, entry):
    problems = []
    blank = bytes(len(image))
    if not rows:
        problems.append("the replay list gives no row")
    if hashlib.sha256(image).digest() == hashlib.sha256(blank).digest():
        problems.append("the expected image is blank")
    lines = run_csv.splitlines()
    if not lines or lines[0] != "t_ps," + ",".join(HEADER):
        problems.append("display.csv has the wrong header")
    got = [line.split(",")[1:] for line in lines[1:]]
    want = [[part, *map(str, row)] for row in rows]
    if got != want:
        problems.append(f"display.csv has {len(got)} rows that differ from the {len(want)} expected")
    if run_image != image:
        problems.append("the image differs from the expected image")
    if entry.get("sha256") != hashlib.sha256(image).hexdigest():
        problems.append("observations.json names another SHA-256")
    if entry.get("flags") != state["flags"]:
        problems.append("flags differ")
    if entry.get("madctl") != state["madctl"]:
        problems.append("madctl differs")
    if entry.get("colmod") != state["colmod"]:
        problems.append("colmod differs")
    if entry.get("registers") != state["registers"]:
        problems.append("registers differ")
    return problems


def check(events, part, run_dir):
    rows, image, state = render(events)
    traces = run_dir / "traces"
    observations = json.loads((run_dir / "observations.json").read_text())
    entry = observations.get("displays", {}).get(part, {})
    problems = compare(
        part,
        rows,
        image,
        state,
        (traces / "display.csv").read_text(),
        (traces / f"display-{part}.rgb565").read_bytes(),
        entry,
    )
    if entry.get("image") != f"traces/display-{part}.rgb565":
        problems.append("observations.json names another image file")
    return rows, image, problems


def entry_for(state, sha):
    return {"sha256": sha, **state}


def self_test():
    events = json.loads((Path(__file__).resolve().parent.parent / "examples/st77916-replay/events.json").read_text())
    rows, image, state = render(events)
    assert len(rows) >= 2, rows
    sha = hashlib.sha256(image).hexdigest()
    run_csv = "t_ps," + ",".join(HEADER) + "\n" + "".join(f"{k},lcd1,{','.join(map(str, row))}\n" for k, row in enumerate(rows))
    entry = entry_for(state, sha)
    assert compare("lcd1", rows, image, state, run_csv, image, entry) == [], "the right run passes"
    blank = bytes(len(image))
    assert compare("lcd1", rows, image, state, run_csv, blank, entry_for(state, hashlib.sha256(blank).hexdigest())), "a blank frame fails"
    missing, _, _ = render(events[:-1])
    assert len(missing) == len(rows) - 1
    _, short, _ = render([event for event in events if event is not last_rectangle(events)])
    assert short != image
    assert compare("lcd1", rows, image, state, run_csv, short, entry_for(state, hashlib.sha256(short).hexdigest())), "a frame missing a rectangle fails"
    dropped = "\n".join(run_csv.splitlines()[:-1]) + "\n"
    assert compare("lcd1", rows, image, state, dropped, image, entry), "a missing row fails"
    assert compare("lcd1", [], image, state, run_csv, image, entry), "no expected row fails"
    assert compare("lcd1", rows, blank, state, run_csv, blank, entry_for(state, hashlib.sha256(blank).hexdigest())), "a blank expected image fails"
    bad_registers = entry_for(state, sha)
    bad_registers["registers"] = state["registers"] + 1
    assert compare("lcd1", rows, image, state, run_csv, image, bad_registers), "a wrong registers count fails"
    bad_flags = entry_for(state, sha)
    bad_flags["flags"] = {**state["flags"], "display_on": not state["flags"]["display_on"]}
    assert compare("lcd1", rows, image, state, run_csv, image, bad_flags), "wrong flags fail"
    bad_madctl = entry_for(state, sha)
    bad_madctl["madctl"] = state["madctl"] ^ 0x40
    assert compare("lcd1", rows, image, state, run_csv, image, bad_madctl), "a wrong madctl fails"
    bad_colmod = entry_for(state, sha)
    bad_colmod["colmod"] = state["colmod"] ^ 0x01
    assert compare("lcd1", rows, image, state, run_csv, image, bad_colmod), "a wrong colmod fails"
    colmod = replay(0x02, 0x3A, [0x55], 1, "released")
    alone = replay(0x32, 0x2C, [], 1, "held")
    pixels = replay(None, 0, [0xAB, 0xCD], 4, "released")
    drawn, frame, _ = render([colmod, alone, pixels])
    assert len(drawn) == 1 and drawn[0][4] == 2, "a RAMWR with no bytes draws the following data"
    assert frame[:2] == bytes([0xAB, 0xCD])
    _, _, on = render([replay(0x02, 0x29, [], 4, "released")])
    assert on["flags"]["display_on"], "a DISPON with no bytes runs"
    _, _, off = render([replay(0x32, 0x29, [], 1, "released")])
    assert not off["flags"]["display_on"], "a data-less command still checks its opcode"
    held = replay(0x32, 0x2C, [0x11, 0x22], 4, "held")
    bad = replay(None, 0, [0x33, 0x44], 1, "held")
    good = replay(None, 0, [0x55, 0x66], 4, "released")
    rows_b, frame_b, _ = render([colmod, held, bad, good])
    assert len(rows_b) == 1 and rows_b[0][4] == 4, "a continuation on the wrong lanes drops its bytes"
    assert frame_b[:4] == bytes([0x11, 0x22, 0x55, 0x66])
    assert frame_b[4:6] == bytes(2)
    print(f"self-test ok: {len(rows)} rows, registers {state['registers']}, image sha256 {sha}")
    pattern_self_test()


def pattern_self_test():
    doc = json.loads(PATTERN.read_text())
    events = pattern_events(doc)
    rows, image, state = render(events)
    assert len(rows) == len(doc["rects"]), rows
    sha = hashlib.sha256(image).hexdigest()
    entry = entry_for(state, sha)
    header = "t_ps," + ",".join(HEADER) + "\n"
    lines = [f"{k},lcd1,{','.join(map(str, row))}\n" for k, row in enumerate(rows)]
    run_csv = header + "".join(lines)
    assert compare("lcd1", rows, image, state, run_csv, image, entry) == [], "the right pattern run passes"
    assert compare("lcd1", rows, image, state, header, image, entry), "a run with no row fails"
    assert compare("lcd1", rows, image, state, header + "".join(lines[:-1]), image, entry), "a missing row fails"
    assert compare("lcd1", rows, image, state, run_csv + lines[-1], image, entry), "an extra row fails"
    blank = bytes(len(image))
    assert compare("lcd1", rows, image, state, run_csv, blank, entry_for(state, hashlib.sha256(blank).hexdigest())), "a blank final image fails"
    flat = {**doc, "rects": [{**doc["rects"][0], "fill": "bars", "colors": [0x1234, 0x1234]}]}
    try:
        pattern_events(flat)
    except ValueError:
        pass
    else:
        raise AssertionError("a rectangle of one value is refused")
    want = link_rows("u1", CS_PIN, events)
    link_header = ",".join(LINK_HEADER) + "\n"
    link = [f"{k},{k},{','.join(row)}\n" for k, row in enumerate(want)]
    assert compare_link(want, link_header + "".join(link)) == [], "the right link.csv passes"
    assert compare_link(want, link_header), "a link.csv with no row fails"
    assert compare_link(want, link_header + "".join(link[:-1])), "a missing transfer fails"
    assert compare_link(want, link_header + "".join(link) + link[-1]), "an extra transfer fails"
    changed = link_header + "".join(link[:2]) + link[2].replace(",u1,", ",u9,", 1) + "".join(link[3:])
    assert "row 2 is " in compare_link(want, changed)[0], "a changed transfer is named by its row"
    print(f"pattern self-test ok: {len(rows)} rows, {len(want)} transfers, image sha256 {sha}")


def replay(opcode, command, data, lanes, cs):
    phases = []
    if opcode is not None:
        phases = [
            {"bits": 8, "lanes": 1, "value": opcode},
            {"bits": 24, "lanes": 1, "value": command << 8},
        ]
    return {"phases": phases, "lanes": lanes, "bytes": list(data), "cs": cs}


def last_rectangle(events):
    """The first pixel transaction of the last rectangle, dropped whole by the self-test."""
    pixel_writes = [event for event in events if event["phases"] and event["phases"][0]["value"] == 0x32]
    return pixel_writes[-1]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("events", nargs="?", type=Path)
    parser.add_argument("--pattern", type=Path)
    parser.add_argument("--part", default="lcd1")
    parser.add_argument("--sender", default="u1")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if (args.events is None) == (args.pattern is None):
        parser.error("give EVENTS or --pattern")
    if args.pattern:
        events = pattern_events(json.loads(args.pattern.read_text()))
        link = link_rows(args.sender, CS_PIN, events)
    else:
        events = json.loads(args.events.read_text())
        link = None
    rows, image, _state = render(events)
    if args.out:
        args.out.mkdir(parents=True, exist_ok=True)
        (args.out / "display.csv").write_text(expected_csv(args.part, rows))
        (args.out / f"display-{args.part}.rgb565").write_bytes(image)
        if link is not None:
            (args.out / "link.csv").write_text("".join(",".join(row) + "\n" for row in [LINK_HEADER[2:], *link]))
    print(f"expected: {len(rows)} rows, image sha256 {hashlib.sha256(image).hexdigest()}")
    if args.check:
        _, _, problems = check(events, args.part, args.check)
        if link is not None:
            problems += compare_link(link, (args.check / "traces/link.csv").read_text())
        for problem in problems:
            print(f"FAIL {problem}")
        if problems:
            sys.exit(1)
        extra = f", {len(link)} transfers" if link is not None else ""
        print(f"ok: {args.check} matches on {len(rows)} rows{extra} and the image")


if __name__ == "__main__":
    main()
