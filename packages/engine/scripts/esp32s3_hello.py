# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-hello` against the board's console, apart from the engine.

    uv run packages/engine/scripts/esp32s3_hello.py --check RUN_DIR

`packages/engine/tests/circuits/esp32s3-hello/console.json` holds the console text a board printed
for the example's app, from its first log line through `Restarting now.`, with each IDF log time
masked to `T`. The
board's log times count from its power-on, which the run starts after, so only the masked text
compares.

`traces/serial.csv` must hold that text, byte for byte once masked, as `u1`'s `USB_SERIAL_JTAG`.
The app prints `Restarting in N seconds...` once a second, through FreeRTOS ticks of 10 ms, so
every adjacent pair of those lines, the first pair included, must lie one second apart, within a
tick. The
run's last diagnostic must be `chip.restart`, one second after the last such line within a tick,
and the run must hold no error.
Its `manifest.json` must list the netlist's ROM, then each flash region with its offset.
"""

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-hello"
CONSOLE = EXAMPLE / "console.json"
LOG_TIME = re.compile(r"^([IWE]) \(\d+\) ", re.M)
COUNTDOWN = re.compile(r"Restarting in \d+ seconds\.\.\.\r\n")
SECOND_PS = 10**12
TICK_PS = 10**10


def masked(text: str) -> str:
    return LOG_TIME.sub(r"\1 (T) ", text)


def lines(serial_csv: str) -> list[tuple[int, str]]:
    """Each console line with the time of its first byte."""
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    known = {"USB_SERIAL_JTAG", "UART0", "UART1", "UART2"}
    if any(row["part"] != "u1" or row["usart"] not in known for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG or a UART")
    rows = [row for row in rows if row["usart"] == "USB_SERIAL_JTAG"]
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


def check_text(got: list[tuple[int, str]], want: str) -> None:
    text = masked("".join(line for _, line in got))
    if text != want:
        at = next((i for i, (a, b) in enumerate(zip(text, want, strict=False)) if a != b), min(len(text), len(want)))
        raise ValueError(f"the console differs from the board's at byte {at}: {text[at : at + 60]!r}")


def check_times(got: list[tuple[int, str]], diagnostics: list[dict]) -> int:
    """The countdown lines a second apart, then the restart. Returns the countdown's length."""
    stamps = [at for at, line in got if COUNTDOWN.fullmatch(line)]
    errors = [d["code"] for d in diagnostics if d["severity"] == "error"]
    if errors:
        raise ValueError(f"the run holds errors {errors}")
    if not diagnostics or diagnostics[-1]["code"] != "chip.restart":
        raise ValueError("the run does not end with chip.restart")
    stamps.append(diagnostics[-1]["t_ps"])
    for before, after in zip(stamps, stamps[1:], strict=False):
        if abs(after - before - SECOND_PS) > TICK_PS:
            raise ValueError(f"{after - before} ps between two countdown steps, not a second")
    return len(stamps) - 1


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [
        {"role": "flash", "sha256": r["sha256"], "bytes": r["bytes"], "offset": r["offset"]}
        for r in part["firmware"]["flash"]
    ]
    if manifest["blobs"] != want:
        raise ValueError(f"manifest.json lists {manifest['blobs']}, not {want}")


def check(run: Path) -> None:
    want = json.loads(CONSOLE.read_text())["transcript"]
    got = lines((run / "traces" / "serial.csv").read_text())
    check_text(got, want)
    steps = check_times(got, json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), json.loads((EXAMPLE / "netlist.json").read_text()))
    print(
        f"esp32s3-hello: {len(want)} console bytes as the board printed them, "
        f"{steps} countdown lines a second apart, then chip.restart"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError) as error:
        print(f"esp32s3-hello: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
