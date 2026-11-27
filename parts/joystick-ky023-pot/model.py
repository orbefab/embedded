# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Side-mounted 10 kohm B103 axis potentiometer inside a KY-023 joystick module.

Part frame: centred on its turning axis along +Y (rotor faces +Y, base sits on
z = 0). Three solder pads sit along X at z = 0. The rotor node `rotor` turns
about +Y with `position` over 60 deg (+/-30 deg thumbstick travel).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box

CASE = (9.6, 2.8, 10.6)  # along X, thickness along Y, height along Z
ROTOR_Z = 5.6
ROTOR_D = 5.6
ROTOR_T = 1.2
PINS = ((1, -2.54), (2, 0.0), (3, 2.54))

_GREEN = plastic("#2E7A4B")
_ROTOR = plastic("#D8DAD6")
_SLOT = plastic("#23252A")
_TIN = tin()


def joystick_ky023_pot_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x, 0.0, -1.2), kind="pad") for lead, x in PINS]


def joystick_ky023_pot_shape():
    cx, cy, cz = CASE
    case = box(0, 0, cz / 2.0, cx, cy, cz)
    hub = bd.Pos(0, cy / 2.0 + ROTOR_T / 2.0, 0) * bd.Rot(90, 0, 0) * bd.Cylinder(ROTOR_D / 2.0, ROTOR_T)
    pointer = box(0, cy / 2.0 + ROTOR_T + 0.05, 1.3, 0.9, 0.25, 2.4)
    rotor = bd.Compound(children=[paint(hub, "hub", _ROTOR), paint(pointer, "pointer", _SLOT)], label="rotor")
    rotor.location = bd.Pos(0, 0, ROTOR_Z)
    children = [
        paint(case, "body", _GREEN),
        rotor,
    ]
    for lead, x in PINS:
        children.append(paint(box(x, 0, -0.6, 0.8, 0.5, 1.2), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="joystick_ky023_pot")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(joystick_ky023_pot_shape(), "joystick-ky023-pot", joystick_ky023_pot_anchors(), Path(sys.argv[1]))
