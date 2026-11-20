# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""SPC SK68XXMINI-HS addressable RGB LED, one pixel.

Part frame: on its pads, centred on the origin, the lens up +Z. The 3.7 mm
side runs along X and the 3.50 mm side along Y. From the sheet's section 6,
top view, pin 3 (VDD) is at -X +Y, pin 2 (DIN) at +X +Y, pin 4 (DOUT) at
-X -Y and pin 1 (GND) at +X -Y.

From SPC / SK68XX MINI-HS Rev. 04, page 3. The body is 3.7 x 3.50 and the
overall height is 1.95, which includes the pads. The pad thickness is not
on the sheet. Pads are 0.9 along X and 0.8 along Y. The outer pad edges are
3.68 by 2.58 and the inner gaps are 1.88 and 0.98, so the centres are at
±1.39 and ±0.89. The lens diameter is 2.85.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box, cyl_z

BODY = (3.7, 3.50, 1.80)  # above the pads, so the overall height is 1.95
PAD_T = 0.15  # verify, the sheet gives no pad thickness
PAD = (0.9, 0.8)  # along the 3.7 side, along the 3.50 side
LENS_R = 1.425  # diameter 2.85
# lead, x, y. Section 6, top view.
PADS = (
    (1, 1.39, -0.89),  # GND
    (2, 1.39, 0.89),  # DIN
    (3, -1.39, 0.89),  # VDD
    (4, -1.39, -0.89),  # DOUT
)

_BLACK = plastic("#1A1A1A")
_LENS = plastic("#E7E4DC")
_TIN = tin()


def sk68xxmini_hs_shape():
    bx, by, bh = BODY
    px, py = PAD
    top = PAD_T + bh
    # The lens sits in a pocket cut from the body, so the two tops never share a face.
    lens = cyl_z(0, 0, top - 0.45, top, LENS_R)
    children = [
        paint(box(0, 0, PAD_T + bh / 2.0, bx, by, bh) - lens, "body", _BLACK),
        paint(cyl_z(0, 0, top - 0.45, top, LENS_R), "lens", _LENS),
    ]
    for lead, x, y in PADS:
        children.append(paint(box(x, y, PAD_T / 2.0, px, py, PAD_T), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="sk68xxmini_hs")


def sk68xxmini_hs_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x, y, 0.0), kind="pad") for lead, x, y in PADS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        sk68xxmini_hs_shape(),
        "sk68xxmini-hs",
        sk68xxmini_hs_anchors(),
        Path(sys.argv[1]),
    )
