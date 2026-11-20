# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""The Nano clone's bare board: the PCB, four mounting holes and the LED legend.

Part frame: the Nano's seated frame. z = 0 is the header plastic's
underside, so the board's underside is at z = NANO_BOARD_Z and its top at
NANO_BOARD_Z + NANO_PCB_T. The PCB is centred on the origin, USB toward -X,
component side +Z. Every part on it is its own model, placed by the board's
layout. The board has no leads of its own.

Sizes from the Arduino Nano A000005 datasheet, page 11 (2020/11/19): the
board 18 × 43.2 × 1.6, the header rows 15.24 apart. The mini-USB shell
makes the outline 45 mm overall; that shell is the usb-mini-b part.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box, cyl_z
from cad.header import PLASTIC
from cad.mark import text

NANO_PCB_W = 18.0  # A000005, page 11
NANO_PCB_L = 43.2  # A000005, page 11
NANO_PCB_T = 1.6  # A000005, page 11
NANO_BOARD_Z = PLASTIC
NANO_HOLE_D = 1.52  # verify
NANO_HOLE_INSET = 1.29  # verify
NANO_SILK_T = 0.06
NANO_LEGEND_X = 12.9
NANO_LEGEND_SIZE = 0.9
NANO_LEGEND = (("L", 3.3), ("PWR", 1.1), ("TX", -1.1), ("RX", -3.3))

_BLUE = plastic("#164E86")
_WHITE = plastic("#F4F1E8")


def _board():
    z0, thick = NANO_BOARD_Z, NANO_PCB_T
    pcb = box(0, 0, z0 + thick / 2.0, NANO_PCB_L, NANO_PCB_W, thick)
    hx = NANO_PCB_L / 2.0 - NANO_HOLE_INSET
    hy = NANO_PCB_W / 2.0 - NANO_HOLE_INSET
    for sx in (-1.0, 1.0):
        for sy in (-1.0, 1.0):
            pcb -= cyl_z(sx * hx, sy * hy, z0 - 0.4, z0 + thick + 0.4, NANO_HOLE_D / 2.0)
    return paint(pcb, "pcb", _BLUE)


def _silk(word, y):
    # Rotation 90 runs the word across the board, tops of the letters toward the USB.
    top = NANO_BOARD_Z + NANO_PCB_T
    mark = text(
        NANO_LEGEND_X,
        y,
        top - 0.01,
        word,
        NANO_LEGEND_SIZE,
        rotation=90,
        thickness=NANO_SILK_T,
    )
    return paint(mark, "silk-" + word.lower(), _WHITE)


def nano_board_shape():
    children = [_board()]
    children += [_silk(word, y) for word, y in NANO_LEGEND]
    return bd.Compound(children=children, label="nano_board")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(nano_board_shape(), "nano-board", None, Path(sys.argv[1]))
