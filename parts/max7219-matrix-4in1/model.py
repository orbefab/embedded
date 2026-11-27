# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""4-in-1 MAX7219 32 x 8 LED dot matrix module (FC-16): the bare PCB.

Part frame: z = 0 is the 1x5 header plastic's underside, so the board's
underside is at z = PLASTIC (2.54 mm) and its top at PLASTIC + PCB_T (4.14 mm).
138.0 x 32.0 mm green PCB carrying four contiguous 32 x 32 mm 8 x 8 matrices
(x = -48, -16, +16, +48 mm) and 1x5 IN/OUT headers at x = +/- 66.04 mm.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box, cyl_z
from cad.header import PLASTIC

BOARD_W = 138.0
BOARD_H = 32.0
PCB_T = 1.6
BOARD_Z = PLASTIC
HOLE_D = 3.0
HOLES = ((-66.0, -13.0), (-66.0, 13.0), (66.0, -13.0), (66.0, 13.0))

_GREEN = plastic("#1E6B3A")


def _board():
    z0, thick = BOARD_Z, PCB_T
    pcb = box(0, 0, z0 + thick / 2.0, BOARD_W, BOARD_H, thick)
    for x, y in HOLES:
        pcb -= cyl_z(x, y, z0 - 0.4, z0 + thick + 0.4, HOLE_D / 2.0)
    return paint(pcb, "pcb", _GREEN)


def max7219_matrix_4in1_shape():
    return bd.Compound(children=[_board()], label="max7219_matrix_4in1")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(max7219_matrix_4in1_shape(), "max7219-matrix-4in1", None, Path(sys.argv[1]))
