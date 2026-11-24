# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""A 16x2 character LCD module panel (1602A class) with its HD44780U controller.

Part frame: the underside pads lie on z = 0, centred on the origin, the long
side along X and the top of the text toward +Y.

Dimensions follow the standard 1602A mechanical drawing (Winstar WH1602A /
Hitachi HD44780 16x2 module): 80.0 x 36.0 x 1.6 mm PCB with four 2.8 mm corner
mounting holes at (+/-37.5, +/-15.5), 71.2 x 24.2 x 8.0 mm black steel bezel,
and a 64.5 x 13.8 mm active glass window (`glass`) flush with the bezel top so
a viewer draws the backlit 16x2 character matrix on it.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import copper, gold, plastic, tin
from cad.geom import box, cyl_z

PCB = (80.0, 36.0, 1.6)
PAD_T = 0.03
BEZEL = (71.2, 24.2, 8.0)
ACTIVE = (64.5, 13.8)
GLASS_T = 0.05
HOLE_D = 2.8
HOLES = tuple((sx * 37.5, sy * 15.5) for sx in (-1, 1) for sy in (-1, 1))
HEADER_Y = 15.5
HEADER_PITCH = 2.54
HEADER_X0 = -32.0
# Five electrical interface pads on the underside for GND, VCC, SDA, SCL, V0.
LEADS = (1, 2, 3, 4, 5)
PAD = (1.2, 1.8)

_PCB = plastic("#1F6E3C")
_BEZEL = plastic("#1B1D21", roughness=0.35)
_GLASS = plastic("#0A1220", roughness=0.08)
_COPPER = copper()
_GOLD = gold()
_TIN = tin()


def _pad_xy(lead):
    return ((lead - 3) * 2.54, -13.0)


def hd44780_1602_i2c_shape():
    px, py, pz = PCB
    bx, by, bz = BEZEL
    ax, ay = ACTIVE
    pcb = box(0, 0, PAD_T + pz / 2.0, px, py, pz)
    for x, y in HOLES:
        pcb -= cyl_z(x, y, 0.0, PAD_T + pz + 0.4, HOLE_D / 2.0)

    bezel_z0 = PAD_T + pz
    bezel_top = bezel_z0 + bz
    pocket = box(0, 0, bezel_top - GLASS_T / 2.0, ax, ay, GLASS_T)
    bezel = box(0, 0, bezel_z0 + bz / 2.0, bx, by, bz) - pocket

    children = [
        paint(pcb, "pcb", _PCB),
        paint(bezel, "bezel", _BEZEL),
        paint(box(0, 0, bezel_top - GLASS_T / 2.0, ax, ay, GLASS_T), "glass", _GLASS),
    ]
    # 16-pin solder row along the top edge of the 1602A board.
    for k in range(16):
        hx = HEADER_X0 + k * HEADER_PITCH
        children.append(paint(cyl_z(hx, HEADER_Y, bezel_z0, bezel_z0 + 0.12, 0.85), f"ring-{k + 1}", _GOLD))
        children.append(paint(cyl_z(hx, HEADER_Y, bezel_z0, bezel_z0 + 0.6, 0.35), f"pin-{k + 1}", _TIN))

    for lead in LEADS:
        x, y = _pad_xy(lead)
        children.append(paint(box(x, y, PAD_T / 2.0, PAD[0], PAD[1], PAD_T), f"lead-{lead}", _COPPER))
    return bd.Compound(children=children, label="hd44780_1602_i2c")


def hd44780_1602_i2c_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (*_pad_xy(lead), 0.0), kind="pad") for lead in LEADS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        hd44780_1602_i2c_shape(),
        "hd44780-1602-i2c",
        hd44780_1602_i2c_anchors(),
        Path(sys.argv[1]),
    )
