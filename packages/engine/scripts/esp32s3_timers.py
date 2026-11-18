# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-timers` against the timer periods, apart from the engine.

    uv run packages/engine/scripts/esp32s3_timers.py --check RUN_DIR

The app waits for `g`, then for TIMG0 T0 at dividers 2 and 80 prints eight one-shot lines and eight
auto-reload lines, `div <divider> shot <k> <cycles>` and `div <divider> reload <k> <cycles>`. The
cycles are from the `CCOUNT` sample taken immediately before `gptimer_start` to the alarm callback.
It then prints `equal <cycles>` and `behind <cycles>` for an alarm armed at the current count and
an alarm armed behind it, and `above <first> <gap>` for an auto-reload whose load is above the
alarm. It then prints `rng <k> nz` for eight reads of `WDEV_RND_REG`, and `TIMERS DONE`.
Lines after `TIMERS DONE` are ignored. A second `g`, which this circuit does not send, prints
`rng_on <8 hex digits>`, `rng_off <8 hex digits>` and `RNG OFF DONE`. The check does not compare
those words. The same lines before `TIMERS DONE` fail.

The alarm is 1000 counts. The app's sdkconfig runs the CPU at 240 MHz, so one APB cycle is 3 CPU
cycles, and the period is `1000 * divider * 3`. Each auto-reload step after the first must equal
that period. The first step may be up to 32 cycles short of it. At both dividers it is 17 short:
that first callback is entered from the instructions just after the start, and every later one from
`waiti`. Each one-shot, and the first auto-reload, must be longer than
the period by less than 5000 cycles, and the eight one-shots of a divider must be equal. `equal` and
`behind` must be above 0 and below 5000 cycles: each fires at the start, and a full turn of the
counter does not. `above`'s first latency follows that one-shot rule against divider 80's period,
and its gap is above 0 and below 5000. `traces/serial.csv` must hold those lines as `u1`'s
`USB_SERIAL_JTAG`. A `UART0`, `UART1` or `UART2` row may share the file: a flash boot programs
UART0, and the early log is sent there before the console moves. The run must hold no error, and
`manifest.json` must list the ROM, then each flash region with its offset.

When `board.txt` sits beside the netlist, it holds the same lines a board printed, including
`equal`, `behind` and `above`. Each one-shot and auto-reload latency must lie within 4000 CPU
cycles of the board's. 4000 cycles is 16.7 µs, room for several instruction-cache misses on the
80 MHz flash, and less than one extra alarm at divider 2, which is 6000 cycles, so a missed alarm
still fails. The three new lines are checked by the closed form above, not against the board's
numbers. The board's `rng` lines must say `nz`. The words are not printed. A board's register is
truly random, so the check does not compare them. Lines after `TIMERS DONE` in `board.txt` are
ignored too, including `rng_on` and `rng_off`. Without the file, that comparison is skipped.
"""

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-timers"
BOARD = EXAMPLE / "board.txt"

# The alarm count the app programs, and CPU cycles per APB cycle at 240 MHz.
ALARM_COUNTS = 1000
CPU_PER_APB = 3
DIVIDERS = (2, 80)
ALARMS = 8
# Below one extra alarm at divider 2 (6000 cycles), and above the driver's start path.
OVERHEAD_MAX = 5000
# The first reload callback is entered from the instructions after the start. That entry is 17
# cycles later than a callback entered from `waiti`, so the first step is 17 short of the period.
# Later steps match it. 32 is above that 17 and far below one alarm.
FIRST_STEP_SLACK = 32
# Several misses from the 80 MHz instruction cache, and still below one extra fast alarm.
BOARD_TOLERANCE = 4000
# An alarm armed at or behind the count, and the second hit of a reload above the alarm, are the
# interrupt entry. 5000 is the one-shot overhead bound, and it is under one divider-2 period.
IMMEDIATE_MAX = 5000

DIV = re.compile(r"div (\d+) (shot|reload) (\d+) (\d+)")
EQUAL = re.compile(r"equal (\d+)")
BEHIND = re.compile(r"behind (\d+)")
ABOVE = re.compile(r"above (\d+) (\d+)")
RNG = re.compile(r"rng (\d+) (nz|z)")
CLOCK = re.compile(r"rng_o(?:n|ff) [0-9a-fA-F]{8}")


def period(divider: int) -> int:
    """CPU cycles from one alarm to the next: the count times the divider times the CPU/APB ratio."""
    return ALARM_COUNTS * divider * CPU_PER_APB


def lines(serial_csv: str) -> str:
    """The console text, from `u1`'s `USB_SERIAL_JTAG` bytes.

    A flash boot programs UART0 and sends its early log there, so those rows may share the file.
    """
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    known = {"USB_SERIAL_JTAG", "UART0", "UART1", "UART2"}
    if any(row["part"] != "u1" or row["usart"] not in known for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG or a UART")
    rows = [row for row in rows if row["usart"] == "USB_SERIAL_JTAG"]
    return "".join(chr(int(row["byte"])) for row in rows)


def rows_of(text: str) -> tuple[list[tuple[int, str, int, int]], tuple[int, int, int, int]]:
    """The `div` lines, and `(equal, behind, above_first, above_gap)`, once the console's shape is right."""
    rows = []
    rngs = []
    done = False
    phase = "div"
    equal = behind = above = None
    for raw in text.splitlines():
        line = raw.rstrip("\r")
        if done:
            break
        if line == "RNG OFF DONE" or CLOCK.fullmatch(line):
            raise ValueError(f"{line} comes before TIMERS DONE")
        div = DIV.fullmatch(line)
        eq = EQUAL.fullmatch(line)
        beh = BEHIND.fullmatch(line)
        above_line = ABOVE.fullmatch(line)
        rng = RNG.fullmatch(line)
        if div:
            if phase != "div":
                raise ValueError(f"{line} follows the immediate lines")
            rows.append((int(div.group(1)), div.group(2), int(div.group(3)), int(div.group(4))))
        elif eq:
            if phase != "div" or equal is not None:
                raise ValueError(f"{line} is out of order")
            equal = int(eq.group(1))
            phase = "equal"
        elif beh:
            if phase != "equal":
                raise ValueError(f"{line} is out of order")
            behind = int(beh.group(1))
            phase = "behind"
        elif above_line:
            if phase != "behind":
                raise ValueError(f"{line} is out of order")
            above = (int(above_line.group(1)), int(above_line.group(2)))
            phase = "above"
        elif rng:
            if phase != "above" or done:
                raise ValueError(f"{line} is out of order")
            rngs.append((int(rng.group(1)), rng.group(2)))
        elif line == "TIMERS DONE":
            if phase != "above":
                raise ValueError("TIMERS DONE comes before the immediate lines")
            done = True
            phase = "done"
    if not done:
        raise ValueError("the console has no TIMERS DONE line")
    want = [(divider, kind, index) for divider in DIVIDERS for kind in ("shot", "reload") for index in range(ALARMS)]
    got = [(divider, kind, index) for divider, kind, index, _ in rows]
    if got != want:
        raise ValueError(f"the div lines are {got}, not {want}")
    if equal is None or behind is None or above is None:
        raise ValueError("the console has no equal, behind and above lines")
    if rngs != [(index, "nz") for index in range(ALARMS)]:
        raise ValueError(f"the rng lines are {rngs}")
    return rows, (equal, behind, above[0], above[1])


