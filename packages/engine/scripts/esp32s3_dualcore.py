# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-dualcore` against its two tasks, apart from the engine.

    uv run packages/engine/scripts/esp32s3_dualcore.py --check RUN_DIR
    uv run packages/engine/scripts/esp32s3_dualcore.py --self-test

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


def serial_of(text: str) -> str:
    rows = "".join(f"0,u1,USB_SERIAL_JTAG,{ord(c)}\n" for c in text)
    return "t_ps,part,usart,byte\n" + rows


def transcript(end: str = "\r\n", swap: bool = False) -> str:
    """Twenty ticks from each core, interleaved one line at a time, then both done lines."""
    lines_out = []
    order = (1, 0) if swap else (0, 1)
    for k in range(TICKS):
        for core in order:
            lines_out.append(f"core {core} tick {k}{end}")
    lines_out.append(f"done{end}")
    lines_out.append(f"done{end}")
    return "".join(lines_out)


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

    good = lines(serial_of("I (57) boot\r\n" + transcript()))
    expect("a good run passes", not raises(lambda: check_console(good)))
    bare = lines(serial_of(transcript(end="\n")))
    expect("a line feed passes", not raises(lambda: check_console(bare)))
    swapped = lines(serial_of(transcript(swap=True)))
    expect("the other core first passes", not raises(lambda: check_console(swapped)))
    lost = lines(serial_of(transcript().replace("core 1 tick 7\r\n", "", 1)))
    expect("a lost tick fails", raises(lambda: check_console(lost)))
    disorder = lines(serial_of(transcript().replace("core 0 tick 3\r\n", "core 0 tick 4\r\n", 1)))
    expect("a tick out of order fails", raises(lambda: check_console(disorder)))
    one_done = lines(serial_of(transcript().replace("done\r\n", "", 1)))
    expect("one done line fails", raises(lambda: check_console(one_done)))
    early = []
    for k in range(TICKS):
        if k == TICKS - 1:
            early.append("done\r\n")
        for core in CORES:
            early.append(f"core {core} tick {k}\r\n")
    early.append("done\r\n")
    expect("a done line before the last tick fails", raises(lambda: check_console(lines(serial_of("".join(early))))))
    owned = []
    for core in CORES:
        for k in range(TICKS):
            owned.append(f"core {core} tick {k}\r\n")
        owned.append("done\r\n")
    expect("a done after that core's ticks passes", not raises(lambda: check_console(lines(serial_of("".join(owned))))))
    expect("an error fails", raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])))
    expect(
        "a warning passes",
        not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])),
    )
    other = serial_of(transcript()).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
    }
    listed = {
        "blobs": [{"role": "rom", "sha256": "r", "bytes": 1}, {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8}]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    unplaced = {"blobs": [listed["blobs"][0], {**listed["blobs"][1], "offset": None}]}
    expect("a region without its offset fails", raises(lambda: check_manifest(unplaced, {"parts": [part]})))
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
    except (ValueError, OSError, KeyError) as error:
        print(f"esp32s3-dualcore: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
