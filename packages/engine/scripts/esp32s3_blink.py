# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-blink` against its program, apart from the engine.

    uv run packages/engine/scripts/esp32s3_blink.py --check RUN_DIR

The expected console text is the block between `EXPECTED-BEGIN` and `EXPECTED-END` in
`packages/engine/tests/firmware/esp32s3/blink.c`. The check first holds that text to the program's closed form: the
period line gives PERIOD in cycles, edge k reads the level edge k - 1 left, from low before
edge 1, and the SYSTIMER line is (EDGES - 1) * PERIOD * 2 / 5, its 16 MHz count over the
40 MHz clock.

Then RUN_DIR. `traces/serial.csv` must hold that text, byte for byte, as `u1`'s
`USB_SERIAL_JTAG`, with every byte of one line at one `t_ps`, the cycle its packet went. In
`traces/logic.vcd`, `u1`'s GPIO21 must change EDGES times, rising first and alternating,
edge k at cycle k * PERIOD + LATENCY of the 25 ns cycle. LATENCY, 18, is the cycles from the
CCOMPARE1 match to the store that changes the pin. It follows from the core's cycle costs and
from the run landing an edge on its store's cycle, so a change to either fails the check.
"""

import argparse
import csv
import io
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "packages/engine/tests/firmware" / "esp32s3" / "blink.c"
PS_PER_CYCLE = 25_000
LATENCY = 18


def expected_text(source: str) -> str:
    block = source.split("EXPECTED-BEGIN\n", 1)[1].split(" * EXPECTED-END", 1)[0]
    return "".join(line.removeprefix(" * ") + "\n" for line in block.splitlines())


def closed_form(text: str) -> tuple[int, int]:
    """PERIOD and EDGES, once the text agrees with the program's closed form."""
    lines = text.splitlines()
    fields = dict(line.split(" ", 1) for line in lines if line.split(" ", 1)[0] != "edge")
    period = int(fields["period"], 16)
    edges = int(fields["edges"], 16)
    want = [f"edge {k:08x} in {(k - 1) % 2:08x}" for k in range(1, edges + 1)]
    got = [line for line in lines if line.startswith("edge ")]
    if got != want:
        raise ValueError(f"edge lines {got} are not {want}")
    systimer = (edges - 1) * period * 2 // 5
    if int(fields["systimer"], 16) != systimer:
        raise ValueError(f"systimer {fields['systimer']} is not {systimer:08x}")
    if lines[0] != "BEGIN blink" or lines[-1] != "END blink":
        raise ValueError("the text does not open with BEGIN blink and close with END blink")
    return period, edges


def check_serial(serial_csv: str, text: str) -> None:
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    if any(row["part"] != "u1" or row["usart"] != "USB_SERIAL_JTAG" for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG")
    got = bytes(int(row["byte"]) for row in rows).decode("ascii", "replace")
    if got != text:
        raise ValueError(f"serial.csv reads {got!r}, not {text!r}")
    at = 0
    for line in text.splitlines(keepends=True):
        stamps = {int(row["t_ps"]) for row in rows[at : at + len(line)]}
        if len(stamps) != 1:
            raise ValueError(f"the line {line!r} went at {sorted(stamps)}, not at one time")
        if stamps.pop() % PS_PER_CYCLE:
            raise ValueError(f"the line {line!r} went between two cycles")
        at += len(line)


def gpio21_changes(vcd: str) -> list[tuple[int, str]]:
    ids = re.findall(r"\$var wire 1 (\S+) GPIO21 \$end", vcd)
    if len(ids) != 1:
        raise ValueError(f"logic.vcd has {len(ids)} GPIO21 signals, not 1")
    ident = ids[0]
    body = vcd.split("$end\n#0\n$dumpvars\n", 1)[1].split("$end\n", 1)[1]
    changes, now = [], None
    for line in body.splitlines():
        if line.startswith("#"):
            now = int(line[1:])
        elif line[1:] == ident:
            changes.append((now, line[0]))
    return changes


def check_edges(vcd: str, period: int, edges: int) -> None:
    """Every edge sits at k * period + LATENCY."""
    changes = gpio21_changes(vcd)
    levels = [level for _, level in changes]
    want = ["1" if k % 2 else "0" for k in range(1, edges + 1)]
    if levels != want:
        raise ValueError(f"GPIO21 changed to {levels}, not {want}")
    latencies = set()
    for k, (at, _) in enumerate(changes, start=1):
        if at % PS_PER_CYCLE:
            raise ValueError(f"edge {k} at {at} ps is between two cycles")
        latencies.add(at // PS_PER_CYCLE - k * period)
    if latencies != {LATENCY}:
        raise ValueError(f"the edges sit {sorted(latencies)} cycles after their matches, not {LATENCY}")


def check(run: Path) -> None:
    text = expected_text(SOURCE.read_text())
    period, edges = closed_form(text)
    check_serial((run / "traces" / "serial.csv").read_text(), text)
    check_edges((run / "traces" / "logic.vcd").read_text(), period, edges)
    print(f"esp32s3-blink: {len(text)} console bytes as written, {edges} edges at k * {period} + {LATENCY}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except ValueError as err:
        print(f"esp32s3-blink: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
