# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of the console sketch, apart from the engine.

    uv run packages/engine/scripts/esp32s3_arduino_console.py --check RUN_DIR
    uv run packages/engine/scripts/esp32s3_arduino_console.py --self-test

The sketch prints `tick <k> core <c>` from `loop` and echoes one byte each pass as `got <b>`.
The check wants `tick 0 core 1` through `tick 10 core 1`, in order, and a `got` line for each
console byte, in order. Other lines may sit between them.

`traces/serial.csv` must hold those lines as `u1`'s `USB_SERIAL_JTAG`. The run must hold no
error, and its `manifest.json` must list the netlist's ROM, then each flash region with its
offset. The netlist is `packages/engine/tests/firmware/esp32s3/arduino/checks/console/netlist.json`.
"""

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
NETLIST = ROOT / "packages/engine/tests/firmware" / "esp32s3" / "arduino" / "checks" / "console" / "netlist.json"
TICK = re.compile(r"tick (\d+) core (\d+)\r?\n")
GOT = re.compile(r"got (\d+)\r?\n")
LAST_TICK = 10


def lines(serial_csv: str) -> list[str]:
    """Each console line."""
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    if any(row["part"] != "u1" or row["usart"] != "USB_SERIAL_JTAG" for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG")
    out, line = [], ""
    for row in rows:
        line += chr(int(row["byte"]))
        if line.endswith("\n"):
            out.append(line)
            line = ""
    if line:
        out.append(line)
    return out


def ticks(got: list[str]) -> list[int]:
    """The core-1 tick numbers, in the order printed."""
    found = []
    for line in got:
        match = TICK.fullmatch(line)
        if not match:
            continue
        number, core = int(match.group(1)), int(match.group(2))
        if core != 1:
            raise ValueError(f"tick {number} ran on core {core}")
        found.append(number)
    return found


def check_ticks(got: list[str]) -> None:
    found = ticks(got)
    want = list(range(LAST_TICK + 1))
    if found[: len(want)] != want:
        raise ValueError(f"the console printed ticks {found}, not {want[0]} to {LAST_TICK} on core 1")


def echoed(got: list[str]) -> list[int]:
    return [int(GOT.fullmatch(line).group(1)) for line in got if GOT.fullmatch(line)]


def sent_bytes(netlist: dict) -> list[int]:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    return [int(byte) for event in part.get("console", []) for byte in event["bytes"]]


def check_echo(got: list[str], netlist: dict) -> None:
    want = sent_bytes(netlist)
    found = echoed(got)
    if found != want:
        raise ValueError(f"the console echoed {found}, not {want}")


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [
        {"role": "flash", "sha256": region["sha256"], "bytes": region["bytes"], "offset": region["offset"]}
        for region in part["firmware"]["flash"]
    ]
    if manifest["blobs"] != want:
        raise ValueError(f"manifest.json lists {manifest['blobs']}, not {want}")


def check_diagnostics(diagnostics: list[dict]) -> None:
    errors = [item["code"] for item in diagnostics if item["severity"] == "error"]
    if errors:
        raise ValueError(f"the run holds errors {errors}")


def echoed_phrase(count: int) -> str:
    noun = "byte" if count == 1 else "bytes"
    return f"{count} {noun} echoed"


def check(run: Path) -> None:
    netlist = json.loads(NETLIST.read_text())
    got = lines((run / "traces" / "serial.csv").read_text())
    check_ticks(got)
    check_echo(got, netlist)
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    print(f"esp32s3-arduino console: ticks 0 to {LAST_TICK} on core 1, {echoed_phrase(len(sent_bytes(netlist)))}")


def serial_of(text: str) -> str:
    rows = "".join(f"0,u1,USB_SERIAL_JTAG,{ord(c)}\n" for c in text)
    return "t_ps,part,usart,byte\n" + rows


def transcript(end: str = "\n", ticks_through: int = LAST_TICK, core: int = 1, extra: str = "") -> str:
    body = "".join(f"tick {k} core {core}{end}" for k in range(ticks_through + 1))
    return body + extra


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

    good = lines(serial_of("boot\n" + transcript()))
    expect("ticks 0 to 10 pass", not raises(lambda: check_ticks(good)))
    crlf = lines(serial_of(transcript(end="\r\n")))
    expect("a carriage return passes", not raises(lambda: check_ticks(crlf)))
    short = lines(serial_of(transcript(ticks_through=9)))
    expect("a missing tick 10 fails", raises(lambda: check_ticks(short)))
    other = lines(serial_of(transcript(core=0)))
    expect("core 0 fails", raises(lambda: check_ticks(other)))
    netlist = {"parts": [{"kind": "esp32s3", "console": [{"t_ps": 1, "bytes": [65, 66]}]}]}
    echoed_text = lines(serial_of(transcript(extra="got 65\ngot 66\n")))
    expect("the echoed bytes pass", not raises(lambda: check_echo(echoed_text, netlist)))
    missed = lines(serial_of(transcript(extra="got 65\n")))
    expect("a missing echo fails", raises(lambda: check_echo(missed, netlist)))
    expect(
        "an error fails",
        raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])),
    )
    expect(
        "a warning passes",
        not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])),
    )
    foreign = serial_of(transcript()).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(foreign)))
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
    }
    listed = {
        "blobs": [
            {"role": "rom", "sha256": "r", "bytes": 1},
            {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8},
        ]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    expect("one byte is singular", echoed_phrase(1) == "1 byte echoed")
    expect("two bytes stay plural", echoed_phrase(2) == "2 bytes echoed")
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
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-arduino console: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
