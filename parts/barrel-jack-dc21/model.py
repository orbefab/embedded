# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2.1 mm DC barrel jack, through-hole: the Uno's power jack.

Part frame: the jack's underside (the seating plane) on z = 0, centred on
the origin in X and Y, the plug's opening on the -X face. Lead 2 is the
centre pin, at the back. Leads 1 (at the -Y side) and 3 (in front of lead
2) are the sleeve and its switch. Each leg is anchored at its tip, 3.0
below the seat.

The body, 13.5 × 9.0 from the footprint's outline and 11.0 tall, and the
6.3 opening with its 2.1 pin are the common 2.1 × 5.5 jack (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box

BODY = (13.5, 9.0, 11.0)  # long, wide, tall  # verify, tall
OPENING = (6.3, 9.0, 6.5)  # diameter, deep, axis height  # verify
PIN = 2.1
LEGS = {1: (3.748, -4.495), 2: (6.745, 0.509), 3: (0.751, 0.509)}  # lead: x, y
LEG = (0.6, 2.4)  # thick, wide (blades across X)  # verify
BELOW = 3.0

_BLACK = plastic("#1A1A1A")
_TIN = tin()


def _along_x(x, y, z, r, length):
    return bd.Pos(x, y, z) * bd.Rot(0, 90, 0) * bd.Cylinder(r, length)


def barrel_jack_dc21_shape():
    length, width, height = BODY
    diameter, deep, axis_z = OPENING
    x0 = -length / 2.0
    body = box(0, 0, height / 2.0, length, width, height) - _along_x(
        x0 + deep / 2.0 - 0.05, 0, axis_z, diameter / 2.0, deep + 0.1
    )
    pin = _along_x(x0 + deep / 2.0 + 1.0, 0, axis_z, PIN / 2.0, deep - 2.0)
    children = [paint(body, "body", _BLACK), paint(pin, "pin", _TIN)]
    thick, wide = LEG
    for lead, (x, y) in LEGS.items():
        children.append(paint(box(x, y, -BELOW / 2.0, thick, wide, BELOW), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="barrel_jack")


def barrel_jack_dc21_anchors():
    return [glb.Anchor(n, f"lead-{n}", (x, y, -BELOW)) for n, (x, y) in LEGS.items()]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        barrel_jack_dc21_shape(),
        "barrel-jack-dc21",
        barrel_jack_dc21_anchors(),
        Path(sys.argv[1]),
    )
