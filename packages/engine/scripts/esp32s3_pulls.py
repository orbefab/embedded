# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-pulls` against the pad's pulls, apart from the engine.

    uv run packages/engine/scripts/esp32s3_pulls.py --check RUN_DIR

GPIO44 sits on a net with only its pad capacitance. After `g` the app prints
`static up|down|both|none <level>`, the level `GPIO_IN1` reads 1 ms after each pull setting, in
that order. Each must be the level the run's own netlist gives: the pad settles on the divider of
its pulls and its leakage, it reads 1 above `input_threshold.high` of the supply and 0 at or below
`input_threshold.low`, and between them it keeps the level it read before. The first setting
follows a pad that only leaks, which reads 0 from power-on in the band.

It then prints `poll <polls> <cycles>`, the cycles of that many polls, and 16 lines
`rep <k> base <cycles> fall <cycles> rise <cycles>`, then `PULLS DONE`. Rep `k` waits `k / 16`
of a poll after each release before its first poll. `base` releases a pad that already reads its
level, `fall` releases a driven high under the pull-down and polls until it reads 0, and `rise`
a driven low under the pull-up until it reads 1, each from the release to the poll that reads.

The four static levels must match the netlist, and the board's when `board.txt` is present.
With `F` and `R` the mean of fall and of rise less their rep's base, the ratio is the RC law's:
each edge starts at the level its drive and pulls hold, heads for the level its pull and the
leakage hold, and crosses its threshold, so the capacitance and the supply cancel. With equal
pulls and thresholds even about half the supply it is the pull-down's resistance over the
pull-up's. The ratio is compared only when both edges resolve it within `TOLERANCE`, 10 %. The
16 reps are one sweep of the phase, not 16 independent readings of one crossing, so an edge that
finishes inside the first poll does so at every phase and the mean does not recover it. `S` is
the largest base less the smallest and `q` the cycles of one poll. Each mean is uncertain by
`U = max(S, q)` cycles, and the ratio's relative uncertainty is `U / F + U / R`. Both edges must
be longer than their base, and that uncertainty at most 10 %, which needs each edge to last at
least `2 U / 0.10` cycles: 20 poll periods when the bases stay inside one poll, and more when
they spread further. A resolved ratio must lie within 10 % of the netlist's. Otherwise the check
prints that the ratio is unresolved at this pad's capacitance and does not count it as passed.

When `board.txt` sits beside the netlist, it holds a board's console from `pulls ready` through
`PULLS DONE`. Its static levels must match the run's. Its ratio is compared only when both
resolve, and must then lie within `TOLERANCE` plus both uncertainties, relatively, of the run's.
With no `board.txt` that comparison is skipped, and the check says so.

`traces/serial.csv` must hold those lines as `u1`'s `USB_SERIAL_JTAG`. Other lines may sit
between them. The run must hold no error, and its `manifest.json` must list the netlist's ROM,
then each flash region with its offset.
"""

import argparse
import csv
import io
import json
import math
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-pulls"
PIN = "GPIO44"
REPS = 16
SETTINGS = ("up", "down", "both", "none")
# A board pad switches near half the supply on both edges, and the ratio moves fast when it does
# not: a pad that switched at 0.48 and 0.52 of the supply would read 12 % over the pulls' ratio.
# The same figure is how close a resolved ratio must be.
TOLERANCE = 0.10
BOARD_TOLERANCE = TOLERANCE
STATIC = re.compile(r"static (up|down|both|none) ([01])")
POLL = re.compile(r"poll (\d+) (\d+)")
REP = re.compile(r"rep (\d+) base (\d+) fall (\d+) rise (\d+)")


class Reading:
    """One probe's edges. `fall` and `rise` are mean cycles past that rep's base, and may be
    negative when the edge never cleared it. `spread` is the largest base less the smallest."""

    def __init__(self, fall: float, rise: float, spread: int, per_poll: float) -> None:
        self.fall = fall
        self.rise = rise
        self.spread = spread
        self.per_poll = per_poll

    def uncertainty(self) -> float | None:
        """Cycles each mean is uncertain by, or `None` when an edge is not a time.

        The reps are one phase sweep, so the spread is not divided by how many there are.
        """
        if self.fall <= 0 or self.rise <= 0:
            return None
        return float(max(self.spread, self.per_poll))

    def bound(self) -> float | None:
        """The ratio's relative uncertainty, `U / F + U / R`."""
        uncertainty = self.uncertainty()
        if uncertainty is None:
            return None
        return uncertainty / self.fall + uncertainty / self.rise

    def resolved(self) -> bool:
        """Both edges are long enough that the ratio is good to `TOLERANCE`."""
        bound = self.bound()
        return bound is not None and bound <= TOLERANCE

    def ratio(self) -> float | None:
        if self.fall <= 0 or self.rise <= 0:
            return None
        return self.fall / self.rise

    def polls_needed(self) -> float:
        """Poll periods each edge must last. `U` is `max(S, q)` even when an edge is not a time,
        and 10 % on both edges takes `2 U / TOLERANCE` cycles."""
        uncertainty = max(self.spread, self.per_poll)
        return (2 * uncertainty / TOLERANCE) / self.per_poll


