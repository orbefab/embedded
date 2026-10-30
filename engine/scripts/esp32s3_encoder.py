# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `examples/esp32s3-encoder` against its contacts, apart from the engine.

    uv run scripts/esp32s3_encoder.py --check RUN_DIR
    uv run scripts/esp32s3_encoder.py --self-test

The app of `firmware/esp32s3/idf/encoder` counts the closings of the knob's two contacts, `sw_a` on
GPIO2 and `sw_b` on GPIO1, from a falling-edge interrupt on each pin, and prints
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

ROOT = Path(__file__).resolve().parent.parent
EXAMPLE = ROOT / "examples" / "esp32s3-encoder"
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
    for (at, k), (a, b, t) in zip(closed, edges):
        counts[k] += 1
        if (a, b) != tuple(counts):
            raise ValueError(f"the edge at {at} ps counts a={a} b={b}, not a={counts[0]} b={counts[1]}")
        start = at // US_PS
        if not start <= t <= start + LATENCY_US:
            raise ValueError(f"the edge at {at} ps took the time {t} us, not {start} to {start + LATENCY_US} us")
    if [counts for _, counts in polls] != [(a, b) for a, b, _ in edges]:
        raise ValueError(f"the polled presses {polls} are not the edges' counts")
    for (closing, _), (at, _) in zip(closed, polls):
        start, end = closing + POLL_US * US_PS, closing + (2 * POLL_US + PRINT_SLACK_US) * US_PS
        if not start <= at <= end:
            raise ValueError(f"the poll after the closing at {closing} ps printed at {at} ps, not {start} to {end} ps")
    return counts[0], counts[1]


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [{"role": "flash", "sha256": r["sha256"], "bytes": r["bytes"], "offset": r["offset"]}
             for r in part["firmware"]["flash"]]
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
    print(f"esp32s3-encoder: {a} closings of sw_a and {b} of sw_b, each an edge within {LATENCY_US} us "
          f"and a polled press")


def serial_of(timed: list[tuple[int, str]]) -> str:
    rows = "".join(f"{at},u1,USB_SERIAL_JTAG,{ord(c)}\n" for at, line in timed for c in line)
    return "t_ps,part,usart,byte\n" + rows


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

    def switch(name: str, closed: list[int]) -> dict:
        events = [e for t in closed for e in ({"t_ps": t, "state": "closed"}, {"t_ps": t + 9 * US_PS, "state": "open"})]
        return {"name": name, "kind": "switch", "initial": "open", "events": events}

    ms = 1000 * US_PS
    netlist = {"parts": [switch("sw_a", [200 * ms, 400 * ms]), switch("sw_b", [300 * ms])]}
    closed = closings(netlist)
    timed = [
        (5, "I (57) cpu_start: Unicore app\r\n"),
        (110 * ms, READY),
        (200 * ms + 30 * US_PS, "edge a=1 b=0 t=200001\r\n"),
        (204 * ms, "poll a=1 b=0\r\n"),
        (300 * ms + 30 * US_PS, "edge a=1 b=1 t=300001\r\n"),
        (304 * ms, "poll a=1 b=1\r\n"),
        (400 * ms + 30 * US_PS, "edge a=2 b=1 t=400001\r\n"),
        (404 * ms, "poll a=2 b=1\r\n"),
    ]

    def console(change=None) -> list[tuple[int, str]]:
        rows = [(at, change(line) if change else line) for at, line in timed]
        return lines(serial_of([row for row in rows if row[1]]))

    expect("the closings come in time order", closed == [(200 * ms, 0), (300 * ms, 1), (400 * ms, 0)])
    expect("a good run passes", not raises(lambda: check_console(console(), closed)))
    expect("a good run counts each contact", check_console(console(), closed) == (2, 1))
    expect("a late edge fails", raises(lambda: check_console(console(lambda l: l.replace("t=300001", "t=300011")), closed)))
    expect("an early edge fails", raises(lambda: check_console(console(lambda l: l.replace("t=300001", "t=299999")), closed)))
    expect("a miscounted edge fails", raises(lambda: check_console(console(lambda l: l.replace("edge a=1 b=1", "edge a=2 b=0")), closed)))
    expect("a lost edge fails", raises(lambda: check_console(console(lambda l: "" if "t=400001" in l else l), closed)))
    expect("a lost poll fails", raises(lambda: check_console(console(lambda l: "" if l == "poll a=1 b=1\r\n" else l), closed)))
    late = [(at + 4 * ms if line == "poll a=1 b=1\r\n" else at, line) for at, line in timed]
    expect("a late poll fails", raises(lambda: check_console(lines(serial_of(late)), closed)))
    expect("a miscounted poll fails", raises(lambda: check_console(console(lambda l: l.replace("poll a=2", "poll a=3")), closed)))
    expect("a console without ready fails", raises(lambda: check_console(console(lambda l: "" if l == READY else l), closed)))
    extra = netlist["parts"] + [switch("sw_c", [500 * ms])]
    expect("another switch is not a contact", closings({"parts": extra}) == closed)
    expect("a third closing of sw_a fails", raises(lambda: check_console(
        console(), closings({"parts": [switch("sw_a", [200 * ms, 400 * ms, 600 * ms]), switch("sw_b", [300 * ms])]}))))
    expect("an error fails", raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])))
    expect("a warning passes", not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])))
    other = serial_of(timed).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    part = {"kind": "esp32s3", "rom": {"sha256": "r", "bytes": 1},
            "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]}}
    listed = {"blobs": [{"role": "rom", "sha256": "r", "bytes": 1},
                        {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8}]}
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    unplaced = {"blobs": [listed["blobs"][0], {**listed["blobs"][1], "offset": None}]}
    expect("a region without its offset fails", raises(lambda: check_manifest(unplaced, {"parts": [part]})))
    committed = closings(json.loads((EXAMPLE / "netlist.json").read_text()))
    expect("the committed contacts close seven times", len(committed) == 7)
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
        print(f"esp32s3-encoder: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
