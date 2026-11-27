# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""The common blue 16x2 parallel character LCD module (1602A): the bare board.

Part frame: the header plastic's underside is z = 0, so the board's
underside is at z = PLASTIC and its top at PLASTIC + PCB_T. The outline is
Vishay LCD-016N002B, Document Number 37295, the dimensions drawing on p. 2:
80.0 x 36.0, centred on the origin, the header edge at +Y, four 2.5 mounting
holes on 75.0 x 31.0, and 16 header holes of 1.0 on 2.54, the first 8.0 in
from the left edge and 2.5 down from the top. The panel, the header, the
backlight and the part on the back are their own models.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box, cyl_z
from cad.header import PLASTIC

BOARD = (80.0, 36.0)
PCB_T = 1.6
BOARD_Z = PLASTIC
HOLE_D = 2.5
HOLES = tuple((sx * 75.0 / 2.0, sy * 31.0 / 2.0) for sx in (-1, 1) for sy in (-1, 1))
PIN_HOLE_D = 1.0
HEADER_X0 = -BOARD[0] / 2.0 + 8.0
HEADER_Y = BOARD[1] / 2.0 - 2.5
HEADER_PITCH = 2.54

_PCB = plastic("#1F6E3C")


def _board():
    z0, thick = BOARD_Z, PCB_T
    pcb = box(0, 0, z0 + thick / 2.0, BOARD[0], BOARD[1], thick)
    for x, y in HOLES:
        pcb -= cyl_z(x, y, z0 - 0.4, z0 + thick + 0.4, HOLE_D / 2.0)
    for k in range(16):
        pcb -= cyl_z(HEADER_X0 + k * HEADER_PITCH, HEADER_Y, z0 - 0.4, z0 + thick + 0.4, PIN_HOLE_D / 2.0)
    return paint(pcb, "pcb", _PCB)


def lcd1602_shape():
    return bd.Compound(children=[_board()], label="lcd1602")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(lcd1602_shape(), "lcd1602", None, Path(sys.argv[1]))
