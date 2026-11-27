# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Center-push tactile switch and thumbstick cap inside a KY-023 joystick module.

Part frame: z = 0 sits on the PCB top inside the KY-023 gimbal. The clickable
node `plunger` is the black PS2-style concave thumbstick cap and shaft rising
to z = 24.4 mm (26 mm total module height over the 1.6 mm PCB, matching Joy-IT
COM-KY023JM dimensions 34 x 39 x 26 mm). Clicking the cap in the 3D viewer
presses this switch and slides `plunger` down along -Z.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box, cyl_z, fuse

CAP_D = 24.0
CAP_Z0 = 16.0
CAP_Z1 = 24.4  # + 1.6 mm PCB = 26.0 mm overall height (Joy-IT COM-KY023JM p1)
SKIRT_D = 21.0
SKIRT_Z0 = 10.5
SHAFT_D = 5.0

_CAP = plastic("#1F2024", roughness=0.62)
_RIM = plastic("#2D2F36", roughness=0.50)
_TIN = tin()


def _cap():
    # Stem + flared lower dome skirt + concave thumb cap.
    stem = cyl_z(0, 0, 5.0, CAP_Z0 + 1.0, SHAFT_D / 2.0)
    skirt = bd.Pos(0, 0, SKIRT_Z0) * bd.Cone(SKIRT_D / 2.0, 7.5, CAP_Z0 - SKIRT_Z0 + 0.5)
    head = cyl_z(0, 0, CAP_Z0, CAP_Z1, CAP_D / 2.0)
    head = bd.fillet(head.edges(), 1.4)
    # Dish out the top slightly so it reads immediately as a thumbstick cap.
    dish = bd.Pos(0, 0, CAP_Z1 + 12.5) * bd.Sphere(14.5)
    head = head - dish
    return fuse([stem, skirt, head]).clean()


def _ring():
    # Subtle grip ring around the concave thumb pad.
    return cyl_z(0, 0, CAP_Z1 - 0.5, CAP_Z1 + 0.15, CAP_D / 2.0 - 1.5) - cyl_z(
        0, 0, CAP_Z1 - 0.7, CAP_Z1 + 0.3, CAP_D / 2.0 - 3.0
    )


def joystick_ky023_switch_shape():
    plunger = bd.Compound(
        children=[paint(_cap(), "cap", _CAP), paint(_ring(), "grip", _RIM)],
        label="plunger",
    )
    children = [
        plunger,
        paint(box(-2.0, -7.5, 0.2, 0.8, 0.8, 0.4), "lead-1", _TIN),
        paint(box(2.0, -7.5, 0.2, 0.8, 0.8, 0.4), "lead-2", _TIN),
    ]
    return bd.Compound(children=children, label="joystick_ky023_switch")


def joystick_ky023_switch_anchors():
    return [
        glb.Anchor(1, "lead-1", (-2.0, -7.5, 0.0), kind="pad"),
        glb.Anchor(2, "lead-2", (2.0, -7.5, 0.0), kind="pad"),
    ]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        joystick_ky023_switch_shape(),
        "joystick-ky023-switch",
        joystick_ky023_switch_anchors(),
        Path(sys.argv[1]),
    )
