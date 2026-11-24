# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""The common blue 16x2 character LCD with a PCF8574 I2C backpack: the backpack board,
SOIC-16 PCF8574 IC, backlight jumper block, A0-A2 solder pads, and riser header.

Part frame: the 4-pin header plastic's underside is z = 0, centred on X, with
the 4-pin header at y = -16.5 mm. The backpack PCB sits at z = 5.5 mm (top at
z = 7.1 mm) on a 3 mm spacer above the header plastic, so the trimmer pot's
through-hole leads stay above z = 0 and the 1602A panel mounts at z = 12.5 mm.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import copper, plastic, tin
from cad.geom import box
from cad.header import PLASTIC

BACKPACK_W = 41.5
BACKPACK_H = 19.0
PCB_T = 1.6
BACKPACK_Z = 5.5
BACKPACK_Y = -8.5
HEADER_Y = -16.5

_BLACK_PCB = plastic("#16181C")
_IC = plastic("#1F2126")
_PLASTIC = plastic("#1A1A1A")
_TIN = tin()
_COPPER = copper()


def _backpack():
    z0 = BACKPACK_Z
    z_top = z0 + PCB_T
    children = [
        # Black backpack PCB.
        paint(box(0, BACKPACK_Y, z0 + PCB_T / 2.0, BACKPACK_W, BACKPACK_H, PCB_T), "backpack", _BLACK_PCB),
        # Spacer block around the 4-pin header tails between z = PLASTIC and BACKPACK_Z.
        paint(box(0, HEADER_Y, (PLASTIC + z0) / 2.0, 4 * 2.54, 2.54, z0 - PLASTIC), "header-spacer", _PLASTIC),
        # PCF8574T SOIC-16 package on the backpack.
        paint(box(-2.0, BACKPACK_Y + 1.5, z_top + 1.15, 10.3, 7.5, 2.3), "pcf8574", _IC),
        # 2-pin backlight jumper block + shunt on the left end.
        paint(box(-16.5, BACKPACK_Y, z_top + 3.0, 2.6, 5.2, 6.0), "backlight-jumper", _PLASTIC),
        # A0, A1, A2 address solder jumper pads.
        paint(box(-12.0, BACKPACK_Y - 5.0, z_top + 0.05, 5.5, 1.8, 0.1), "addr-pads", _COPPER),
        # 16-pin black riser strip linking the backpack up to the 1602A panel at z = 12.5.
        paint(box(0, 0.0, (z_top + 12.5) / 2.0, 40.64, 2.54, 12.5 - z_top), "riser", _PLASTIC),
    ]
    # SOIC-16 gull-wing lead rows (visual detail).
    for k in range(8):
        lx = -2.0 + (k - 3.5) * 1.27
        for sy in (-1, 1):
            children.append(paint(box(lx, BACKPACK_Y + 1.5 + sy * 4.5, z_top + 0.25, 0.45, 1.5, 0.5), f"ic-pin-{k}-{sy}", _TIN))
    return bd.Compound(children=children, label="lcd1602_i2c")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(_backpack(), "lcd1602-i2c", None, Path(sys.argv[1]))
