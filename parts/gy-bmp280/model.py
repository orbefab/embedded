# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""GY-BMP280-3.3 pressure sensor breakout: the bare board.

Part frame: the header plastic's underside is z = 0, so the board's
underside is at z = PLASTIC and its top at PLASTIC + PCB_T. The outline
is 15.0 x 11.5 mm, centred on the origin, 6-pin header along the -Y edge
(pin 1 VCC at -X to pin 6 SDO at +X), matching the photos at
startingelectronics.org/pinout/GY-BMP280-pressure-sensor-module/ and
domoticx.net/docs/bmp280-information/. The BMP280, resistors, capacitors
and header are their own models.

The domoticx.net notes list the module size as 11.5 x 15 mm with 2.54 mm
pin pitch (its size photo labels the board 15.5 x 12 mm). PCB thickness
(1.6 mm), mounting-hole diameter (3.0 mm) and hole centres are not
dimensioned by either source and follow the module photos.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import gold, plastic
from cad.geom import box, cyl_z, fuse
from cad.header import PLASTIC

BOARD_W = 15.0  # domoticx.net/docs/bmp280-information/ ("Module size: 11.5mm*15mm")
BOARD_H = 11.5  # domoticx.net/docs/bmp280-information/
PCB_T = 1.6  # standard FR-4; not stated in the module notes
BOARD_Z = PLASTIC
# Two M3-sized mounting holes at the +Y corners; diameter and centres are not
# dimensioned in the module sources and follow the module photos.
HOLE_D = 3.0
RING_OD = 3.8
RING_T = 0.04
HOLES = ((-4.9, 3.1), (4.9, 3.1))

_PURPLE = plastic("#5A1E61")
_GOLD = gold()


def _board():
    z0, thick = BOARD_Z, PCB_T
    pcb = box(0, 0, z0 + thick / 2.0, BOARD_W, BOARD_H, thick)
    rings = []
    for x, y in HOLES:
        pcb -= cyl_z(x, y, z0 - 0.4, z0 + thick + 0.4, HOLE_D / 2.0)
        ring = cyl_z(x, y, z0 + thick, z0 + thick + RING_T, RING_OD / 2.0)
        ring -= cyl_z(x, y, z0 + thick - 0.1, z0 + thick + RING_T + 0.1, HOLE_D / 2.0)
        rings.append(ring)
    return [paint(pcb, "pcb", _PURPLE), paint(fuse(rings), "mounting-rings", _GOLD)]


def gy_bmp280_shape():
    return bd.Compound(children=_board(), label="gy_bmp280")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(gy_bmp280_shape(), "gy-bmp280", None, Path(sys.argv[1]))
