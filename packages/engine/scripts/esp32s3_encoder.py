# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-encoder` against its contacts, apart from the engine.

    uv run packages/engine/scripts/esp32s3_encoder.py --check RUN_DIR

The app of `packages/engine/tests/firmware/esp32s3/idf/encoder` counts the closings of the knob's two contacts,
`sw_a` on GPIO2 and `sw_b` on GPIO1, from a falling-edge interrupt on each pin, and prints
`edge a=<n> b=<m> t=<us>` for each, with the `esp_timer` time the interrupt took. A 3 ms
`esp_timer` polls both pins and prints `poll a=<n> b=<m>` for each press it counts.

`traces/serial.csv` must hold `encoder ready` before the first closing, then one `edge` line for
each closing in the netlist's events, in time order, with the counts after that closing. The
SYSTIMER counts from the run's start, so `t` is the run's time in microseconds, and it must lie
from the closing's microsecond to `LATENCY_US` after it. The `poll` lines must hold the same counts
in the same order, each printed from `POLL_US` to two periods and `PRINT_SLACK_US` after its closing,
and the last counts must be each contact's number of closings. The run must
hold no error, and its `manifest.json` must list the netlist's ROM, then each flash region with
its offset.
"""

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-encoder"
EDGE = re.compile(r"edge a=(\d+) b=(\d+) t=(\d+)\r\n")
POLL = re.compile(r"poll a=(\d+) b=(\d+)\r\n")
READY = "encoder ready\r\n"
CONTACTS = ("sw_a", "sw_b")
US_PS = 10**6
# The interrupt's entry, IDF's dispatch and `esp_timer_get_time` take a few hundred cycles at
# 240 MHz, and the timer reads whole microseconds.
LATENCY_US = 10
POLL_US = 3000
# The timer counts a press at its second low read, one to two periods after the closing. Its
# dispatch, the queue and the print add a fraction of a millisecond, so a third of a period is
# room enough, and a poll a period late still fails.
PRINT_SLACK_US = 1000


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


def closings(netlist: dict) -> list[tuple[int, int]]:
    """Each closing of either contact as its time and the contact's index, in time order."""
    parts = {part["name"]: part for part in netlist["parts"]}
    out = []
    for k, name in enumerate(CONTACTS):
        part = parts[name]
        if part["kind"] != "switch" or part["initial"] != "open":
            raise ValueError(f"{name} is not a switch that starts open")
        out += [(event["t_ps"], k) for event in part["events"] if event["state"] == "closed"]
    return sorted(out)


def check_console(got: list[tuple[int, str]], closed: list[tuple[int, int]]) -> tuple[int, int]:
    """The edge and poll lines against the closings. Returns the last counts."""
    ready = [at for at, line in got if line == READY]
    if len(ready) != 1 or (closed and ready[0] >= closed[0][0]):
        raise ValueError("the console does not print one `encoder ready` before the first closing")
    edges = [EDGE.fullmatch(line) for _, line in got]
    edges = [tuple(int(g) for g in match.groups()) for match in edges if match]
    polls = [(at, POLL.fullmatch(line)) for at, line in got]
    polls = [(at, tuple(int(g) for g in match.groups())) for at, match in polls if match]
    if len(edges) != len(closed):
        raise ValueError(f"{len(edges)} edge lines for {len(closed)} closings")
    counts = [0, 0]
    for (at, k), (a, b, t) in zip(closed, edges, strict=True):
        counts[k] += 1
        if (a, b) != tuple(counts):
            raise ValueError(f"the edge at {at} ps counts a={a} b={b}, not a={counts[0]} b={counts[1]}")
        start = at // US_PS
        if not start <= t <= start + LATENCY_US:
            raise ValueError(f"the edge at {at} ps took the time {t} us, not {start} to {start + LATENCY_US} us")
    if [counts for _, counts in polls] != [(a, b) for a, b, _ in edges]:
        raise ValueError(f"the polled presses {polls} are not the edges' counts")
    for (closing, _), (at, _) in zip(closed, polls, strict=True):
        start, end = closing + POLL_US * US_PS, closing + (2 * POLL_US + PRINT_SLACK_US) * US_PS
        if not start <= at <= end:
            raise ValueError(f"the poll after the closing at {closing} ps printed at {at} ps, not {start} to {end} ps")
    return counts[0], counts[1]


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
    closed = closings(netlist)
    got = lines((run / "traces" / "serial.csv").read_text())
    a, b = check_console(got, closed)
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    print(
        f"esp32s3-encoder: {a} closings of sw_a and {b} of sw_b, each an edge within {LATENCY_US} us and a polled press"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError) as error:
        print(f"esp32s3-encoder: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
