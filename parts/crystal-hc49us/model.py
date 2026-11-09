# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""HC-49/US low profile crystal, any frequency.

Part frame: the base's underside (the seating plane) on z = 0, the can
standing up +Z, centred on the origin, its length along X. Lead 1 is the
leg at -X, lead 2 the one at +X; the part is not polarised. Each leg is
anchored at its tip.

From ECS' HC-49USX datasheet, height code 4X: base 11.35 × 4.65 at most,
can 10.5 × 3.7, 3.5 tall at most, legs Φ0.43 on 4.88. The legs are drawn
3.0 below the seat, through a 1.6 board.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import srgb
from cad.geom import box, cyl_z

BASE = (11.35, 4.65, 0.4)  # long, wide, tall
CAN = (10.5, 3.7, 3.5)  # long, wide, top above the seat
LEG = (0.43, 4.88, 3.0)  # Φ, apart, below the seat

_CAN = srgb("#D0D3D8")
_BASE = srgb("#B9BCC2")
_TIN = srgb("#C5C8CE")


def _stadium(length, width, z0, z1):
    r = width / 2.0
    straight = length - width
    return (
        box(0, 0, (z0 + z1) / 2.0, straight, width, z1 - z0)
        + cyl_z(-straight / 2.0, 0, z0, z1, r)
        + cyl_z(straight / 2.0, 0, z0, z1, r)
    )


def _leg_x(lead):
    return (-1 if lead == 1 else 1) * LEG[1] / 2.0


def crystal_hc49us_shape():
    bl, bw, bh = BASE
    cl, cw, top = CAN
    d, _, below = LEG
    children = [
        paint(_stadium(bl, bw, 0.0, bh), "base", _BASE),
        paint(_stadium(cl, cw, bh, top), "can", _CAN),
    ]
    for lead in (1, 2):
        children.append(paint(tht.round_lead(_leg_x(lead), 0, -below, 0.0, d), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="crystal_hc49us")


def crystal_hc49us_anchors():
    return [glb.Anchor(n, f"lead-{n}", (_leg_x(n), 0.0, -LEG[2])) for n in (1, 2)]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(crystal_hc49us_shape(), "crystal-hc49us", crystal_hc49us_anchors(), Path(sys.argv[1]))
