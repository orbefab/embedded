# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""The common 0.96 inch SSD1306 OLED module: the bare board.

Part frame: the header plastic's underside is z = 0, so the board's
underside is at z = PLASTIC and its top at PLASTIC + PCB_T. The outline is
the LCDWiki MC096GX size drawing, 27.30 x 27.80 mm, centred on the origin,
the header edge at +Y. The panel, the header and the parts on the back are
their own models.

The drawing dimensions one 2 mm hole, 2.00 mm in from the right and bottom
edges. The other three sit the same way in their corners.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box, cyl_z
from cad.header import PLASTIC

BOARD_W = 27.30  # LCDWiki MC096GX size drawing
BOARD_H = 27.80
PCB_T = 1.6
BOARD_Z = PLASTIC
HOLE_D = 2.0
HOLE_IN = 2.0
HOLES = tuple((sx * (BOARD_W / 2.0 - HOLE_IN), sy * (BOARD_H / 2.0 - HOLE_IN)) for sx in (-1, 1) for sy in (-1, 1))

_BLUE = plastic("#1D4E9E")


def _board():
    z0, thick = BOARD_Z, PCB_T
    pcb = box(0, 0, z0 + thick / 2.0, BOARD_W, BOARD_H, thick)
    for x, y in HOLES:
        pcb -= cyl_z(x, y, z0 - 0.4, z0 + thick + 0.4, HOLE_D / 2.0)
    return paint(pcb, "pcb", _BLUE)


def oled_ssd1306_128x64_shape():
    return bd.Compound(children=[_board()], label="oled_ssd1306_128x64")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(oled_ssd1306_128x64_shape(), "oled-ssd1306-128x64", None, Path(sys.argv[1]))
