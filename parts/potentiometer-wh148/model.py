# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""10 kΩ rotary potentiometer, 16 mm, with a push-on knob.

Part frame: the pot stands on the board, its back (the seating plane) on
z = 0 and the shaft up +Z on the Z axis. The three pins run down on 5.0
along X at y = -7.5. Seen from the shaft with the pins down, terminal 1
is on the left (-X), the wiper 2 in the middle and terminal 3 on the
right. These are leads 1 to 3, `pin-1` to `pin-3`. The knob's pointer line
faces -Y at the end of a clockwise turn.

The shaft, the knob and its pointer are one node, `shaft`, whose origin is
on the turn axis.

From the WH148 outline: can 16 × 7, bushing 7 × 7, knurled shaft 6 rising
15 from the can's face, pins on 5.0, a locating lug at radius 7. The knob
(15 × 13, `# verify`) is a common kit cap.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import gold, metal, plastic, steel, tin
from cad.geom import box, cyl_z

POT_CAN = (16.0, 7.0)  # diameter, deep
POT_BUSHING = (7.0, 7.0)  # diameter, long
POT_SHAFT = (6.0, 15.0)  # diameter, above the can's face
POT_PINS = ((1, -5.0), (2, 0.0), (3, 5.0))  # lead, x
POT_PIN_Y = -7.5  # verify
POT_PIN = (1.2, 0.5, 6.0)  # wide, thick, below z = 0  # verify
POT_LUG = (7.0, 1.2, 2.0, 1.0)  # radius, wide, along, tall  # verify
POT_KNOB = (15.0, 13.0, 2.0)  # diameter, tall, gap above the bushing  # verify

_CAN = metal("#B9BCC2")
_BASE = plastic("#2B5FA8")
_BRASS = gold("#C8A85A")
_STEEL = steel("#D0D3D8")
_KNOB = plastic("#1B1B1D")
_POINTER = plastic("#F2F2F2")
_TIN = tin()


def potentiometer_wh148_anchors():
    below = POT_PIN[2]
    return [glb.Anchor(lead, f"pin-{lead}", (x, POT_PIN_Y, -below)) for lead, x in POT_PINS]


def potentiometer_wh148_shape():
    diameter, deep = POT_CAN
    bushing_d, bushing_l = POT_BUSHING
    shaft_d, shaft_l = POT_SHAFT
    base_t = 2.0
    children = [
        paint(cyl_z(0, 0, 0, base_t, diameter / 2.0), "base", _BASE),
        paint(cyl_z(0, 0, base_t, deep, diameter / 2.0), "can", _CAN),
        paint(
            cyl_z(0, 0, deep, deep + bushing_l, bushing_d / 2.0)
            - cyl_z(0, 0, deep - 0.1, deep + bushing_l + 0.1, shaft_d / 2.0 + 0.05),
            "bushing",
            _BRASS,
        ),
    ]
    shaft = cyl_z(0, 0, deep, deep + shaft_l, shaft_d / 2.0)
    for k in range(18):
        groove = bd.Pos(0, 0, deep + shaft_l - 3.5) * bd.Rot(0, 0, k * 20) * box(shaft_d / 2.0, 0, 0, 0.4, 0.4, 7.2)
        shaft -= groove
    turning = [paint(shaft, "spindle", _STEEL)]
    radius, lug_w, lug_a, lug_h = POT_LUG
    children.append(paint(box(0, radius, deep + lug_h / 2.0, lug_w, lug_a, lug_h), "lug", _CAN))
    knob_d, knob_h, gap = POT_KNOB
    knob_z = deep + bushing_l + gap
    knob = cyl_z(0, 0, knob_z, knob_z + knob_h, knob_d / 2.0) - cyl_z(0, 0, knob_z - 0.1, deep + shaft_l, shaft_d / 2.0)
    pointer = box(0, -knob_d / 4.0, knob_z + knob_h + 0.01, 0.8, knob_d / 2.0 - 1.0, 0.02)
    turning += [paint(knob, "knob", _KNOB), paint(pointer, "knob-pointer", _POINTER)]
    children.append(bd.Compound(children=turning, label="shaft"))
    pin_w, pin_t, below = POT_PIN
    for lead, x in POT_PINS:
        children.append(paint(tht.square_lead(x, POT_PIN_Y, -below, 0.0, pin_w, pin_t), f"pin-{lead}", _TIN))
    return bd.Compound(children=children, label="pot_wh148")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        potentiometer_wh148_shape(),
        "potentiometer-wh148",
        potentiometer_wh148_anchors(),
        Path(sys.argv[1]),
    )
