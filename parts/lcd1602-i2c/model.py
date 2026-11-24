# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""The common blue 16x2 character LCD with an FC-113 style PCF8574 I2C backpack:
the backpack board, SOIC-16 PCF8574 IC, backlight jumper block, A0-A2 solder
pads, and 16-pin riser header.

Part frame: z = 0 is the bench plane (underside of the backpack components),
centred on the 1602A panel in X and Y with the glass facing +Z and text top
toward +Y. The backpack PCB sits at z = 5.0 mm (top at z = 6.6 mm) under the
16-pin row along the top edge, and its 4-pin right-angle header sticks out
parallel to the PCB along -X for female jumper wires. The 1602A panel mounts
above the backpack at z = 10.5 mm.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import copper, plastic, tin
from cad.geom import box

BACKPACK_W = 42.0
BACKPACK_H = 19.0
PCB_T = 1.6
BACKPACK_X = -13.5
BACKPACK_Y = 7.5
BACKPACK_Z = 5.0
PANEL_Z = 10.5

_BLACK_PCB = plastic("#16181C")
_IC = plastic("#1F2126")
_PLASTIC = plastic("#1A1A1A")
_TIN = tin()
_COPPER = copper()


def _backpack():
    z0 = BACKPACK_Z
    z_top = z0 + PCB_T
    children = [
        # Black FC-113 backpack PCB under the 1602A top edge.
        paint(box(BACKPACK_X, BACKPACK_Y, z0 + PCB_T / 2.0, BACKPACK_W, BACKPACK_H, PCB_T), "backpack", _BLACK_PCB),
        # PCF8574T SOIC-16 package on the underside of the backpack.
        paint(box(-12.5, 6.5, z0 - 1.15, 10.3, 7.5, 2.3), "pcf8574", _IC),
        # 2-pin backlight jumper block + shunt on the right end, resting on z = 0.
        paint(box(5.0, 6.5, z0 / 2.0, 2.6, 5.2, z0), "backlight-jumper", _PLASTIC),
        # A0, A1, A2 address solder jumper pads on the underside.
        paint(box(-22.5, 1.0, z0 - 0.05, 5.5, 1.8, 0.1), "addr-pads", _COPPER),
        # 16-pin black riser strip linking the backpack up to the 1602A panel.
        paint(box(-12.95, 15.5, (z_top + PANEL_Z) / 2.0, 40.64, 2.54, PANEL_Z - z_top), "riser", _PLASTIC),
    ]
    # SOIC-16 gull-wing lead rows (visual detail).
    for k in range(8):
        lx = -12.5 + (k - 3.5) * 1.27
        for sy in (-1, 1):
            lead = box(lx, 6.5 + sy * 4.5, z0 - 0.25, 0.45, 1.5, 0.5)
            children.append(paint(lead, f"ic-pin-{k}-{sy}", _TIN))
    return bd.Compound(children=children, label="lcd1602_i2c")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(_backpack(), "lcd1602-i2c", None, Path(sys.argv[1]))
