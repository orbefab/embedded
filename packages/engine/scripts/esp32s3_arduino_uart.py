# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of the console sketch with `Serial` on UART0, apart from the engine.

    uv run packages/engine/scripts/esp32s3_arduino_uart.py --check RUN_DIR

The sketch is the console sketch, built with CDC on boot disabled, so `Serial` is UART0.
It prints `tick <k> core <c>` from `loop` and echoes one byte each pass as `got <b>`.
The check wants `tick 0 core 1` through `tick 10 core 1`, in order, and a `got` line for
each byte a netlist `serial` source sends, in order. Other lines may sit between them.

`traces/serial.csv` must hold those lines as `u1`'s `UART0`. A `serial` source also drives
the RX pad, and the UART logs that byte with its start-bit time. The check drops each such
row once: the same byte, and a `t_ps` within one frame of the source byte's start. One frame
is `floor(10 * 10^12 / baud)` picoseconds, the span from bit boundary 0 to bit boundary 10.
A later copy of the byte, such as the `g` in `got 103`, stays. The run must hold no error,
and its `manifest.json` must list the netlist's ROM, then each flash region with its offset.
The netlist is `packages/engine/tests/firmware/esp32s3/arduino/checks/uart/netlist.json`.
"""

import argparse
import csv
import io
import json
import sys
from pathlib import Path

from esp32s3_arduino_console import (
    LAST_TICK,
    check_diagnostics,
    check_manifest,
    check_ticks,
    echoed,
    echoed_phrase,
)

ROOT = Path(__file__).resolve().parents[3]
NETLIST = ROOT / "packages/engine/tests/firmware" / "esp32s3" / "arduino" / "checks" / "uart" / "netlist.json"
KNOWN = {"USB_SERIAL_JTAG", "UART0", "UART1", "UART2"}


def frame_ps(baud: int) -> int:
    """The picoseconds from a frame's start bit to the next frame's start bit."""
    return (10 * 10**12) // baud


def source_starts(netlist: dict) -> list[tuple[int, int, int]]:
    """Each byte a `serial` source sends, as its start-bit time, baud and byte."""
    found = []
    for part in netlist["parts"]:
        voltage = part.get("voltage")
        if not isinstance(voltage, dict) or voltage.get("kind") != "serial":
            continue
        for event in voltage["events"]:
            origin = int(event["t_ps"])
            baud = int(event["baud"])
            for index, byte in enumerate(event["bytes"]):
                start = origin + (10 * index * 10**12) // baud
                found.append((start, baud, int(byte)))
    return found


def strip_source(rows: list[dict], sources: list[tuple[int, int, int]]) -> list[dict]:
    """`rows` without one UART row for each source byte, matched once by byte and start time."""
    used = [False] * len(sources)
    kept = []
    for row in rows:
        at = int(row["t_ps"])
        byte = int(row["byte"])
        drop = False
        for index, (start, baud, want) in enumerate(sources):
            if used[index] or byte != want or abs(at - start) > frame_ps(baud):
                continue
            used[index] = True
            drop = True
            break
        if not drop:
            kept.append(row)
    return kept


def lines(serial_csv: str, netlist: dict) -> list[str]:
    """Each UART0 line, after the serial-source bytes are dropped."""
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    if any(row["part"] != "u1" or row["usart"] not in KNOWN for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG or a UART")
    uart = [row for row in rows if row["usart"] == "UART0"]
    kept = strip_source(uart, source_starts(netlist))
    out, line = [], ""
    for row in kept:
        line += chr(int(row["byte"]))
        if line.endswith("\n"):
            out.append(line)
            line = ""
    if line:
        out.append(line)
    return out


def sent_bytes(netlist: dict) -> list[int]:
    return [byte for _, _, byte in source_starts(netlist)]


def check_echo(got: list[str], netlist: dict) -> None:
    want = sent_bytes(netlist)
    found = echoed(got)
    if found != want:
        raise ValueError(f"the console echoed {found}, not {want}")


def check(run: Path) -> None:
    netlist = json.loads(NETLIST.read_text())
    got = lines((run / "traces" / "serial.csv").read_text(), netlist)
    check_ticks(got)
    check_echo(got, netlist)
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    print(f"esp32s3-arduino uart: ticks 0 to {LAST_TICK} on core 1, {echoed_phrase(len(sent_bytes(netlist)))}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-arduino uart: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
