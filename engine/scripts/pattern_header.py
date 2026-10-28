# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Write firmware/esp32s3/pattern.h from firmware/esp32s3/pattern.json.

    uv run scripts/pattern_header.py           write pattern.h
    uv run scripts/pattern_header.py --check   fail unless the committed pattern.h is a fresh write

pattern.json holds the panel's init commands, each with its parameters and the milliseconds the
program waits after it, then the rectangles, each with its inclusive corners and a fill rule.
pattern.c draws them from the header, and scripts/render_st77916.py renders the same file.
"""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PATTERN = ROOT / "firmware/esp32s3/pattern.json"
HEADER = ROOT / "firmware/esp32s3/pattern.h"
FILLS = ["bars", "checker", "gradient", "xor"]
WIDTH = HEIGHT = 360
MAX_COLORS = 8


def header(doc):
    out = [
        "/* Written by scripts/pattern_header.py from pattern.json. Do not edit. */",
        "",
        "#define FILL_BARS 0",
        "#define FILL_CHECKER 1",
        "#define FILL_GRADIENT 2",
        "#define FILL_XOR 3",
        "",
        "struct rect {",
        "    uint16_t x0, y0, x1, y1;",
        "    uint8_t fill, cell, blue, ncolors;",
        f"    uint16_t colors[{MAX_COLORS}];",
        "};",
        "",
        "/* Each init command as its command, its parameter count, its delay in ms, then its",
        "   parameters. */",
        "static const uint8_t init_commands[] = {",
    ]
    for command in doc["init"]:
        data = command["data"]
        assert 0 <= command["cmd"] <= 0xFF and len(data) <= 0xFF, command
        assert 0 <= command["delay_ms"] <= 0xFF and all(0 <= b <= 0xFF for b in data), command
        fields = [f"0x{command['cmd']:02x}", str(len(data)), str(command["delay_ms"])]
        fields += [f"0x{b:02x}" for b in data]
        out.append(f"    {', '.join(fields)},")
    out += ["};", f"#define INIT_COMMANDS {len(doc['init'])}", "", "static const struct rect rects[] = {"]
    for rect in doc["rects"]:
        x0, y0, x1, y1 = (rect[k] for k in ("x0", "y0", "x1", "y1"))
        assert 0 <= x0 <= x1 < WIDTH and 0 <= y0 <= y1 < HEIGHT, rect
        colors = rect.get("colors", [])
        assert len(colors) <= MAX_COLORS and all(0 <= c <= 0xFFFF for c in colors), rect
        palette = ", ".join(f"0x{c:04x}" for c in colors)
        fill = FILLS.index(rect["fill"])
        out.append(
            f"    {{{x0}, {y0}, {x1}, {y1}, {fill}, {rect.get('cell', 0)}, {rect.get('blue', 0)}, {len(colors)}, {{{palette}}}}},"
        )
    out += ["};", f"#define RECTS {len(doc['rects'])}"]
    return "\n".join(out) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = header(json.loads(PATTERN.read_text()))
    if args.check:
        if not HEADER.exists() or HEADER.read_text() != text:
            sys.exit(f"{HEADER.relative_to(ROOT)} is not what pattern_header.py writes from pattern.json")
        print(f"{HEADER.relative_to(ROOT)} is a fresh write of pattern.json")
        return
    HEADER.write_text(text)
    print(f"wrote {HEADER.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
