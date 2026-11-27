# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Worldsemi WS2812B-V5 addressable RGB LED in a 5050 package, one pixel.

Part frame: on its pads at z = 0, centred on the origin, lens up +Z.
From Worldsemi WS2812B-V5 V1.0 p2 Mechanical Dimensions and PIN Configuration
(top view): body 5.0 x 5.0 x 1.57 mm, lead-end span 5.4 mm along X, pad size
0.9 x 0.9 mm, lens recess dia 4.00 mm (inner emitter cup 3.35 mm).
Pin 1 (VDD) is at -X +Y, Pin 2 (DOUT) at -X -Y, Pin 3 (VSS, chamfered corner)
at +X -Y, Pin 4 (DIN) at +X +Y.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box, cyl_z

BODY = (5.0, 5.0, 1.42)  # above pads (1.57 total with 0.15 pad thickness)
PAD_T = 0.15
PAD = (0.9, 0.9)
LENS_R = 1.85  # 3.7 mm glowing emitter cup inside 4.0 mm recess
# (lead, x, y) per p2 Back View: outer X = 5.4 -> center +-2.25; outer Y = 4.2 -> center +-1.65.
PADS = (
    (1, -2.25, 1.65),  # VDD
    (2, -2.25, -1.65),  # DOUT
    (3, 2.25, -1.65),  # VSS (chamfered corner)
    (4, 2.25, 1.65),  # DIN
)

_WHITE = plastic("#ECEAE4")
_LENS = plastic("#D8D4C8")
_TIN = tin()


def ws2812b_5050_shape():
    bx, by, bh = BODY
    px, py = PAD
    top = PAD_T + bh
    lens = cyl_z(0, 0, top - 0.45, top, LENS_R)
    body = box(0, 0, PAD_T + bh / 2.0, bx, by, bh) - lens
    # Chamfer Pin 3 (+X, -Y) corner per p2 PIN Configuration.
    chamfer = bd.Pos(bx / 2.0, -by / 2.0, top) * bd.Rot(0, 0, 45) * box(0, 0, 0, 1.0, 1.0, 0.9)
    body -= chamfer
    children = [
        paint(body, "body", _WHITE),
        paint(cyl_z(0, 0, top - 0.45, top, LENS_R), "lens", _LENS),
    ]
    for lead, x, y in PADS:
        children.append(paint(box(x, y, PAD_T / 2.0, px, py, PAD_T), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="ws2812b_5050")


def ws2812b_5050_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x, y, 0.0), kind="pad") for lead, x, y in PADS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(ws2812b_5050_shape(), "ws2812b-5050", ws2812b_5050_anchors(), Path(sys.argv[1]))
