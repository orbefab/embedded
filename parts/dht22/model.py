# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""DHT22 (AM2302) temperature and humidity sensor, four leads in a row.

Part frame: the body's underside (the seating plane) on z = 0, the body
standing up +Z, centred on the Z axis. The grille faces -Y. Seen from the
grille, lead 1 (VDD) is at -X and lead 4 (GND) at +X, on 2.54. These are
leads 1 to 4, `lead-1` to `lead-4`.

The sizes are from the drawing on page 1 of the AM2302 manual v1.1: a
body 15.4 wide, 7.8 deep and 25.3 tall, leads 0.5 across the front and 0.3
deep, and a Φ2.7 mounting hole near the top. The drawing gives no lead
length or hole height, so 6.5 below the seating plane and 2.8 below the
top are `# verify`. The grille is a lattice of square openings across the
front, below that hole.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin
from cad.geom import box, fuse

DHT_BODY = (15.4, 7.8, 25.3)  # wide, deep, tall
DHT_LEAD = (0.5, 0.3)  # across the front, deep
DHT_BELOW = 6.5  # verify
DHT_PITCH = 2.54
DHT_HOLE = (2.7, 2.8)  # Φ, its centre below the top  # verify, below
DHT_LATTICE = (0.72, 1.22, 0.6, 10, 12)  # opening, pitch, deep, columns, rows

_WHITE = plastic("#EEEDE6")
_SLOT = plastic("#2B2B2B")
_TIN = tin()


def _x(lead):
    return (lead - 2.5) * DHT_PITCH


def dht22_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (_x(lead), 0.0, -DHT_BELOW)) for lead in range(1, 5)]


def _hole():
    d, below = DHT_HOLE
    z = DHT_BODY[2] - below
    return bd.Pos(0, 0, z) * bd.Rot(90, 0, 0) * bd.Cylinder(d / 2.0, DHT_BODY[1] + 1.0)


def _lattice():
    opening, pitch, deep, cols, rows = DHT_LATTICE
    front = -DHT_BODY[1] / 2.0
    x0 = -((cols - 1) / 2.0) * pitch
    z0 = 3.4
    holes = [
        box(x0 + c * pitch, front + deep / 2.0, z0 + r * pitch, opening, deep, opening)
        for c in range(cols)
        for r in range(rows)
    ]
    return fuse(holes), (x0, z0, cols, rows, opening, pitch, deep)


def dht22_shape():
    wide, deep, tall = DHT_BODY
    shell = box(0, 0, tall / 2.0, wide, deep, tall)
    holes, (x0, z0, cols, rows, opening, pitch, grille_deep) = _lattice()
    body = (shell - _hole() - holes).clean()
    # One dark floor behind the openings, so the lattice reads as open.
    floor = -deep / 2.0 + grille_deep + 0.05
    span_x = (cols - 1) * pitch + opening
    span_z = (rows - 1) * pitch + opening
    backs = box(x0 + (cols - 1) * pitch / 2.0, floor, z0 + (rows - 1) * pitch / 2.0, span_x, 0.1, span_z)
    children = [paint(body, "body", _WHITE), paint(backs, "grille", _SLOT)]
    for lead in range(1, 5):
        children.append(paint(tht.square_lead(_x(lead), 0, -DHT_BELOW, 0.0, *DHT_LEAD), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="dht22")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(dht22_shape(), "dht22", dht22_anchors(), Path(sys.argv[1]))
