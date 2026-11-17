# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""N&H Technology NTC013 SMT tact switch, 4.2 x 3.2 x 2.5.

Part frame: on its pads, the can on z = 0, centred on the origin, the
button up +Z. The drawing's top view puts pins 1 and 2 on the +Y row and
pins 3 and 4 on the -Y row, left to right, so pin 1 is at -X. Pins 1 and 2
are one contact and pins 3 and 4 the other. Pressing the button joins them.
The button is its own node, `button`.

From the NTC013 series drawing dated 20170417. The body is 4.2 by 3.2.
The side view reads 1.8 for the can, 2.5 overall and 0.25 for the terminal
thickness, and 4.6 over the terminals. The top view labels the button 3.0
by 1.5. The land drawing gives an outer span of 5.2, an inner gap of 3.1
and a row pitch of 2.8, so the land is 1.05 along X and its centre is at
±2.075. The land's extent along Y is not numbered.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

CAN = (4.2, 3.2, 1.8)  # x, y, height, the side view's 1.8
BUTTON = (3.0, 1.5)  # the top view's labelled button
BUTTON_TOP = 2.5  # overall height
TRAVEL = 0.2
POCKET = 0.3  # verify, the pocket under the button
# x, y, thickness. The Y extent is not on the drawing.
LEG = (1.05, 1.0, 0.25)  # verify the 1.0
LEG_X = 2.075
LEG_Y = 1.4  # half of the 2.8 row pitch
LEGS = (
    (1, -1, 1),
    (2, 1, 1),
    (3, -1, -1),
    (4, 1, -1),
)  # lead, side in x, side in y

_BLACK = srgb("#1A1A1A")
_BUTTON = srgb("#4A4E55")
_TIN = srgb("#C5C8CE")


def _leg_xy(sx, sy):
    return sx * LEG_X, sy * LEG_Y


def tactile_switch_ntc013_shape():
    cx, cy, ch = CAN
    bx, by = BUTTON
    lx, ly, lt = LEG
    floor = ch - POCKET
    can = box(0, 0, ch / 2.0, cx, cy, ch) - box(0, 0, ch, bx, by, 2 * POCKET)
    bottom = floor + TRAVEL
    children = [
        paint(can, "body", _BLACK),
        paint(
            box(0, 0, (bottom + BUTTON_TOP) / 2.0, bx, by, BUTTON_TOP - bottom),
            "button",
            _BUTTON,
        ),
    ]
    for lead, sx, sy in LEGS:
        children.append(paint(box(*_leg_xy(sx, sy), lt / 2.0, lx, ly, lt), f"leg-{lead}", _TIN))
    return bd.Compound(children=children, label="tactile_switch_ntc013")


def tactile_switch_ntc013_anchors():
    return [glb.Anchor(lead, f"leg-{lead}", (*_leg_xy(sx, sy), 0.0), kind="pad") for lead, sx, sy in LEGS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        tactile_switch_ntc013_shape(),
        "tactile-switch-ntc013",
        tactile_switch_ntc013_anchors(),
        Path(sys.argv[1]),
    )
