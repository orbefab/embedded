# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Bourns CAY16-J4 resistor array, four 0603 resistors in one 1206 chip: RN1 to RN4 on the Uno.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Eight convex terminations, four a side on 0.8, count
counter-clockwise from the top: 1 to 4 along -Y from -X, 5 to 8 back along
+Y (`cad.ic.dual_row`). Resistor k joins leads k and 9 - k.

From Bourns' CAT16/CAY16 datasheet, CAY16-J4: 3.20 ± 0.20 long, 1.60 ± 0.20
wide, 0.50 ± 0.10 tall, terminations on 0.80, 0.30 deep.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic, paint
from cad.color import srgb
from cad.geom import box

CAY_BODY = (3.2, 1.6, 0.5)  # long, wide, tall
CAY_PITCH = 0.8
CAY_TERMINAL = (0.4, 0.3)  # wide, in from the side

_BODY = srgb("#1A1A1A")
_TIN = srgb("#C5C8CE")


def _terminal_xy(x, side):
    return x, side * (CAY_BODY[1] / 2.0 - CAY_TERMINAL[1] / 2.0)


def resistor_array_cay16_shape():
    length, width, height = CAY_BODY
    wide, deep = CAY_TERMINAL
    terminals = {
        n: box(*_terminal_xy(x, side), height / 2.0, wide, deep, height)
        for n, x, side in ic.dual_row(8, CAY_PITCH)
    }
    body = box(0, 0, height / 2.0, length, width, height).cut(*terminals.values())
    children = [paint(body, "body", _BODY)]
    children += [paint(t, f"lead-{n}", _TIN) for n, t in terminals.items()]
    return bd.Compound(children=children, label="resistor_array_cay16")


def resistor_array_cay16_anchors():
    return [
        glb.Anchor(n, f"lead-{n}", (*_terminal_xy(x, side), 0.0), kind="pad")
        for n, x, side in ic.dual_row(8, CAY_PITCH)
    ]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        resistor_array_cay16_shape(),
        "resistor-array-4x0603",
        resistor_array_cay16_anchors(),
        Path(sys.argv[1]),
        merge_faces=False,
    )
