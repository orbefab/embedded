# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Decode a WS2812E-1313 chain from a run folder's logic.vcd, apart from the engine.

    uv run scripts/ws2812_decode.py RUN_DIR [--part NAME]
    uv run scripts/ws2812_decode.py --self-test

The rule is the Worldsemi WS2812E-1313 datasheet's, V1.1, page 3, in picoseconds. A high
of 220000 to 380000 (T0H) is a 0, and a high of 580000 to 1000000 (T1H) is a 1. A low of
more than 280000000 (RES) is a reset.

The script cuts `DIN` into runs of one level. Each high run is a bit when its width is in
T0H or T1H, and is dropped when it is in neither. A low run longer than RES ends the
frame, and the chain latches RES + 1 after that low began. A shorter low only parts two
bits, and the low from power-on ends no frame. In a frame, LED 0 takes the first 24 bits,
LED 1 the next 24, and so on, green then red then blue, most significant bit first. An LED
the frame did not fill keeps its colour.

It compares the latches with `traces/leds.csv` and the colours with that part's `leds`
entry in `observations.json`. `--part` limits the comparison to one part. `--self-test`
decodes made-up lines instead of a run folder.
"""

import argparse
import json
import sys
from pathlib import Path

T0H = range(220_000, 380_000 + 1)
T1H = range(580_000, 1_000_000 + 1)
RES = 280_000_000


def read_vcd(path):
    """`{(part, pin): [(t_ps, high)]}`, the initial level first."""
    tokens = Path(path).read_text().split()
    names = {}
    out = {}
    scope = None
    at = 0
    k = 0
    while k < len(tokens):
        token = tokens[k]
        if token == "$scope":
            scope = tokens[k + 2]
            k += 4
        elif token == "$upscope":
            k += 2
        elif token == "$var":
            names[tokens[k + 3]] = (scope, tokens[k + 4])
            k += 6
        elif token == "$timescale":
            if tokens[k + 1 : k + 3] != ["1", "ps"]:
                raise SystemExit("the timescale is 1 ps")
            k = tokens.index("$end", k) + 1
        elif token in ("$enddefinitions", "$dumpvars", "$end"):
            k += 1
        elif token.startswith("#"):
            at = int(token[1:])
            k += 1
        else:
            out.setdefault(names[token[1:]], []).append((at, token[0] == "1"))
            k += 1
    return out


def runs(samples, stop):
    """`[(high, start, end)]`, one per run of one level, the last ending at `stop`."""
    out = []
    for at, high in samples:
        if out and out[-1][0] == high:
            continue
        if out:
            out[-1][2] = at
        out.append([high, at, None])
    out[-1][2] = stop
    return out


def frames(runs):
    """`[(latch_ps, bits)]`, one per low run longer than RES after the first run."""
    out = []
    bits = []
    for k, (high, start, end) in enumerate(runs):
        width = end - start
        if high:
            if width in T0H:
                bits.append(0)
            elif width in T1H:
                bits.append(1)
        elif k > 0 and width > RES:
            out.append((start + RES + 1, bits))
            bits = []
    return out


def show(count, frames):
    """The colours after each latch, `[(latch_ps, pixels)]`."""
    leds = [[0, 0, 0] for _ in range(count)]
    rows = []
    for at, bits in frames:
        for led in range(min(count, len(bits) // 24)):
            word = bits[24 * led : 24 * led + 24]
            leds[led] = [int("".join(map(str, word[8 * c : 8 * c + 8])), 2) for c in range(3)]
        rows.append((at, [px[:] for px in leds]))
    return leds, rows


def hex_grb(pixels):
    return "".join(f"{g:02x}{r:02x}{b:02x}" for g, r, b in pixels)


def csv_rows(text):
    lines = text.splitlines()
    if not lines or lines[0] != "t_ps,part,count,grb":
        raise SystemExit("leds.csv header is t_ps,part,count,grb")
    rows = []
    for line in lines[1:]:
        t_ps, part, count, grb = line.split(",")
        rows.append((int(t_ps), part, int(count), grb))
    return rows


def self_test():
    """A power-on low longer than RES, then one LED of 0x80 0x00 0x00 and a reset."""
    start = 2 * RES
    samples = [(0, False)]
    at = start
    for k in range(24):
        high = 625_000 if k == 0 else 312_500
        samples += [(at, True), (at + high, False)]
        at += 1_250_000
    fall = samples[-1][0]
    stop = fall + 2 * RES
    want = [(fall + RES + 1, [1] + [0] * 23)]
    got = frames(runs(samples, stop))
    if got != want:
        raise SystemExit(f"self-test: frames {got}, want {want}")
    print("self-test: 1 frame after a power-on low longer than RES")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", nargs="?")
    parser.add_argument("--part")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if args.run is None:
        parser.error("RUN_DIR is required")
    run = Path(args.run)
    netlist = json.loads((run / "netlist.json").read_text())
    observations = json.loads((run / "observations.json").read_text())
    stop = netlist["analysis"]["transient"]["stop_ps"]
    parts = [
        part
        for part in netlist["parts"]
        if part["kind"] == "ws2812" and (args.part is None or part["name"] == args.part)
    ]
    if not parts:
        raise SystemExit("no ws2812 part")
    samples = read_vcd(run / "traces" / "logic.vcd")
    written = csv_rows((run / "traces" / "leds.csv").read_text())
    failed = False
    for part in parts:
        name = part["name"]
        count = part.get("leds", 13)
        leds, rows = show(count, frames(runs(samples[(name, "DIN")], stop)))
        got = observations["leds"][name]
        if got["pixels"] != leds or got["order"] != "grb" or got["count"] != count:
            print(f"{name}: observations {got['pixels']} decoded {leds}", file=sys.stderr)
            failed = True
        want = [(at, name, count, hex_grb(pixels)) for at, pixels in rows]
        have = [row for row in written if row[1] == name]
        if have != want:
            print(f"{name}: leds.csv {have} decoded {want}", file=sys.stderr)
            failed = True
        print(f"{name}: {count} leds, {len(rows)} latches")
    if failed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
