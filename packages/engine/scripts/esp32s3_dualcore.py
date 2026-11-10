# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-dualcore` against its two tasks, apart from the engine.

    uv run packages/engine/scripts/esp32s3_dualcore.py --check RUN_DIR

The app of `packages/engine/tests/firmware/esp32s3/idf/dualcore` pins one task to each core. Each prints
`core <c> tick <k>` twenty times, `k` from 0, and then `done`. The two tasks may interleave.
A `done` line names no core. The check gives it to a core that has already printed tick 19
and has no `done` yet. A line may end with `\\n` or `\\r\\n`.

`traces/serial.csv` must hold those lines as `u1`'s `USB_SERIAL_JTAG`. The run must hold no
error, and its `manifest.json` must list the netlist's ROM, then each flash region with its
offset.
"""

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-dualcore"
TICK = re.compile(r"core ([01]) tick (\d+)\r?\n")
DONE = re.compile(r"done\r?\n")
CORES = (0, 1)
TICKS = 20


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


def check_console(got: list[str]) -> None:
    """Each core's twenty ticks in order, and a `done` after that core's last tick.

    A `done` line names no core, so it is given to the lowest-numbered core that has already
    printed ticks 0 to 19 and has no `done` yet. Other lines may sit between them.
    """
    ticks = {core: [] for core in CORES}
    done = {core: False for core in CORES}
    for line in got:
        match = TICK.fullmatch(line)
        if match:
            core = int(match.group(1))
            if done[core]:
                raise ValueError(f"core {core} printed a tick after done")
            ticks[core].append(int(match.group(2)))
        elif DONE.fullmatch(line):
            ready = [core for core in CORES if ticks[core] == list(range(TICKS)) and not done[core]]
            if not ready:
                raise ValueError("a done line before a core's last tick")
            done[ready[0]] = True
    for core in CORES:
        if ticks[core] != list(range(TICKS)):
            raise ValueError(f"core {core} printed {ticks[core]}, not ticks 0 to {TICKS - 1}")
        if not done[core]:
            raise ValueError(f"core {core} has no done line after its last tick")


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [
        {"role": "flash", "sha256": r["sha256"], "bytes": r["bytes"], "offset": r["offset"]}
        for r in part["firmware"]["flash"]
    ]
    if manifest["blobs"] != want:
        raise ValueError(f"manifest.json lists {manifest['blobs']}, not {want}")


def check_diagnostics(diagnostics: list[dict]) -> None:
    errors = [d["code"] for d in diagnostics if d["severity"] == "error"]
    if errors:
        raise ValueError(f"the run holds errors {errors}")


def check(run: Path) -> None:
    netlist = json.loads((EXAMPLE / "netlist.json").read_text())
    got = lines((run / "traces" / "serial.csv").read_text())
    check_console(got)
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    print(f"esp32s3-dualcore: each core printed {TICKS} ticks, then done")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError) as error:
        print(f"esp32s3-dualcore: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
