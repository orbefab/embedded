# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Keyes KY-019 style 1-channel relay module: the board.

Part frame: the board's underside is at z = 2.54, where the leads under it hold
it off the table, and its top is at z = 4.14. The outline is 34 x 27 mm, the
size quoted for the KY-019, with a mounting hole in each corner. The header
stands on the -Y edge with its pins pointing up, and the screw terminal sits on
the +Y edge with its wire entries facing out. The header, the
terminal, the relay, the transistor, the resistors, the diode and the LED are
their own models.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box, cyl_z

BOARD_L = 34.0
BOARD_W = 27.0
PCB_T = 1.6
BOARD_Z = 2.54
# The header's edge. part.jsonc places the header beside it.
EDGE_Y = -11.5
BOARD_Y = EDGE_Y + BOARD_W / 2.0
HOLE = (3.0, 2.8)  # diameter, centre in from each edge  # verify

_MASK = plastic("#9B2335")


def _board():
    z = BOARD_Z + PCB_T / 2.0
    pcb = box(0, BOARD_Y, z, BOARD_L, BOARD_W, PCB_T)
    d, inset = HOLE
    for sx in (-1.0, 1.0):
        for sy in (-1.0, 1.0):
            x = sx * (BOARD_L / 2.0 - inset)
            y = BOARD_Y + sy * (BOARD_W / 2.0 - inset)
            pcb = pcb - cyl_z(x, y, BOARD_Z - 0.1, BOARD_Z + PCB_T + 0.1, d / 2.0)
    return [paint(pcb, "pcb", _MASK)]


def relay_module_1ch_shape():
    return bd.Compound(children=_board(), label="relay_module_1ch")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        relay_module_1ch_shape(),
        "relay-module-1ch",
        None,
        Path(sys.argv[1]),
    )
