# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Adafruit NeoPixel Stick, product 1426: the bare board.

Part frame: the header plastic's underside is z = 0, so the board's
underside is at z = PLASTIC and its top at PLASTIC + PCB_T. The outline
is the published Eagle board, 50.80 x 10.16 mm, centred on the origin,
length along X. The pixels and the 1x3 header are their own models.

The product page gives 51.10 x 10.22 x 3.19 mm overall. That height is
the board and its LEDs, with no through-hole header. The Eagle outline
is the board itself. Two 2.0 mm holes sit on the drawing, at Eagle
(12.700, 8.128) and (38.100, 8.128) with the origin at a corner.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box, cyl_z
from cad.header import PLASTIC

BOARD_L = 50.800  # Eagle outline, Adafruit NeoPixel 8 Stick v2
BOARD_W = 10.160
PCB_T = 1.6
BOARD_Z = PLASTIC
# Eagle coordinates minus the board centre (25.4, 5.08).
HOLES = ((-12.7, 3.048), (12.7, 3.048))
HOLE_D = 2.0

_BLACK = plastic("#141414")


def _board():
    z0, thick = BOARD_Z, PCB_T
    pcb = box(0, 0, z0 + thick / 2.0, BOARD_L, BOARD_W, thick)
    for x, y in HOLES:
        pcb -= cyl_z(x, y, z0 - 0.4, z0 + thick + 0.4, HOLE_D / 2.0)
    return paint(pcb, "pcb", _BLACK)


def neopixel_stick_8_shape():
    return bd.Compound(children=[_board()], label="neopixel_stick_8")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(neopixel_stick_8_shape(), "neopixel-stick-8", None, Path(sys.argv[1]))
