# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of the console sketch, apart from the engine.

    uv run packages/engine/scripts/esp32s3_arduino_console.py --check RUN_DIR

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
    known = {"USB_SERIAL_JTAG", "UART0", "UART1", "UART2"}
    if any(row["part"] != "u1" or row["usart"] not in known for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG or a UART")
    rows = [row for row in rows if row["usart"] == "USB_SERIAL_JTAG"]
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-arduino console: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
