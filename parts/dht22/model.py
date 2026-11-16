# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""DHT22 (AM2302) temperature and humidity sensor, four leads in a row.

Part frame: the body's underside (the seating plane) on z = 0, the body
standing up +Z, centred on the Z axis. The grille faces -Y. Seen from the
grille, lead 1 (VDD) is at -X and lead 4 (GND) at +X, on 2.54. These are
leads 1 to 4, `lead-1` to `lead-4`.

The DHT22 sheet gives no drawing in its text. The sizes are the common
15.1 wide, 7.7 deep and 25.1 tall body, leads 0.5 square, 6.5 below the
seating plane, and a Φ3 mounting hole near the top (`# verify`, all of
them). The grille is eight slots across the front.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import srgb
from cad.geom import box, fuse

DHT_BODY = (15.1, 7.7, 25.1)  # wide, deep, tall  # verify
DHT_LEAD = 0.5  # square  # verify
DHT_BELOW = 6.5  # verify
DHT_PITCH = 2.54
DHT_HOLE = (3.0, 2.8)  # Φ, its centre below the top  # verify
DHT_GRILLE = (8, 11.0, 1.0, 0.6, 2.2)  # slots, long, wide, deep, pitch

_WHITE = srgb("#EEEDE6")
_SLOT = srgb("#2B2B2B")
_TIN = srgb("#C5C8CE")


def _x(lead):
    return (lead - 2.5) * DHT_PITCH


def dht22_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (_x(lead), 0.0, -DHT_BELOW)) for lead in range(1, 5)]


def _hole():
    d, below = DHT_HOLE
    z = DHT_BODY[2] - below
    return bd.Pos(0, 0, z) * bd.Rot(90, 0, 0) * bd.Cylinder(d / 2.0, DHT_BODY[1] + 1.0)


def _slots():
    count, long, wide, deep, pitch = DHT_GRILLE
    front = -DHT_BODY[1] / 2.0
    first = DHT_BODY[2] - DHT_HOLE[1] - DHT_HOLE[0] - 1.5
    return fuse([box(0, front + deep / 2.0, first - k * pitch, long, deep, wide) for k in range(count)])


def dht22_shape():
    wide, deep, tall = DHT_BODY
    shell = box(0, 0, tall / 2.0, wide, deep, tall)
    slots = _slots()
    body = (shell - _hole() - slots).clean()
    # Each slot's floor is dark, so the grille reads as open.
    floor = -deep / 2.0 + DHT_GRILLE[3] + 0.05
    backs = fuse([box(0, floor, s.center().Z, DHT_GRILLE[1], 0.1, DHT_GRILLE[2]) for s in slots.solids()])
    children = [paint(body, "body", _WHITE), paint(backs, "grille", _SLOT)]
    for lead in range(1, 5):
        children.append(paint(tht.square_lead(_x(lead), 0, -DHT_BELOW, 0.0, DHT_LEAD), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="dht22")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(dht22_shape(), "dht22", dht22_anchors(), Path(sys.argv[1]))
