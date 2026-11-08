# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Surface-mount tact switch, 3.5 × 4.7: the Nano's reset button.

Part frame: on its pads, metal can on z = 0, centred on the origin, the
button up +Z. The four legs leave the ±Y ends and are numbered as the
6 mm switch's: 1 and 2 at -Y, 3 and 4 at +Y, odd numbers at +X. Legs 1
and 2 are joined, and so are 3 and 4. Pressing the button joins the pairs.
The button is its own node, `button`, sitting in a pocket in the can so it
presses 0.2 down -Z.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

SMD_SW_CAN = (3.5, 4.7, 1.5)  # x, y, height
SMD_SW_BUTTON = (1.7, 2.7)  # x, y
SMD_SW_BUTTON_TOP = 2.02
SMD_SW_TRAVEL = 0.2  # verify, the button's press
SMD_SW_POCKET = 0.3  # verify, the can's pocket under the button
SMD_SW_LEG = (0.7, 0.8, 0.15)  # x, y out of the can, thickness  # verify
SMD_SW_LEG_X = 1.1  # verify
SMD_SW_LEGS = (
    (1, 1, -1),
    (2, -1, -1),
    (3, 1, 1),
    (4, -1, 1),
)  # lead, side in x, side in y  # verify

_TIN = srgb("#C5C8CE")
_CREAM = srgb("#F3E6C4")


def _leg_xy(sx, sy):
    return sx * SMD_SW_LEG_X, sy * (SMD_SW_CAN[1] + SMD_SW_LEG[1]) / 2.0


def tactile_switch_smd_shape():
    cx, cy, ch = SMD_SW_CAN
    bx, by = SMD_SW_BUTTON
    lx, ly, lt = SMD_SW_LEG
    floor = ch - SMD_SW_POCKET
    can = box(0, 0, ch / 2.0, cx, cy, ch) - box(0, 0, ch, bx, by, 2 * SMD_SW_POCKET)
    bottom = floor + SMD_SW_TRAVEL
    children = [
        paint(can, "body", _TIN),
        paint(
            box(0, 0, (bottom + SMD_SW_BUTTON_TOP) / 2.0, bx, by, SMD_SW_BUTTON_TOP - bottom),
            "button",
            _CREAM,
        ),
    ]
    for lead, sx, sy in SMD_SW_LEGS:
        children.append(paint(box(*_leg_xy(sx, sy), lt / 2.0, lx, ly, lt), f"leg-{lead}", _TIN))
    return bd.Compound(children=children, label="tactile_switch_smd")


def tactile_switch_smd_anchors():
    return [
        glb.Anchor(lead, f"leg-{lead}", (*_leg_xy(sx, sy), 0.0), kind="pad")
        for lead, sx, sy in SMD_SW_LEGS
    ]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        tactile_switch_smd_shape(),
        "tactile-switch-smd",
        tactile_switch_smd_anchors(),
        Path(sys.argv[1]),
    )