def lines(serial_csv: str) -> list[str]:
    """Each console line, without its line ending."""
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    if any(row["part"] != "u1" or row["usart"] != "USB_SERIAL_JTAG" for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG")
    return "".join(chr(int(row["byte"])) for row in rows).splitlines()


class Probe:
    """What the app printed: the four static levels, the poll's cycles, and each rep's base,
    fall and rise."""

    def __init__(self, text: list[str]) -> None:
        self.static: dict[str, int] = {}
        self.poll: tuple[int, int] | None = None
        self.reps: list[tuple[int, int, int]] = []
        done = False
        for line in text:
            line = line.rstrip("\r")
            if done:
                continue
            if line == "PULLS DONE":
                done = True
            elif match := STATIC.fullmatch(line):
                self.static[match.group(1)] = int(match.group(2))
            elif match := POLL.fullmatch(line):
                self.poll = (int(match.group(1)), int(match.group(2)))
            elif match := REP.fullmatch(line):
                if int(match.group(1)) != len(self.reps):
                    raise ValueError(f"rep {match.group(1)} is out of order")
                self.reps.append((int(match.group(2)), int(match.group(3)), int(match.group(4))))
        if not done:
            raise ValueError("the console has no PULLS DONE line")
        if list(self.static) != list(SETTINGS):
            raise ValueError(f"the console reads the pulls {list(self.static)}, not {list(SETTINGS)}")
        if self.poll is None or self.poll[0] == 0:
            raise ValueError("the console has no poll line")
        if len(self.reps) != REPS:
            raise ValueError(f"the console holds {len(self.reps)} reps, not {REPS}")
        if any(0 in rep for rep in self.reps):
            raise ValueError("a rep never read its level")

    def per_poll(self) -> float:
        polls, cycles = self.poll
        return cycles / polls

    def reading(self) -> Reading:
        """The mean edges, the base spread and the cycles of one poll."""
        fall = sum(fall - base for base, fall, _ in self.reps) / len(self.reps)
        rise = sum(rise - base for base, _, rise in self.reps) / len(self.reps)
        bases = [base for base, _, _ in self.reps]
        return Reading(fall, rise, max(bases) - min(bases), self.per_poll())


class Pad:
    """The pad's stages, pulls, leakage and thresholds, from the `esp32s3` part, in siemens and
    fractions of the supply."""

    def __init__(self, netlist: dict) -> None:
        part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
        if PIN not in part["pins"]:
            raise ValueError(f"the part has no net on {PIN}")
        ohm = lambda field: float(field["value"])  # noqa: E731
        self.high = 1 / ohm(part["output"]["high_resistance"])
        self.low = 1 / ohm(part["output"]["low_resistance"])
        self.up = 1 / ohm(part["pull_up_resistance"])
        if "pull_down_resistance" not in part:
            raise ValueError("the part has no pull_down_resistance")
        self.down = 1 / ohm(part["pull_down_resistance"])
        self.leak = 1 / ohm(part["leakage_resistance"])
        self.vil = float(part["input_threshold"]["low"]["value"])
        self.vih = float(part["input_threshold"]["high"]["value"])
        if part["output"]["high_drop"]["value"] or part["output"]["low_drop"]["value"]:
            raise ValueError("the check takes stages with no drop")

    def settles(self, to_vcc: float, to_gnd: float) -> float:
        """The level, as a fraction of the supply, that conductances to each rail hold."""
        return to_vcc / (to_vcc + to_gnd)

    def level(self, setting: str, before: int) -> int:
        up = self.up if setting in ("up", "both") else 0.0
        down = self.down if setting in ("down", "both") else 0.0
        v = self.settles(self.leak + up, self.leak + down)
        if v > self.vih:
            return 1
        if v <= self.vil:
            return 0
        return before

    def levels(self) -> dict[str, int]:
        """Each setting's level, in order, from the power-on 0."""
        out, before = {}, 0
        for setting in SETTINGS:
            before = self.level(setting, before)
            out[setting] = before
        return out

    def ratio(self) -> float:
        """The fall's time over the rise's, from the RC law with each edge's own conductance."""
        start = self.settles(self.high + self.leak, self.leak + self.down)
        end = self.settles(self.leak, self.leak + self.down)
        fall = math.log((start - end) / (self.vil - end)) / (2 * self.leak + self.down)
        start = self.settles(self.leak + self.up, self.low + self.leak)
        end = self.settles(self.leak + self.up, self.leak)
        rise = math.log((end - start) / (end - self.vih)) / (2 * self.leak + self.up)
        return fall / rise


def check_static(probe: Probe, pad: Pad) -> None:
    want = pad.levels()
    if probe.static != want:
        raise ValueError(f"the pulls read {probe.static}, not {want}")


def check_ratio(probe: Probe, pad: Pad) -> str:
    """The ratio clause. An unresolved ratio is reported and not failed."""
    reading = probe.reading()
    if not reading.resolved():
        fall, rise = reading.fall / reading.per_poll, reading.rise / reading.per_poll
        return (
            "ratio unresolved at this pad's capacitance "
            f"(fall {fall:.1f} polls, rise {rise:.1f} polls, "
            f"base spread {reading.spread} cycles, need {reading.polls_needed():.1f} polls)"
        )
    ratio = reading.ratio()
    want = pad.ratio()
    assert ratio is not None
    if abs(ratio - want) > TOLERANCE * want:
        raise ValueError(f"the run's fall over rise is {ratio:.4f}, not {want:.4f} within {TOLERANCE:.0%}")
    return f"fall over rise {ratio:.4f} against {want:.4f} within {TOLERANCE:.0%}"


def check_board(probe: Probe, board: Probe) -> str:
    """The board clause. Static levels always, the ratio only when both probes resolve it."""
    if probe.static != board.static:
        raise ValueError(f"the pulls read {probe.static}, the board reads {board.static}")
    run, theirs = probe.reading(), board.reading()
    if not run.resolved() or not theirs.resolved():
        return "the board's static levels match, ratio not compared"
    run_bound, their_bound = run.bound(), theirs.bound()
    ratio, other = run.ratio(), theirs.ratio()
    assert run_bound is not None and their_bound is not None and ratio is not None and other is not None
    allowed = TOLERANCE + run_bound + their_bound
    if abs(ratio - other) > allowed * other:
        raise ValueError(f"the run's fall over rise is {ratio:.4f}, the board's {other:.4f}, not within {allowed:.1%}")
    return f"the board's {other:.4f}"


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


def check(run: Path, example: Path = EXAMPLE) -> str:
    netlist = json.loads((run / "netlist.json").read_text())
    probe = Probe(lines((run / "traces" / "serial.csv").read_text()))
    pad = Pad(netlist)
    check_static(probe, pad)
    clause = check_ratio(probe, pad)
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    board_txt = example / "board.txt"
    if board_txt.exists():
        board = ", " + check_board(probe, Probe(board_txt.read_text().splitlines()))
    else:
        board = ", no board.txt, so no board comparison"
    report = f"esp32s3-pulls: static {probe.static}, {clause}{board}"
    print(report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-pulls: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
