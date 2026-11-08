# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Microchip ATmega328P-AU, TQFP-32, 7 × 7 mm body: the Nano's microcontroller.

Part frame: on the board's top at z = 0, centred on the origin, edges
along X and Y. Leads are numbered counter-clockwise from the top: 1 to 8
down the -X edge from +Y, 9 to 16 along -Y, 17 to 24 up +X, 25 to 32 back
along +Y. The dot marks the lead-1 corner.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

TQFP_BODY = 7.0
TQFP_SPAN = 9.0  # tip to tip
TQFP_H = 1.0
TQFP_STANDOFF = 0.15
TQFP_PITCH = 0.8
TQFP_LEAD = (1.3, 0.22, 0.16)  # out from the body, wide, thick
TQFP_LEAD_Z = 0.1  # lead centre

_BLACK = srgb("#1A1A1A")
_TIN = srgb("#C5C8CE")


def _lead_xy(lead):
    side, k = divmod(lead - 1, 8)
    out = TQFP_SPAN / 2.0 - TQFP_LEAD[0] / 2.0
    along = (k - 3.5) * TQFP_PITCH
    return ((-out, -along), (along, -out), (out, along), (-along, out))[side], side % 2 == 1


def atmega328p_tqfp32_shape():
    children = [
        paint(box(0, 0, TQFP_STANDOFF + TQFP_H / 2.0, TQFP_BODY, TQFP_BODY, TQFP_H), "body", _BLACK),
        paint(box(-2.45, 2.45, TQFP_STANDOFF + TQFP_H + 0.05, 0.55, 0.55, 0.12), "dot", _TIN),
    ]
    out, wide, thick = TQFP_LEAD
    for lead in range(1, 33):
        (x, y), across_y = _lead_xy(lead)
        sx, sy = (wide, out) if across_y else (out, wide)
        children.append(paint(box(x, y, TQFP_LEAD_Z, sx, sy, thick), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="atmega328p_tqfp32")


def atmega328p_tqfp32_anchors():
    bottom = TQFP_LEAD_Z - TQFP_LEAD[2] / 2.0
    return [
        glb.Anchor(lead, f"lead-{lead}", (*_lead_xy(lead)[0], bottom), kind="pad")
        for lead in range(1, 33)
    ]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        atmega328p_tqfp32_shape(),
        "atmega328p-tqfp32",
        atmega328p_tqfp32_anchors(),
        Path(sys.argv[1]),
    )
