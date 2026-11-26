# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-millis` against its sketch, apart from the engine.

    uv run packages/engine/scripts/esp32s3_millis.py --check RUN_DIR

The sketch is `packages/engine/tests/firmware/esp32s3/arduino/stock/millis/millis.ino`. Its
`loop` never waits: it reads `millis()` and toggles GPIO4 each time 500 more milliseconds have
passed since the start, so the pin first rises 500 ms after `setup` and then changes every
500 ms. `millis()` counts from the boot, a few milliseconds before `setup`.

In `traces/logic.vcd`, `u1`'s GPIO4 must change once for each whole PERIOD in the run's stop
time after the first BOOT, rising first and alternating. The first edge must lie from PERIOD to
PERIOD + BOOT after the start, and each later edge PERIOD after the one before it, within
SLACK. SLACK, 10 µs, covers the loop's own pass and the boot's offset of the millisecond count,
and is far below the 1 ms a wrong count of `millis()` would move an edge.
"""

import argparse
import json
import re
import sys
from pathlib import Path

PS_PER_MS = 1_000_000_000
PERIOD = 500 * PS_PER_MS
BOOT = 20 * PS_PER_MS
SLACK = 10_000_000


def gpio4_changes(vcd: str) -> list[tuple[int, str]]:
    ids = re.findall(r"\$var wire 1 (\S+) GPIO4 \$end", vcd)
    if len(ids) != 1:
        raise ValueError(f"logic.vcd has {len(ids)} GPIO4 signals, not 1")
    ident = ids[0]
    body = vcd.split("$end\n#0\n$dumpvars\n", 1)[1].split("$end\n", 1)[1]
    changes, now = [], None
    for line in body.splitlines():
        if line.startswith("#"):
            now = int(line[1:])
        elif line[1:] == ident:
            changes.append((now, line[0]))
    return changes


def check_edges(vcd: str, stop_ps: int) -> int:
    """The number of edges, once each sits where the sketch puts it."""
    changes = gpio4_changes(vcd)
    edges = (stop_ps - BOOT) // PERIOD
    levels = [level for _, level in changes]
    want = ["1" if k % 2 else "0" for k in range(1, edges + 1)]
    if levels != want:
        raise ValueError(f"GPIO4 changed to {levels}, not {want}")
    first = changes[0][0]
    if not PERIOD <= first <= PERIOD + BOOT:
        raise ValueError(f"the first edge is at {first} ps, not from {PERIOD} to {PERIOD + BOOT} ps")
    for k in range(1, edges):
        gap = changes[k][0] - changes[k - 1][0]
        if abs(gap - PERIOD) > SLACK:
            raise ValueError(f"edge {k + 1} is {gap} ps after edge {k}, not {PERIOD} ps within {SLACK} ps")
    return edges


def check(run: Path) -> None:
    netlist = json.loads((run / "netlist.json").read_text())
    stop_ps = netlist["analysis"]["transient"]["stop_ps"]
    edges = check_edges((run / "traces" / "logic.vcd").read_text(), stop_ps)
    print(f"esp32s3-millis: {edges} edges, each {PERIOD // PS_PER_MS} ms after the last within {SLACK // 1_000_000} us")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except ValueError as err:
        print(f"esp32s3-millis: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