def check_periods(rows: list[tuple[int, str, int, int]]) -> None:
    """The model's closed form: exact auto-reload steps, and one-shots within one period."""
    for divider in DIVIDERS:
        span = period(divider)
        shots = [cycles for div, kind, _, cycles in rows if div == divider and kind == "shot"]
        reloads = [cycles for div, kind, _, cycles in rows if div == divider and kind == "reload"]
        if any(cycles <= span or cycles - span >= OVERHEAD_MAX for cycles in shots):
            raise ValueError(f"divider {divider} one-shots {shots} are not within {OVERHEAD_MAX} of {span}")
        if max(shots) != min(shots):
            raise ValueError(f"divider {divider} one-shots differ: {shots}")
        if reloads[0] <= span or reloads[0] - span >= OVERHEAD_MAX:
            raise ValueError(f"divider {divider} first reload {reloads[0]} is not within {OVERHEAD_MAX} of {span}")
        deltas = [reloads[index] - reloads[index - 1] for index in range(1, ALARMS)]
        if span - deltas[0] > FIRST_STEP_SLACK or deltas[0] > span:
            raise ValueError(
                f"divider {divider} first reload step {deltas[0]} is not within {FIRST_STEP_SLACK} of {span}"
            )
        if any(delta != span for delta in deltas[1:]):
            raise ValueError(f"divider {divider} reload steps {deltas[1:]} are not {span}")


def check_immediate(immediate: tuple[int, int, int, int]) -> None:
    """`equal` and `behind` fire at the start. `above` reaches its alarm, then fires again at once."""
    equal, behind, first, gap = immediate
    if not 0 < equal < IMMEDIATE_MAX:
        raise ValueError(f"equal {equal} is not within {IMMEDIATE_MAX} cycles")
    if not 0 < behind < IMMEDIATE_MAX:
        raise ValueError(f"behind {behind} is not within {IMMEDIATE_MAX} cycles")
    span = period(80)
    if first <= span or first - span >= OVERHEAD_MAX:
        raise ValueError(f"above first {first} is not within {OVERHEAD_MAX} of {span}")
    if not 0 < gap < IMMEDIATE_MAX:
        raise ValueError(f"above gap {gap} is not within {IMMEDIATE_MAX} cycles")


def check_console(text: str) -> list[tuple[int, str, int, int]]:
    """The div lines of a model console, checked against the closed form."""
    rows, immediate = rows_of(text)
    check_periods(rows)
    check_immediate(immediate)
    return rows


def compare(model: str, board: str, tolerance: int = BOARD_TOLERANCE) -> None:
    """Each model latency against the board's, within `tolerance` CPU cycles."""
    left, _ = rows_of(model)
    right, _ = rows_of(board)
    for (div, kind, index, cycles), (bdiv, bkind, bindex, bcycles) in zip(left, right, strict=True):
        if (div, kind, index) != (bdiv, bkind, bindex):
            raise ValueError(f"div {div} {kind} {index} has no board line")
        if abs(cycles - bcycles) > tolerance:
            raise ValueError(f"div {div} {kind} {index} is {cycles}, the board's is {bcycles}, tolerance {tolerance}")


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


def check(run: Path) -> None:
    netlist = json.loads((EXAMPLE / "netlist.json").read_text())
    text = lines((run / "traces" / "serial.csv").read_text())
    check_console(text)
    if BOARD.is_file():
        compare(text, BOARD.read_text())
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    print(
        "esp32s3-timers: two dividers, eight one-shots and eight reloads, "
        "equal, behind, above, eight non-zero RNG words"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-timers: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
