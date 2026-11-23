# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Keyes KY-019 style 1-channel relay module: the board and the screw terminal.

Part frame: the header plastic's underside is z = 0, so the board's underside
is at z = 2.54 and its top at z = 4.14. The outline is 34 x 27 mm, the size
quoted for the KY-019. The header, the relay, the transistor, the resistors,
the diode and the LED are their own models. The screw terminal is this body.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box

BOARD_L = 34.0
BOARD_W = 27.0
PCB_T = 1.6
BOARD_Z = 2.54
# The header pins sit on this edge. part.jsonc places the header at the same y.
EDGE_Y = -11.5
BOARD_Y = EDGE_Y + BOARD_W / 2.0

_MASK = plastic("#9B2335")
_TERM = plastic("#1A1A1A")


def _board():
    z = BOARD_Z + PCB_T / 2.0
    pcb = box(0, BOARD_Y, z, BOARD_L, BOARD_W, PCB_T)
    terminal = box(0, 11, BOARD_Z + PCB_T + 4.0, 16, 8, 8)
    return [paint(pcb, "pcb", _MASK), paint(terminal, "terminal", _TERM)]


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
