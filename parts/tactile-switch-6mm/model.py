# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""6 × 6 mm through-hole tact switch with a tall round stem.

Part frame: body centre on the Z axis, body underside (the leg side) on
z = 0, stem toward +Z. Legs leave the two ±X faces and point down.

The 6 × 6 family comes in stem heights from 4.3 to 13 mm. This is the
7 mm one. The body size is the common outline.

The stem is its own node, `stem`, and presses 0.25 down -Z. Legs are
numbered as in the B3F top view: 1 and 2 along -Y, 3 and 4 along +Y, odd
numbers at +X. Legs 1 and 2 are joined, and so are 3 and 4. Pressing the
stem joins the pairs.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box, cyl_z

SWITCH_BODY = 6.0  # verify, square body
SWITCH_BODY_H = 3.5  # verify, underside to the top of the metal frame
SWITCH_H = 7.0  # verify, underside to the stem tip
SWITCH_STEM_D = 3.5  # verify
SWITCH_TRAVEL = 0.25
SWITCH_LEG_PITCH = 4.5  # verify, leg pairs along Y
SWITCH_LEG_X = 3.25  # verify, legs just outside the ±X faces
SWITCH_LEG_L = 3.5  # verify
SWITCH_LEG_W = 0.7
SWITCH_LEG_T = 0.3
SWITCH_LEGS = ((1, 1, -1), (2, -1, -1), (3, 1, 1), (4, -1, 1))  # lead, side in x, side in y

_BLACK = srgb("#1F1F22")
_STEEL = srgb("#B9BEC4")
_STEM = srgb("#2B2B2E")


def _leg_xy(sx, sy):
    return sx * SWITCH_LEG_X, sy * SWITCH_LEG_PITCH / 2.0


def tactile_switch_6mm_anchors():
    return [
        glb.Anchor(lead, f"leg-{lead}", (*_leg_xy(sx, sy), -SWITCH_LEG_L))
        for lead, sx, sy in SWITCH_LEGS
    ]


def tactile_switch_6mm_shape():
    base = box(0, 0, 1.25, SWITCH_BODY, SWITCH_BODY, 2.5)
    frame = box(0, 0, (2.5 + SWITCH_BODY_H) / 2.0, SWITCH_BODY, SWITCH_BODY, SWITCH_BODY_H - 2.5)
    frame -= cyl_z(0, 0, 2.4, SWITCH_BODY_H + 0.1, SWITCH_STEM_D / 2.0 + 0.4)
    stem = cyl_z(0, 0, 2.5 + SWITCH_TRAVEL, SWITCH_H, SWITCH_STEM_D / 2.0)
    legs = [
        paint(
            box(
                *_leg_xy(sx, sy),
                -SWITCH_LEG_L / 2.0 + 0.5,
                SWITCH_LEG_T,
                SWITCH_LEG_W,
                SWITCH_LEG_L + 1.0,
            ),
            f"leg-{lead}",
            _STEEL,
        )
        for lead, sx, sy in SWITCH_LEGS
    ]
    return bd.Compound(
        children=[
            paint(base, "body", _BLACK),
            paint(frame, "frame", _STEEL),
            paint(stem, "stem", _STEM),
            *legs,
        ],
        label="tact_switch",
    )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        tactile_switch_6mm_shape(),
        "tactile-switch-6mm",
        tactile_switch_6mm_anchors(),
        Path(sys.argv[1]),
    )
