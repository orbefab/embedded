# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""SparkFun BOB-12009 style BSS138 level shifter: the bare board.

Part frame: the header plastic's underside is z = 0, so the board's
underside is at z = PLASTIC and its top at PLASTIC + PCB_T. The outline
is the Eagle board, 15.24 x 12.7 mm, centred on the origin, length along
X. The transistors, the resistors and the two headers are their own models.

The product page lists 16.05 x 13.33 mm overall. That includes the
headers past the Eagle outline.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box
from cad.header import PLASTIC

BOARD_L = 15.24  # Eagle dimension, Logic_Level_Bidirectional.brd
BOARD_W = 12.7
PCB_T = 1.6
BOARD_Z = PLASTIC

_MASK = plastic("#123C73")


def _board():
    z0, thick = BOARD_Z, PCB_T
    pcb = box(0, 0, z0 + thick / 2.0, BOARD_L, BOARD_W, thick)
    return paint(pcb, "pcb", _MASK)


def bss138_level_shifter_shape():
    return bd.Compound(children=[_board()], label="bss138_level_shifter")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        bss138_level_shifter_shape(),
        "bss138-level-shifter",
        None,
        Path(sys.argv[1]),
    )
