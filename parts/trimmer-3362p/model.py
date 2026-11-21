# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""1/4 inch square single-turn trimmer, top adjust, pins in a triangle (Bourns 3362P).

Part frame: the standoffs' feet (the seating plane) on z = 0, the rotor's
axis up +Z on the Z axis. The front edge, where the top reads "1 3362 3",
faces -Y. Seen from the top, pin 1 is at -X, pin 3 at +X, both
3.53 behind the front edge, and the wiper, pin 2, 2.54 further back on
x = 0. These are leads 1 to 3, `pin-1` to `pin-3`.

The rotor, with its cross slot, is one node, `rotor`.

From Bourns' 3362 sheet, p1: body 6.60 wide, 6.99 deep and 4.88 above the
seating plane, standoffs 0.38, the rotor's axis 3.84 behind the front edge,
cross slot 2.77 long, 0.64 wide and 0.89 deep, pins Φ0.46 and 5.46 below
the seating plane. The rotor's diameters (`# verify`) are scaled off the
drawing.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin
from cad.geom import box, cyl_z

TRIM_BODY = (6.60, 6.99, 4.88)  # wide, deep, top above z = 0
TRIM_STANDOFF = (0.38, 0.8)  # tall, square  # verify, square
FRONT = -3.84  # the front edge's y: the rotor's axis is 3.84 behind it
TRIM_ROTOR = (3.8, 5.0, 0.6)  # Φ, Φ over the teeth, deep  # verify
TRIM_TEETH = 12  # verify
TRIM_SLOT = (2.77, 0.64, 0.89)  # long, wide, deep
TRIM_PIN = (0.46, 5.46)  # Φ, below z = 0
TRIM_PINS = ((1, -2.54, 3.53), (2, 0.0, 3.53 + 2.54), (3, 2.54, 3.53))  # lead, x, behind the front edge

_BLUE = plastic("#2F63B5")
_ROTOR = plastic("#EFEBE0")
_TIN = tin()


def trimmer_3362p_anchors():
    below = TRIM_PIN[1]
    return [glb.Anchor(lead, f"pin-{lead}", (x, FRONT + back, -below)) for lead, x, back in TRIM_PINS]


def _rotor(top):
    d, teeth_d, deep = TRIM_ROTOR
    z0 = top - deep
    rotor = cyl_z(0, 0, z0, top, d / 2.0)
    tooth = (teeth_d - d) / 2.0 + 0.2
    for k in range(TRIM_TEETH):
        rotor += (
            bd.Pos(0, 0, z0 + deep / 2.0)
            * bd.Rot(0, 0, k * 360.0 / TRIM_TEETH)
            * box(d / 2.0 + tooth / 2.0 - 0.2, 0, 0, tooth, 0.5, deep)
        )
    long, wide, slot_deep = TRIM_SLOT
    for turn in (0, 90):
        rotor -= bd.Pos(0, 0, top) * bd.Rot(0, 0, turn) * box(0, 0, 0, long, wide, 2.0 * slot_deep)
    return rotor.clean()


def trimmer_3362p_shape():
    wide, deep, top = TRIM_BODY
    tall, square = TRIM_STANDOFF
    _, teeth_d, rotor_deep = TRIM_ROTOR
    y = FRONT + deep / 2.0
    housing = box(0, y, (tall + top) / 2.0, wide, deep, top - tall)
    housing -= cyl_z(0, 0, top - rotor_deep, top + 0.1, teeth_d / 2.0 + 0.1)
    for sx in (-1, 1):
        for sy in (-1, 1):
            housing += box(
                sx * (wide - square) / 2.0,
                y + sy * (deep - square) / 2.0,
                (tall + 0.01) / 2.0,
                square,
                square,
                tall + 0.01,
            )
    children = [paint(housing.clean(), "body", _BLUE)]
    children.append(paint(_rotor(top), "rotor", _ROTOR))
    d, below = TRIM_PIN
    for lead, x, back in TRIM_PINS:
        children.append(paint(tht.round_lead(x, FRONT + back, -below, tall, d), f"pin-{lead}", _TIN))
    return bd.Compound(children=children, label="trimmer_3362p")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(trimmer_3362p_shape(), "trimmer-3362p", trimmer_3362p_anchors(), Path(sys.argv[1]))
