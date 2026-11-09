# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Write packages/engine/tests/circuits/st77916-replay: its replay list and the netlist that replays it.

    uv run packages/engine/scripts/st77916_example.py           write events.json and netlist.json
    uv run packages/engine/scripts/st77916_example.py --check   exit 1 unless both are what it would write

The replay list, `events.json`, is a JSON array of `spi_replay` events. `netlist.json`'s
`spi_replay` part carries the same array as its `events` field.

- rectangle A, one pixel write whose data comes in two transactions, the first with CS held;
- rectangle B, one transaction;
- rectangle C, written with MADCTL's MX bit set, then MADCTL 0x00 again;
- rectangle D, one pixel write whose data comes in three transactions.

`packages/engine/scripts/render_st77916.py` reads `events.json` and writes the rows and the image the run
must show.
"""

import argparse
import json
import sys
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "st77916-replay"
CLOCK_HZ = 80_000_000
GAP_PS = 2_000_000
STOP_PS = 2_000_000_000
STEP_PS = 50_000_000


def startup():
    return [
        (0x01, []),
        (0x11, []),
        (0x3A, [0x55]),
        (0x36, [0x00]),
        window(0x2A, 0, 359),
        window(0x2B, 0, 359),
        (0x29, []),
        (0xF0, [0xA1]),
        (0xF1, [0xA2, 0xA3]),
        (0xE9, [0xA4]),
        (0xD4, [0xA5]),
    ]


def rgb565(r, g, b):
    """Two bytes, high first, of 5-bit red, 6-bit green and 5-bit blue."""
    value = (r & 0x1F) << 11 | (g & 0x3F) << 5 | (b & 0x1F)
    return [value >> 8, value & 0xFF]


def gradient(w, h):
    return [byte for y in range(h) for x in range(w) for byte in rgb565(x * 31 // (w - 1), y * 63 // (h - 1), 8)]


def checker(w, h):
    def tile(x, y):
        return (x // 8 + y // 8) % 2

    return [
        byte for y in range(h) for x in range(w) for byte in (rgb565(31, 63, 0) if tile(x, y) else rgb565(0, 16, 31))
    ]


def stripes(w, h):
    return [byte for y in range(h) for x in range(w) for byte in rgb565((x + y) % 32, 63 - (x + y) % 64, 31 - y % 32)]


def bars(w, h):
    return [byte for y in range(h) for x in range(w) for byte in rgb565(y % 32, x * 2, 31 - x)]


def window(command, start, end):
    return (command, [start >> 8, start & 0xFF, end >> 8, end & 0xFF])


def varying(n):
    return [byte for i in range(n) for byte in rgb565((i * 3) % 32, (i * 5) % 64, (i * 7) % 32)]


class Start(NamedTuple):
    command: int
    size: int
    hold: bool


class Continue(NamedTuple):
    size: int
    hold: bool


def pattern():
    wrap = varying(2 * 3 + 1)
    follow = varying(4 * 2)
    return [
        (
            [window(0x2A, 20, 99), window(0x2B, 24, 68)],
            gradient(80, 45),
            [Start(0x2C, 3600, True), Continue(3600, False)],
        ),
        ([window(0x2A, 200, 239), window(0x2B, 40, 79)], checker(40, 40), [Start(0x2C, 3200, False)]),
        ([(0x36, [0x40]), window(0x2A, 60, 123), window(0x2B, 150, 181)], stripes(64, 32), [Start(0x2C, 4096, False)]),
        (
            [(0x36, [0x00]), window(0x2A, 300, 329), window(0x2B, 200, 289)],
            bars(30, 90),
            [Start(0x2C, 1800, True), Continue(1800, True), Continue(1800, False)],
        ),
        ([window(0x2A, 0, 1), window(0x2B, 0, 2)], wrap, [Start(0x2C, len(wrap), False)]),
        ([window(0x2A, 350, 353), window(0x2B, 350, 351)], follow, [Start(0x2C, 8, False), Start(0x3C, 8, False)]),
    ]


def cycles(event):
    def over(bits, lanes):
        return -(-bits // lanes)

    return sum(over(p["bits"], p["lanes"]) for p in event["phases"]) + over(8 * len(event["bytes"]), event["lanes"])


def events():
    out = []
    at = GAP_PS

    def send(phases, lanes, data, cs):
        nonlocal at
        event = {"t_ps": at, "phases": phases, "lanes": lanes, "bytes": data, "cs": cs}
        out.append(event)
        at += cycles(event) * 10**12 // CLOCK_HZ + GAP_PS

    def word(opcode, command):
        return [{"bits": 8, "lanes": 1, "value": opcode}, {"bits": 24, "lanes": 1, "value": command << 8}]

    def register(command, data):
        send(word(0x02, command), 1, data, "released")

    for command, data in startup():
        register(command, data)
    for writes, pixels, chunks in pattern():
        for command, data in writes:
            register(command, data)
        assert sum(chunk.size for chunk in chunks) == len(pixels)
        first = 0
        for chunk in chunks:
            if isinstance(chunk, Continue):
                phases = []
            else:
                phases = word(0x32, chunk.command)
            send(phases, 4, pixels[first : first + chunk.size], "held" if chunk.hold else "released")
            first += chunk.size
    assert at < STOP_PS
    return out


def events_text(items):
    return "[\n" + ",\n".join("  " + json.dumps(event, separators=(", ", ": ")) for event in items) + "\n]\n"


def netlist_text(items):
    def stage(volts, ohms):
        return {
            "high_drop": {"value": volts, "unit": "V", "source": "user"},
            "high_resistance": {"value": ohms, "unit": "ohm", "source": "user"},
            "low_drop": {"value": volts, "unit": "V", "source": "user"},
            "low_resistance": {"value": ohms, "unit": "ohm", "source": "user"},
        }

    data = {"D0": "d0", "D1": "d1", "D2": "d2", "D3": "d3"}
    netlist = {
        "version": 0,
        "seed": 0,
        "analysis": {"transient": {"stop_ps": STOP_PS, "max_step_ps": STEP_PS, "grid_ps": STEP_PS}},
        "ground": "gnd",
        "nets": ["gnd", "cs", "d0", "d1", "d2", "d3", "rst", "sclk", "te", "v3v3"],
        "parts": [
            {
                "name": "lcd1",
                "kind": "st77916",
                "pins": {
                    "VCC": "v3v3",
                    "GND": "gnd",
                    "CS": "cs",
                    "SCLK": "sclk",
                    **data,
                    "RST": "rst",
                    "TE": "te",
                    "BLK": "v3v3",
                },
                "output": stage(0.0, 25.0),
                "input_threshold": {
                    "low": {"value": 0.3, "unit": "1", "source": "user"},
                    "high": {"value": 0.7, "unit": "1", "source": "user"},
                },
            },
            {
                "name": "r1",
                "kind": "resistor",
                "pins": {"a": "v3v3", "b": "rst"},
                "resistance": {"value": 10000.0, "unit": "ohm", "source": "user"},
                "ratings": {"power": None},
            },
            {
                "name": "rp1",
                "kind": "spi_replay",
                "pins": {"VCC": "v3v3", "GND": "gnd", "CS": "cs", "SCLK": "sclk", **data},
                "clock_hz": {"value": CLOCK_HZ, "unit": "Hz", "source": "user"},
                "output": stage(0.0, 25.0),
                "events": "EVENTS",
            },
            {
                "name": "v1",
                "kind": "voltage_source",
                "pins": {"p": "v3v3", "n": "gnd"},
                "voltage": {"value": 3.3, "unit": "V", "source": "user"},
            },
        ],
    }
    text = json.dumps(netlist, indent=2)
    listed = events_text(items).rstrip("\n").replace("\n", "\n      ")
    return text.replace('"EVENTS"', listed) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="exit 1 unless the files are current")
    args = parser.parse_args()
    items = events()
    files = {EXAMPLE / "events.json": events_text(items), EXAMPLE / "netlist.json": netlist_text(items)}
    if args.check:
        stale = [path for path, text in files.items() if not path.is_file() or path.read_text() != text]
        if stale:
            print("stale: " + ", ".join(str(path.relative_to(ROOT)) for path in stale))
            sys.exit(1)
        listed = json.loads((EXAMPLE / "netlist.json").read_text())
        replay = next(part for part in listed["parts"] if part["kind"] == "spi_replay")
        assert replay["events"] == json.loads((EXAMPLE / "events.json").read_text())
        print(f"ok: {len(items)} events in events.json and netlist.json")
        return
    EXAMPLE.mkdir(exist_ok=True)
    for path, text in files.items():
        path.write_text(text)
    print(f"wrote {len(items)} events")


if __name__ == "__main__":
    main()
