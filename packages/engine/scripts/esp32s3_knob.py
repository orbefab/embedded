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
    uv run packages/engine/scripts/esp32s3_knob.py --self-test

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
import tempfile
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
    if any(row["part"] != "u1" or row["usart"] != "USB_SERIAL_JTAG" for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG")
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
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
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


def serial_of(text: str, ready_at: int = 1_000_000_000_000) -> str:
    ready_nl = text.find("\n", text.find("KNOB READY")) if "KNOB READY" in text else -1
    rows = ["t_ps,part,usart,byte"]
    for index, char in enumerate(text):
        stamp = ready_at if index == ready_nl else index + 1
        rows.append(f"{stamp},u1,USB_SERIAL_JTAG,{ord(char)}")
    return "\n".join(rows) + "\n"


def display_csv(flushes: list[dict]) -> str:
    rows = ["t_ps,part,x0,y0,x1,y1,bytes,hash"]
    for index, fl in enumerate(flushes, 1):
        rows.append(f"{index},lcd1,{fl['x0']},{fl['y0']},{fl['x1']},{fl['y1']},{fl['length']},{fl['hash']}")
    return "\n".join(rows) + "\n"


def link_csv(second: str = "4096/4096@5000") -> str:
    rows = [
        "t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash,lanes",
        "1,1,u1,GPIO21,duty,,0/4096@5000,,,",
        f"2,2,u1,GPIO21,duty,,{second},,,",
    ]
    return "\n".join(rows) + "\n"


def synthetic() -> str:
    pixels = bytes(range(16))
    digest = h16(pixels)
    caset = bytes([0x00, 0x00, 0x00, 0x03])
    raset = bytes([0x00, 0x00, 0x00, 0x01])
    lines = [
        "BEFORE THE START",
        "KNOB START",
        "KNOB INFO sample",
        "KNOB FRAME 0 BEGIN",
        f"F 1 0 0 3 1 16 {digest} 10",
        f"P 2 02002a00 4 {caset.hex()} 11",
        "S 2 poll 0100 - 0 - 0 32",
        "S 2 poll 0000 - 0 - 0 32",
        f"P 3 02002b00 4 {raset.hex()} 12",
        "S 3 poll 0100 - 0 - 0 32",
        "S 3 poll 0000 - 0 - 0 32",
        f"C 4 32002c00 16 {digest} 13",
        "B 4 " + base64.b64encode(pixels[:8]).decode(),
        "B 4 " + base64.b64encode(pixels[8:]).decode(),
        "S 4 poll 0100 - 0 - 0 32",
        "S 4 queue 0102 - 0 - 0 64",
        "S 4 queue 0002 - 0 - 0 64",
        "KNOB FRAME 0 END",
        "K a=1 b=0",
        "T down 1",
        "T up",
        "KNOB READY",
        "KNOB DONE",
    ]
    return "\n".join(lines) + "\n"


def write_run(
    root: Path,
    text: str,
    refs: dict,
    ready_at: int = 1_000_000_000_000,
    duty: str = "4096/4096@5000",
    diagnostic: str = "chip.unmodeled_register",
    frame: bytes | None = None,
) -> None:
    traces = root / "traces"
    traces.mkdir(parents=True, exist_ok=True)
    (traces / "serial.csv").write_text(serial_of(text, ready_at))
    (traces / "display.csv").write_text(display_csv(refs["flushes"]))
    image = frame if frame is not None else base64.b64decode(refs["frame_b64"])
    (traces / "display-lcd1.rgb565").write_bytes(image)
    (traces / "link.csv").write_text(link_csv(duty))
    (root / "diagnostics.json").write_text(json.dumps([{"code": diagnostic, "severity": "warning"}]))


def bad(got: object):
    raise ValueError(got)


def self_test() -> int:
    print(
        "constants "
        f"ready_before_ps={READY_BEFORE_PS} close_from_ps={CLOSE_FROM_PS} close_len_ps={CLOSE_LEN_PS} "
        f"close_apart_ps={CLOSE_APART_PS} touch_hold_ps={TOUCH_HOLD_PS} touch_apart_ps={TOUCH_APART_PS} "
        f"after_ps={AFTER_PS} min_gap_ps={MIN_GAP_PS} dots={DOTS[0]} {DOTS[1]} {DOTS[2]} duties={DUTIES[1][4]}"
    )
    failures = 0

    def expect(name: str, fn, refuse: bool = False) -> None:
        nonlocal failures
        try:
            fn()
        except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
            if refuse:
                print(f"mutation {name}: refused ({error})")
            else:
                print(f"FAIL {name}: {error}")
                failures += 1
            return
        if refuse:
            print(f"FAIL {name}: accepted")
            failures += 1
        else:
            print(f"mutation {name}: accepted")

    text = synthetic()
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        refs_path = root / "refs.json"
        print(
            write_refs(
                [_write(root / "a.txt", text), _write(root / "b.txt", text), _write(root / "c.txt", text)],
                refs_path,
                DEFAULT_START,
            )
        )
        refs = load_refs(refs_path)
        expect(
            "the line before the start is not a mark",
            lambda: "BEFORE THE START" not in json.dumps(refs) or bad("present"),
        )
        ready_at = 1_000_000_000_000
        run = root / "run"
        write_run(run, text, refs, ready_at)

        def check_summary() -> None:
            summary = check(run, refs_path)
            if f"ready at {ready_at} ps" not in summary:
                bad(summary)

        expect("panel", lambda: panel(run, refs_path, False))
        expect("marks", lambda: marks(run, refs_path))
        expect("events", lambda: events(run, refs_path))
        expect("check", check_summary)

        shaped = json.loads(refs_path.read_text())
        shaped["transfers"][2]["hash"] = "0" * 16
        shaped["flushes"][0]["hash"] = "0" * 16
        shaped["frame_b64"] = base64.b64encode(b"\x00" * len(base64.b64decode(shaped["frame_b64"]))).decode()
        shaped_path = root / "shaped.json"
        shaped_path.write_text(json.dumps(shaped))
        expect("a colour hash fails panel", lambda: panel(run, shaped_path, False), refuse=True)
        expect("panel --shape drops the colour hash and the frame", lambda: panel(run, shaped_path, True))

        moved = json.loads(refs_path.read_text())
        moved["flushes"][0]["x1"] = 4
        moved_path = root / "moved.json"
        moved_path.write_text(json.dumps(moved))
        expect("panel --shape keeps the rectangle", lambda: panel(run, moved_path, True), refuse=True)

        param = json.loads(refs_path.read_text())
        param["transfers"][1]["hash"] = "0" * 16
        param_path = root / "param.json"
        param_path.write_text(json.dumps(param))
        expect("panel --shape keeps a parameter hash", lambda: panel(run, param_path, True), refuse=True)

        other_marks = text.replace("KNOB INFO sample", "KNOB INFO other")
        marked = root / "marked"
        write_run(marked, other_marks, refs)
        expect("panel reads no mark", lambda: panel(marked, refs_path, False))
        expect("a changed mark fails", lambda: marks(marked, refs_path), refuse=True)

        other_events = text.replace("K a=1 b=0", "K a=1 b=1")
        evented = root / "evented"
        write_run(evented, other_events, refs)
        expect("a changed K line fails events", lambda: events(evented, refs_path), refuse=True)
        expect("events reads no mark", lambda: events(marked, refs_path))

        late = root / "late"
        write_run(late, text, refs, ready_at=READY_BEFORE_PS)
        expect("KNOB READY at 2.0 s fails", lambda: check(late, refs_path), refuse=True)
        early = root / "early"
        write_run(early, text, refs, ready_at=READY_BEFORE_PS - 1)
        expect("KNOB READY 1 ps earlier passes", lambda: check(early, refs_path))

        warned = root / "warned"
        write_run(warned, text, refs, diagnostic="chip.clock")
        expect("another diagnostic fails", lambda: check(warned, refs_path), refuse=True)

        dim = root / "dim"
        write_run(dim, text, refs, duty="4095/4096@5000")
        expect("duty 4095 of 4096 fails", lambda: check(dim, refs_path), refuse=True)

        short = text.replace("S 2 poll 0000 - 0 - 0 32", "S 2 poll 0100 - 0 - 0 32")
        expect(
            "CS held on the last transaction fails",
            lambda: write_refs(
                [
                    _write(root / "bad-1.txt", short),
                    _write(root / "bad-2.txt", short),
                    _write(root / "bad-3.txt", short),
                ],
                root / "bad.json",
                DEFAULT_START,
            ),
            refuse=True,
        )

        drifted = text.replace("KNOB INFO sample", "KNOB INFO drifted")
        expect(
            "captures that disagree fail",
            lambda: write_refs(
                [_write(root / "d1.txt", text), _write(root / "d2.txt", text), _write(root / "d3.txt", drifted)],
                root / "drift.json",
                DEFAULT_START,
            ),
            refuse=True,
        )

        net_path = root / "net.json"
        net_path.write_text(
            json.dumps(
                {
                    "analysis": {"transient": {"stop_ps": 1, "max_step_ps": 1, "grid_ps": 1}},
                    "parts": [
                        {"name": "u1", "kind": "esp32s3", "console": [{"t_ps": 100_000_000_000, "bytes": [103]}]},
                        {
                            "name": "sw_a",
                            "kind": "switch",
                            "initial": "open",
                            "events": [{"t_ps": 1, "state": "closed"}],
                        },
                        {"name": "sw_b", "kind": "switch", "initial": "open", "events": []},
                        {"name": "u2", "kind": "cst816", "touches": []},
                    ],
                }
            )
        )
        print(script(refs_path, net_path))
        net = json.loads(net_path.read_text())
        parts = {part["name"]: part for part in net["parts"]}
        expect(
            "one closing of a is 80 ms from 2.0 s",
            lambda: (
                parts["sw_a"]["events"]
                == [
                    {"t_ps": CLOSE_FROM_PS, "state": "closed"},
                    {"t_ps": CLOSE_FROM_PS + CLOSE_LEN_PS, "state": "open"},
                ]
                or bad(parts["sw_a"]["events"])
            ),
        )
        expect("b has no closing", lambda: parts["sw_b"]["events"] == [] or bad("b"))
        touch_at = CLOSE_FROM_PS + CLOSE_LEN_PS + TOUCH_APART_PS
        expect(
            "dot 1 is held 300 ms",
            lambda: (
                parts["u2"]["touches"]
                == [
                    {"t_ps": touch_at, "x": 180, "y": 80, "down": True},
                    {"t_ps": touch_at + TOUCH_HOLD_PS, "x": 180, "y": 80, "down": False},
                ]
                or bad(parts["u2"]["touches"])
            ),
        )
        x_ps = touch_at + AFTER_PS
        core = next(part for part in net["parts"] if part["kind"] == "esp32s3")
        expect(
            "g stays and x follows the touch by 500 ms",
            lambda: (
                core["console"]
                == [
                    {"t_ps": 100_000_000_000, "bytes": [103]},
                    {"t_ps": x_ps, "bytes": [120]},
                ]
                or bad(core["console"])
            ),
        )
        expect(
            "the stop moves to 500 ms after x",
            lambda: (
                net["analysis"]["transient"]["stop_ps"] == x_ps + AFTER_PS
                or bad(net["analysis"]["transient"]["stop_ps"])
            ),
        )

        empty = dict(refs)
        empty["events"] = []
        empty_path = root / "empty.json"
        empty_path.write_text(json.dumps(empty))
        empty_net = root / "empty-net.json"
        empty_net.write_text(net_path.read_text())
        bare = json.loads(empty_net.read_text())
        bare["analysis"]["transient"]["stop_ps"] = 1
        bare["parts"][0]["console"] = [{"t_ps": 100_000_000_000, "bytes": [103]}]
        empty_net.write_text(json.dumps(bare))
        print(script(empty_path, empty_net))
        got = json.loads(empty_net.read_text())
        core = next(part for part in got["parts"] if part["kind"] == "esp32s3")
        expect(
            "no event puts x at 2.5 s",
            lambda: core["console"][1]["t_ps"] == CLOSE_FROM_PS + AFTER_PS or bad(core["console"]),
        )

        jumped = dict(refs)
        jumped["events"] = ["K a=2 b=0"]
        jump_path = root / "jump.json"
        jump_path.write_text(json.dumps(jumped))
        expect("a count that skips fails", lambda: script(jump_path, net_path), refuse=True)

        hit = root / "hit.json"
        hit.write_text(
            json.dumps(
                {
                    "analysis": {"transient": {"stop_ps": 1, "max_step_ps": 1, "grid_ps": 1}},
                    "parts": [
                        {
                            "name": "u1",
                            "kind": "esp32s3",
                            "console": [{"t_ps": CLOSE_FROM_PS + AFTER_PS, "bytes": [103]}],
                        },
                        {"name": "sw_a", "kind": "switch", "initial": "open", "events": []},
                        {"name": "sw_b", "kind": "switch", "initial": "open", "events": []},
                        {"name": "u2", "kind": "cst816", "touches": []},
                    ],
                }
            )
        )
        expect("console bytes under 2000 ps apart fail", lambda: script(empty_path, hit), refuse=True)

        def three(name: str, body: str, dest: str):
            paths = []
            for index in range(3):
                path = root / f"{name}-{index}.txt"
                path.write_text(body)
                paths.append(path)
            return write_refs(paths, root / dest, DEFAULT_START)

        k_only = text.replace("K a=1 b=0", "K a=2 b=0")
        expect(
            "captures that differ only in a K line fail",
            lambda: write_refs(
                [_write(root / "k1.txt", text), _write(root / "k2.txt", text), _write(root / "k3.txt", k_only)],
                root / "k.json",
                DEFAULT_START,
            ),
            refuse=True,
        )
        t_only = text.replace("T down 1", "T down 2")
        expect(
            "captures that differ only in a T line fail",
            lambda: write_refs(
                [_write(root / "t1.txt", text), _write(root / "t2.txt", text), _write(root / "t3.txt", t_only)],
                root / "t.json",
                DEFAULT_START,
            ),
            refuse=True,
        )

        def extra_p(payload: bytes) -> str:
            block = f"P 5 00000000 4 {payload.hex()} 20\nS 5 poll 0100 - 0 - 0 32\nS 5 poll 0000 - 0 - 0 32\n"
            return text.replace("KNOB FRAME 0 END\n", block + "KNOB FRAME 0 END\n")

        expect(
            "captures that differ only in a transfer hash fail",
            lambda: write_refs(
                [
                    _write(root / "h1.txt", extra_p(b"\x01\x02\x03\x04")),
                    _write(root / "h2.txt", extra_p(b"\x01\x02\x03\x04")),
                    _write(root / "h3.txt", extra_p(b"\x05\x06\x07\x08")),
                ],
                root / "h.json",
                DEFAULT_START,
            ),
            refuse=True,
        )

        csv_run = root / "csv-run"
        write_run(csv_run, text, refs)
        csv_path = csv_run / "traces" / "display.csv"
        csv_path.write_text(csv_path.read_text().replace("lcd1,0,0,3,1,16,", "lcd1,0,0,4,1,16,", 1))
        expect("a display.csv row that differs fails", lambda: panel(csv_run, refs_path, False), refuse=True)

        frame_run = root / "frame-run"
        image = base64.b64decode(refs["frame_b64"])
        write_run(frame_run, text, refs, frame=bytes([image[0] ^ 0xFF]) + image[1:])
        expect("a frame that differs fails", lambda: panel(frame_run, refs_path, False), refuse=True)

        other_rect = text.replace("P 2 02002a00 4 00000003 ", "P 2 02002a00 4 00000004 ", 1)
        expect(
            "a flush whose CASET gives another rectangle fails",
            lambda: three("rect", other_rect, "rect.json"),
            refuse=True,
        )
        short_bits = text.replace("S 4 queue 0002 - 0 - 0 64", "S 4 queue 0002 - 0 - 0 32", 1)
        expect(
            "a transaction that does not carry the bytes fails",
            lambda: three("bits", short_bits, "bits.json"),
            refuse=True,
        )
        orphan = (
            "C 5 00000000 4 " + ("0" * 16) + " 20\n"
            "B 5 " + base64.b64encode(b"abcd").decode() + "\n"
            "S 5 poll 0100 - 0 - 0 32\n"
            "S 5 queue 0002 - 0 - 0 32\n"
        )
        bad_hash = text.replace("KNOB FRAME 0 END\n", orphan + "KNOB FRAME 0 END\n")
        expect(
            "a transfer whose bytes do not give its hash fails",
            lambda: three("bhash", bad_hash, "bhash.json"),
            refuse=True,
        )
        qio_first = text.replace("S 2 poll 0100 - 0 - 0 32", "S 2 poll 0102 - 0 - 0 32", 1)
        expect("a first transaction on a data lane fails", lambda: three("lane", qio_first, "lane.json"), refuse=True)
        repeated = "P 2 02002a00 4 00000003 11\nS 2 poll 0100 - 0 - 0 32\nS 2 poll 0000 - 0 - 0 32\n"
        expect(
            "a repeated transfer number fails",
            lambda: three("dup", text.replace(repeated, repeated + repeated, 1), "dup.json"),
            refuse=True,
        )
        expect(
            "a capture with no done line fails",
            lambda: three("nodone", text.replace("KNOB DONE\n", ""), "nodone.json"),
            refuse=True,
        )

        def after_done():
            three("after", text + "KNOB INFO after\n", "after.json")
            if "KNOB INFO after" in (root / "after.json").read_text():
                bad("present")

        expect("the line after the done line is not a mark", after_done)

        alien = root / "alien"
        write_run(alien, text, refs)
        serial_file = alien / "traces" / "serial.csv"
        serial_file.write_text(serial_file.read_text().replace(",u1,USB_SERIAL_JTAG,", ",u1,UART0,", 1))
        expect("a serial byte that is not USB_SERIAL_JTAG fails", lambda: panel(alien, refs_path, False), refuse=True)

        bare_net = {
            "analysis": {"transient": {"stop_ps": 1, "max_step_ps": 1, "grid_ps": 1}},
            "parts": [
                {"name": "u1", "kind": "esp32s3", "console": [{"t_ps": 100_000_000_000, "bytes": [103]}]},
                {"name": "sw_a", "kind": "switch", "initial": "open", "events": []},
                {"name": "sw_b", "kind": "switch", "initial": "open", "events": []},
                {"name": "u2", "kind": "cst816", "touches": []},
            ],
        }
        zero = dict(refs)
        zero["events"] = ["T down 0"]
        zero_path = root / "zero.json"
        zero_path.write_text(json.dumps(zero))
        zero_net = root / "zero-net.json"
        zero_net.write_text(json.dumps(bare_net))
        expect("T down 0 fails", lambda: script(zero_path, zero_net), refuse=True)

        def close_touches():
            global TOUCH_HOLD_PS
            saved = TOUCH_HOLD_PS
            TOUCH_HOLD_PS = MIN_GAP_PS - 1
            gap_net = root / "gap-net.json"
            gap_net.write_text(json.dumps(bare_net))
            try:
                script(refs_path, gap_net)
            finally:
                TOUCH_HOLD_PS = saved

        expect("touches under 2000 ps apart fail", close_touches, refuse=True)

        def before_start():
            raw = b"\xff\xfe" + text.encode("ascii")
            paths = []
            for index in range(3):
                path = root / f"bin-{index}.txt"
                path.write_bytes(raw)
                paths.append(path)
            write_refs(paths, root / "bin.json", DEFAULT_START)

        expect("bytes before the start mark are ignored", before_start)

        def non_ascii():
            raw = text.encode("ascii").replace(b"KNOB INFO sample", b"KNOB INFO sampl\xff", 1)
            paths = []
            for index in range(3):
                path = root / f"asc-{index}.txt"
                path.write_bytes(raw)
                paths.append(path)
            write_refs(paths, root / "asc.json", DEFAULT_START)

        expect("a section that is not ASCII fails", non_ascii, refuse=True)
        plain_addr = text.replace("KNOB INFO sample\n", "KNOB INFO sample\nKNOB INFO token 001122334455\n", 1)
        expect("a 12-hex address token fails", lambda: three("addr", plain_addr, "addr.json"), refuse=True)
        colon_addr = text.replace("KNOB INFO sample\n", "KNOB INFO sample\naa:bb:cc:dd:ee:ff\n", 1)
        expect("a colon-form address fails", lambda: three("colon", colon_addr, "colon.json"), refuse=True)

        seen_b = 0
        kept_lines = []
        for line in text.splitlines():
            if line.startswith("B "):
                seen_b += 1
                if seen_b == 2:
                    continue
            kept_lines.append(line)
        expect(
            "a short colour write fails",
            lambda: three("shortc", "\n".join(kept_lines) + "\n", "shortc.json"),
            refuse=True,
        )

        no_bytes = "\n".join(line for line in text.splitlines() if not line.startswith("B ")) + "\n"

        def colour_after():
            try:
                three("dump", no_bytes, "dump.json")
            except ValueError as error:
                if "a colour write after the dump ended" not in str(error):
                    bad(error)
                return
            bad("accepted")

        expect("a colour write after the dump ended is refused", colour_after)

        one = _write(root / "one.txt", text)
        two = _write(root / "two.txt", text)
        three_path = _write(root / "three.txt", text)
        expect("one capture fails", lambda: write_refs([one], root / "one.json", DEFAULT_START), refuse=True)
        expect("two captures fail", lambda: write_refs([one, two], root / "two.json", DEFAULT_START), refuse=True)
        expect(
            "four captures fail",
            lambda: write_refs([one, two, three_path, one], root / "four.json", DEFAULT_START),
            refuse=True,
        )
        expect(
            "an existing refs path fails",
            lambda: write_refs([one, two, three_path], refs_path, DEFAULT_START),
            refuse=True,
        )
        expect(
            "a refs path that is a capture fails",
            lambda: write_refs([one, two, three_path], one, DEFAULT_START),
            refuse=True,
        )
        expect(
            "the same capture path twice fails",
            lambda: write_refs([one, one, two], root / "same.json", DEFAULT_START),
            refuse=True,
        )
        link = root / "link.txt"
        link.symlink_to(one)
        expect(
            "two paths that resolve to one file fail",
            lambda: write_refs([one, link, two], root / "link.json", DEFAULT_START),
            refuse=True,
        )

        def colour_token(suffix: str, dest: str):
            first, rest = text.split("\nB 4 ", 1)
            payload, after = rest.split("\n", 1)
            return three(dest, first + "\nB 4 " + payload + suffix + "\n" + after, dest + ".json")

        expect("a 12-hex token on a colour line is kept", lambda: colour_token(" 001122334455", "btoken"))
        expect("a colon-form token on a colour line is kept", lambda: colour_token(" aa:bb:cc:dd:ee:ff", "bcolon"))

        def line_token(old: str, new: str, dest: str):
            try:
                three(dest, text.replace(old, new, 1), dest + ".json")
            except ValueError as error:
                if "12-hex address token" not in str(error):
                    bad(error)
                print(f"refused ({error})")
                return
            bad("accepted")

        expect(
            "a 12-hex token on a K line fails", lambda: line_token("K a=1 b=0\n", "K a=1 b=0 001122334455\n", "kaddr")
        )
        expect("a 12-hex token on a T line fails", lambda: line_token("T down 1\n", "T down 1 001122334455\n", "taddr"))

        def min_flag():
            sink = io.StringIO()
            old = sys.stderr
            sys.stderr = sink
            try:
                code = main(
                    [
                        "refs",
                        "--write",
                        "--min",
                        "1",
                        str(one),
                        str(two),
                        str(three_path),
                        str(root / "min.json"),
                    ]
                )
            except SystemExit as error:
                code = error.code if isinstance(error.code, int) else 2
            finally:
                sys.stderr = old
            if code == 0 or "unrecognized arguments: --min" not in sink.getvalue():
                bad(sink.getvalue() or code)
            print(f"min flag exit {code}: {sink.getvalue().strip()}")

        expect("a --min flag fails", min_flag)

    print("self-test passed" if not failures else f"self-test: {failures} failed")
    return 1 if failures else 0


def _write(path: Path, text: str) -> Path:
    path.write_text(text)
    return path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--self-test", action="store_true")
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
        if args.self_test:
            return self_test()
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
