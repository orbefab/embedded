# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""A 16x2 character LCD panel (1602A class) with its HD44780U controller.

Part frame: the bezel's underside, the top of the module's board, is z = 0,
centred on the board's centre, the long side along X and the 16-pin header
row toward +Y.

From Vishay LCD-016N002B, Document Number 37295, the dimensions drawing on
p. 2: an 80.0 x 36.0 board, a 71.2 x 25.2 bezel 4.95 in from the board's
left edge and 5.7 down from its top edge, the 66.0 x 16.0 viewing area and
the 56.2 x 11.5 active area centred in it, and 16 holes on 2.54, the first
8.0 in from the left edge and 2.5 down from the top. The drawing's H2 for an
LED backlight, 8.6, is read as the board's underside to the bezel's top, so
the bezel stands 7.0 above a 1.6 board.

`glass` is its own node, so a viewer can draw the screen on it: the viewing
area's 66.0 width at the drawn glass's 320 by 68 aspect, centred in the
viewing area, flush with the bezel's top. Each header hole's land on the
board's top is the lead of that pin.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import gold, plastic
from cad.geom import box

BOARD = (80.0, 36.0)
PCB_T = 1.6
BEZEL = (71.2, 25.2)
BEZEL_LEFT = 4.95  # from the board's left edge
BEZEL_TOP = 5.7  # from the board's top edge
BEZEL_H = 8.6 - PCB_T
VIEW = (66.0, 16.0)
GLASS = (66.0, 66.0 * 68.0 / 320.0)
GLASS_T = 0.05
HEADER_X0 = -BOARD[0] / 2.0 + 8.0
HEADER_Y = BOARD[1] / 2.0 - 2.5
HEADER_PITCH = 2.54
PAD = 1.7  # verify, the square land around each hole
PAD_T = 0.12
LEADS = tuple(range(1, 17))

BEZEL_X = -BOARD[0] / 2.0 + BEZEL_LEFT + BEZEL[0] / 2.0
BEZEL_Y = BOARD[1] / 2.0 - BEZEL_TOP - BEZEL[1] / 2.0

_BEZEL = plastic("#1B1D21", roughness=0.35)
_GLASS = plastic("#0A1220", roughness=0.08)
_GOLD = gold()


def _lead_xy(lead):
    return (HEADER_X0 + (lead - 1) * HEADER_PITCH, HEADER_Y)


def hd44780_1602_shape():
    bx, by = BEZEL
    gx, gy = GLASS
    top = BEZEL_H
    pocket = box(BEZEL_X, BEZEL_Y, top - GLASS_T / 2.0, gx, gy, GLASS_T)
    bezel = box(BEZEL_X, BEZEL_Y, top / 2.0, bx, by, top) - pocket
    children = [
        paint(bezel, "bezel", _BEZEL),
        paint(box(BEZEL_X, BEZEL_Y, top - GLASS_T / 2.0, gx, gy, GLASS_T), "glass", _GLASS),
    ]
    for lead in LEADS:
        x, y = _lead_xy(lead)
        children.append(paint(box(x, y, PAD_T / 2.0, PAD, PAD, PAD_T), f"lead-{lead}", _GOLD))
    return bd.Compound(children=children, label="hd44780_1602")


def hd44780_1602_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (*_lead_xy(lead), 0.0), kind="pad") for lead in LEADS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(hd44780_1602_shape(), "hd44780-1602", hd44780_1602_anchors(), Path(sys.argv[1]))
