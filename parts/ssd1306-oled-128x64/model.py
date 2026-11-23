# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""A 0.96 inch 128 x 64 OLED panel with its SSD1306 on the glass, UG-2864HSWEG01 class.

Part frame: the panel's underside is z = 0, centred on the origin, the long
side along X. +Y is the edge away from the FPC, which is the top of the
picture on the module. The FPC tail leaves the -Y edge, flat on its pads at z = 0.

From Univision UG-2864HSWEG01, SAS1-9046-B, section 1.2 and the section 1.4
mechanical drawing. The panel is 26.70 x 19.26 x 1.45. The active area is
21.744 x 10.864, 2.478 from each side edge and 2.1 from the edge opposite the
FPC. The FPC has 30 pins on 0.70. The tail's length and the pad size here
are not on the drawing; the real tail folds behind the module's board.

`glass` is the active area, its own node, so a viewer can draw the screen on
it. Its top is the panel's top, in a pocket cut from the body.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import copper, plastic
from cad.geom import box

PANEL = (26.70, 19.26, 1.45)
ACTIVE = (21.744, 10.864)
ACTIVE_FROM_TOP = 2.1  # the edge opposite the FPC
GLASS_T = 0.05  # verify, the pocket that keeps the two tops apart
FPC_PITCH = 0.70
FPC_TAIL = 3.0  # verify
FPC_T = 0.1  # verify
PAD = (0.4, 1.2)  # verify
PAD_T = 0.03  # verify, under the tail, so each pad's underside is its anchor
# Every FPC pin is a lead, numbered as section 1.5 numbers it.
LEADS = tuple(range(1, 31))

_BODY = plastic("#2A2D33", roughness=0.15)
_GLASS = plastic("#050607", roughness=0.05)
_FPC = plastic("#C9822B")
_COPPER = copper()


def _glass_centre_y():
    return PANEL[1] / 2.0 - ACTIVE_FROM_TOP - ACTIVE[1] / 2.0


def _pad_xy(lead):
    return ((lead - 15.5) * FPC_PITCH, -PANEL[1] / 2.0 - FPC_TAIL + PAD[1] / 2.0)


def ssd1306_oled_128x64_shape():
    px, py, pz = PANEL
    ax, ay = ACTIVE
    gy = _glass_centre_y()
    pocket = box(0, gy, pz - GLASS_T / 2.0, ax, ay, GLASS_T)
    tail_w = 30 * FPC_PITCH + 0.6
    children = [
        paint(box(0, 0, pz / 2.0, px, py, pz) - pocket, "panel", _BODY),
        paint(box(0, gy, pz - GLASS_T / 2.0, ax, ay, GLASS_T), "glass", _GLASS),
        paint(box(0, -py / 2.0 - FPC_TAIL / 2.0, PAD_T + FPC_T / 2.0, tail_w, FPC_TAIL, FPC_T), "fpc", _FPC),
    ]
    for lead in LEADS:
        x, y = _pad_xy(lead)
        children.append(paint(box(x, y, PAD_T / 2.0, PAD[0], PAD[1], PAD_T), f"lead-{lead}", _COPPER))
    return bd.Compound(children=children, label="ssd1306_oled_128x64")


def ssd1306_oled_128x64_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (*_pad_xy(lead), 0.0), kind="pad") for lead in LEADS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        ssd1306_oled_128x64_shape(),
        "ssd1306-oled-128x64",
        ssd1306_oled_128x64_anchors(),
        Path(sys.argv[1]),
    )
