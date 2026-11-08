# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-touch` against its controller, apart from the engine.

    uv run packages/engine/scripts/esp32s3_touch.py --check RUN_DIR
    uv run packages/engine/scripts/esp32s3_touch.py --self-test

The app of `packages/engine/tests/firmware/esp32s3/idf/touch` holds TP_RST on GPIO8 low for 10 ms, then reads the
controller at 0x15. It prints `id b4` for register 0xA7, then ten `poll f=0` lines for register
0x02, 33 ms apart. Each falling edge on TP_INT, GPIO7, prints `touch f=<n> x=<x> y=<y>`.

`traces/serial.csv` must hold those lines in that order. Each touch line reads `f=1`, and its
X and Y are a point of the run's own `netlist.json`, in time order. `packages/engine/scripts/i2c_decode.py --timing-s3 400000` must accept the run,
and the bytes it decodes on the chip's pins must be those lines: a write of 0xA7 and a read of
0xb4, ten writes of 0x02 and reads of five 0 bytes, then one write of 0x02 and the five bytes of
each point. The run must hold no error and no `chip.unmodeled_register` for `FILTER_CFG`, and
its `manifest.json` must list the netlist's ROM, then each flash region with its offset.
"""

import argparse
import csv
import io
import json
import re
import subprocess
import sys
from pathlib import Path

import i2c_decode

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-touch"
ID = re.compile(r"id ([0-9a-f]{2})\r\n")
POLL = re.compile(r"poll f=(\d+)\r\n")
TOUCH = re.compile(r"touch f=(\d+) x=(\d+) y=(\d+)\r\n")
FILTER_CFG = 0x6001_3050
POLLS = 10


def lines(serial_csv: str) -> list[tuple[int, str]]:
    """Each console line with the time of its first byte."""
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    if any(row["part"] != "u1" or row["usart"] != "USB_SERIAL_JTAG" for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG")
    out, line, at = [], "", 0
    for row in rows:
        if not line:
            at = int(row["t_ps"])
        line += chr(int(row["byte"]))
        if line.endswith("\n"):
            out.append((at, line))
            line = ""
    if line:
        out.append((at, line))
    return out


def points(netlist: dict) -> list[tuple[int, int]]:
    """Each touch the netlist holds down, in the order it lists them."""
    part = next(part for part in netlist["parts"] if part["kind"] == "cst816")
    return [(event["x"], event["y"]) for event in part["touches"] if event["down"]]


def transfers(phases: list[dict]) -> list[tuple[list[int], list[int]]]:
    """Each write of a register followed by its read, as `(written, read)`."""
    frames = []
    current = None
    for phase in phases:
        kind = phase["phase"]
        if kind in ("start", "repeated_start"):
            if current:
                frames.append(current)
            current = {"read": None, "data": []}
        elif kind == "address" and current is not None:
            current["read"] = phase["read"]
        elif kind == "data" and current is not None:
            current["data"].append(phase["byte"])
        elif kind == "stop" and current is not None:
            frames.append(current)
            current = None
    if current:
        frames.append(current)
    out = []
    index = 0
    while index + 1 < len(frames):
        write, read = frames[index], frames[index + 1]
        if write["read"] == 0 and read["read"] == 1:
            out.append((write["data"], read["data"]))
            index += 2
        else:
            index += 1
    return out


def point_bytes(fingers: int, x: int, y: int) -> list[int]:
    return [fingers, (x >> 8) & 0x0F, x & 0xFF, (y >> 8) & 0x0F, y & 0xFF]


def console_reads(got: list[tuple[int, str]], touched: list[tuple[int, int]]) -> list[tuple[list[int], list[int]]]:
    """The register reads the console lines name, in order."""
    matched = [(kind, match) for _, line in got for kind, match in (
        ("id", ID.fullmatch(line)),
        ("poll", POLL.fullmatch(line)),
        ("touch", TOUCH.fullmatch(line)),
    ) if match]
    ids = [match for kind, match in matched if kind == "id"]
    polls = [match for kind, match in matched if kind == "poll"]
    touches = [match for kind, match in matched if kind == "touch"]
    if [kind for kind, _ in matched] != ["id"] + ["poll"] * POLLS + ["touch"] * len(touched):
        raise ValueError("the console is not one id line, ten polls and one touch line per point")
    if ids[0].group(1) != "b4":
        raise ValueError(f"the id line reads {ids[0].group(1)}, not b4")
    if any(match.group(1) != "0" for match in polls):
        raise ValueError("a poll line reads fingers other than 0")
    want = [([0xA7], [0xB4])]
    want += [([0x02], [0, 0, 0, 0, 0]) for _ in polls]
    for match, (x, y) in zip(touches, touched):
        fingers, got_x, got_y = (int(match.group(k)) for k in (1, 2, 3))
        if fingers != 1:
            raise ValueError(f"a touch line reads f={fingers}, not f=1")
        if (got_x, got_y) != (x, y):
            raise ValueError(f"a touch line reads x={got_x} y={got_y}, not x={x} y={y}")
        want.append(([0x02], point_bytes(fingers, x, y)))
    return want


def check_diagnostics(diagnostics: list[dict]) -> None:
    errors = [item["code"] for item in diagnostics if item["severity"] == "error"]
    if errors:
        raise ValueError(f"the run holds errors {errors}")
    named = [
        item for item in diagnostics
        if item["code"] == "chip.unmodeled_register" and item["data"]["address"] == FILTER_CFG
    ]
    if named:
        raise ValueError("FILTER_CFG is listed as chip.unmodeled_register")


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [
        {"role": "flash", "sha256": region["sha256"], "bytes": region["bytes"], "offset": region["offset"]}
        for region in part["firmware"]["flash"]
    ]
    if manifest["blobs"] != want:
        raise ValueError(f"manifest.json lists {manifest['blobs']}, not {want}")


def chip_phases(run: Path, netlist: dict) -> list[dict]:
    """The phases the esp32s3 view decodes."""
    _atmega, extra = i2c_decode.bus_views(netlist)
    view = next((label, scl, sda) for label, scl, sda in extra if scl[1].startswith("GPIO"))
    vcd = i2c_decode.read_vcd(run / "traces" / "logic.vcd")
    return i2c_decode.decode(vcd[view[1]], vcd[view[2]])


def check(run: Path) -> None:
    netlist = json.loads((run / "netlist.json").read_text())
    touched = points(netlist)
    got = lines((run / "traces" / "serial.csv").read_text())
    want = console_reads(got, touched)
    decoded = transfers(chip_phases(run, netlist))
    if decoded != want:
        raise ValueError(f"the bus decoded {decoded}, not {want}")
    timing = subprocess.run(
        ["uv", "run", "packages/engine/scripts/i2c_decode.py", str(run), "--timing-s3", "400000"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if timing.returncode != 0:
        raise ValueError(timing.stdout + timing.stderr)
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    print(f"esp32s3-touch: id b4, {POLLS} idle polls and {len(touched)} touches, each read on the bus at 400 kHz")


def serial_of(timed: list[tuple[int, str]]) -> str:
    rows = "".join(f"{at},u1,USB_SERIAL_JTAG,{ord(c)}\n" for at, line in timed for c in line)
    return "t_ps,part,usart,byte\n" + rows


def read_of(reg: int, data: list[int]) -> list[dict]:
    phases = [
        {"phase": "start"},
        {"phase": "address", "read": 0},
        {"phase": "data", "byte": reg, "read": 0},
        {"phase": "repeated_start"},
        {"phase": "address", "read": 1},
    ]
    phases += [{"phase": "data", "byte": byte, "read": 1} for byte in data]
    phases.append({"phase": "stop"})
    return phases


def self_test() -> int:
    failures = 0

    def expect(name: str, ok: bool) -> None:
        nonlocal failures
        if not ok:
            print(f"FAIL {name}")
            failures += 1

    def raises(fn) -> bool:
        try:
            fn()
        except ValueError:
            return True
        return False

    touched = [(291, 69), (16, 32)]
    timed = [
        (1, "id b4\r\n"),
        *[(2 + n, "poll f=0\r\n") for n in range(POLLS)],
        (20, "touch f=1 x=291 y=69\r\n"),
        (21, "touch f=1 x=16 y=32\r\n"),
    ]
    phases = read_of(0xA7, [0xB4])
    phases += [phase for _ in range(POLLS) for phase in read_of(0x02, [0, 0, 0, 0, 0])]
    phases += read_of(0x02, point_bytes(1, 291, 69))
    phases += read_of(0x02, point_bytes(1, 16, 32))

    def console(change=None):
        rows = [(at, change(line) if change else line) for at, line in timed]
        return lines(serial_of([row for row in rows if row[1]]))

    expect("a good console names the reads", console_reads(console(), touched)[-1] == ([0x02], point_bytes(1, 16, 32)))
    expect("a good bus matches the console", transfers(phases) == console_reads(console(), touched))
    expect("a wrong x fails", raises(lambda: console_reads(console(lambda line: line.replace("x=291", "x=292")), touched)))
    expect("a missing id fails", raises(lambda: console_reads(console(lambda line: "" if line.startswith("id") else line), touched)))
    expect("a touch with no finger fails", raises(lambda: console_reads(console(lambda line: line.replace("touch f=1", "touch f=0", 1)), touched)))
    expect("a poll with fingers fails", raises(lambda: console_reads(console(lambda line: line.replace("poll f=0", "poll f=1", 1)), touched)))
    expect("a short bus differs", transfers(phases[:-2]) != console_reads(console(), touched))
    expect("an error fails", raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])))
    expect("another kept register passes", not raises(lambda: check_diagnostics([
        {"code": "chip.unmodeled_register", "severity": "warning", "data": {"address": 0x60013004}}
    ])))
    expect("FILTER_CFG fails", raises(lambda: check_diagnostics([
        {"code": "chip.unmodeled_register", "severity": "warning", "data": {"address": FILTER_CFG}}
    ])))
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
    }
    listed = {"blobs": [
        {"role": "rom", "sha256": "r", "bytes": 1},
        {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8},
    ]}
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    unplaced = {"blobs": [listed["blobs"][0], {**listed["blobs"][1], "offset": None}]}
    expect("a region without its offset fails", raises(lambda: check_manifest(unplaced, {"parts": [part]})))
    other = serial_of(timed).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    committed = points(json.loads((EXAMPLE / "netlist.json").read_text()))
    expect("the committed touches are the two points", committed == [(291, 69), (16, 32)])
    print("self-test passed" if not failures else f"self-test: {failures} failed")
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--check", type=Path, metavar="RUN_DIR")
    group.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        print(f"esp32s3-touch: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
