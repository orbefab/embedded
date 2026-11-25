# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Bosch Sensortec BMP280 pressure and temperature sensor, 8-pad metal-lid LGA.

Part frame: on the board's top at z = 0, upright (metal lid toward +Z), centred
on the origin, 2.50 mm length along X and 2.00 mm width along Y, matching the
top view in BST-BMP280-DS001-26 Figure 18. Seen from the top, leads count
clockwise (Figure 14 and Figure 18): 1 to 4 along the +Y row from -X to +X,
5 to 8 along the -Y row from +X to -X.

From BST-BMP280-DS001-26 section 7.1, Figure 18 (nominals, mm):
- outline 2.50 x 2.00, height 0.95, substrate 0.13
- metal lid outer brim 2.25 x 1.75, flat top 1.55 x 1.05
- vent hole diameter 0.30, centred 0.825 from the -X and +Y edges
- eight 0.35 x 0.35 pads, 0.65 pitch along X, 1.45 row spacing across Y

Pad thickness (0.03), lid brim thickness (0.08), lid wall inset (0.10 from the
brim) and vent bore depth (0.20) are not dimensioned in Figure 18.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import gold, nickel, plastic
from cad.geom import box, cyl_z, fuse

BODY_X = 2.50  # BST-BMP280-DS001-26 Figure 18
BODY_Y = 2.00  # Figure 18
HEIGHT = 0.95  # Figure 18
SUBSTRATE_T = 0.13  # Figure 18
LID_BRIM_X = 2.25  # Figure 18
LID_BRIM_Y = 1.75  # Figure 18
LID_TOP_X = 1.55  # Figure 18
LID_TOP_Y = 1.05  # Figure 18
VENT_D = 0.30  # Figure 18
VENT_FROM_EDGE = 0.825  # Figure 18, from the -X and +Y edges
PAD_SIZE = 0.35  # Figure 18, bottom view
PAD_PITCH = 0.65  # Figure 18, bottom view
PAD_ROW = 1.45  # Figure 18, bottom view

# Not dimensioned in Figure 18.
PAD_T = 0.03
BRIM_T = 0.08
LID_WALL_X = 2.05
LID_WALL_Y = 1.55
LID_BEVEL_H = 0.20
VENT_DEPTH = 0.20

_SUBSTRATE = plastic("#232528")
_LID = nickel("#C9CCD0")
_CAVITY = plastic("#121212")
_PAD = gold()


def _pads():
    """(lead, x, y) clockwise from the top view, matching Figure 14 and Figure 18."""
    y_row = PAD_ROW / 2.0
    out = []
    for k in range(4):
        out.append((k + 1, (k - 1.5) * PAD_PITCH, y_row))
    for k in range(4):
        out.append((5 + k, (1.5 - k) * PAD_PITCH, -y_row))
    return out


def _metal_lid():
    brim_top = SUBSTRATE_T + BRIM_T
    bevel_base = HEIGHT - LID_BEVEL_H
    brim = box(0, 0, SUBSTRATE_T + BRIM_T / 2.0, LID_BRIM_X, LID_BRIM_Y, BRIM_T)
    wall = box(0, 0, (brim_top + bevel_base) / 2.0, LID_WALL_X, LID_WALL_Y, bevel_base - brim_top)
    crown = bd.loft(
        [
            bd.Plane.XY.offset(bevel_base) * bd.Rectangle(LID_WALL_X, LID_WALL_Y),
            bd.Plane.XY.offset(HEIGHT) * bd.Rectangle(LID_TOP_X, LID_TOP_Y),
        ]
    )
    vent_x = -BODY_X / 2.0 + VENT_FROM_EDGE
    vent_y = BODY_Y / 2.0 - VENT_FROM_EDGE
    vent_cut = cyl_z(vent_x, vent_y, HEIGHT - VENT_DEPTH, HEIGHT + 0.05, VENT_D / 2.0)
    lid = fuse([brim, wall, crown]) - vent_cut
    cavity = cyl_z(vent_x, vent_y, HEIGHT - VENT_DEPTH - 0.02, HEIGHT - VENT_DEPTH + 0.02, VENT_D / 2.0 - 0.01)
    return paint(lid, "lid", _LID), paint(cavity, "vent", _CAVITY)


def _parts():
    sub_h = SUBSTRATE_T - PAD_T
    substrate = paint(box(0, 0, PAD_T + sub_h / 2.0, BODY_X, BODY_Y, sub_h), "body", _SUBSTRATE)
    lid, cavity = _metal_lid()
    children = [substrate, lid, cavity]
    anchors = []
    for lead, x, y in _pads():
        children.append(paint(box(x, y, PAD_T / 2.0, PAD_SIZE, PAD_SIZE, PAD_T), f"lead-{lead}", _PAD))
        anchors.append(glb.Anchor(lead, f"lead-{lead}", (x, y, 0.0), kind="pad"))
    return children, anchors


def bmp280_shape():
    return bd.Compound(children=_parts()[0], label="bmp280")


def bmp280_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(bmp280_shape(), "bmp280", bmp280_anchors(), Path(sys.argv[1]))
