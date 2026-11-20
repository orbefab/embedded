# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Radial aluminium electrolytic capacitor, 100 µF 16 V (6.3 × 11).

Part frame: the can's underside (the seating plane) on z = 0, the can
standing up +Z on the Z axis. Leads on 2.5 along X: positive (the longer,
lead 1) at -X, negative (lead 2) at +X, on the side of the sleeve's stripe.

From Nichicon's UVR / Panasonic's FC case tables for 100 µF 16 V: Φ6.3 ×
11, leads Φ0.5 on 2.5. The stripe's width, the top's recess and the lead
lengths are estimates (`# verify`).
"""

import math
import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import metal, plastic, tin
from cad.geom import cyl_z

CE_CAN = (6.3, 11.0)  # Φ, tall
CE_TOP = (5.4, 0.3)  # the top's foil Φ, recess  # verify
CE_STRIPE = 70.0  # degrees of the sleeve, centred on +X  # verify
CE_LEAD_D = 0.5
CE_PITCH = 2.5
CE_LEADS = ((1, -1.25, 20.0), (2, 1.25, 17.5))  # lead (+, -), x, below z = 0  # verify

_SLEEVE = plastic("#1B1B1D")
_STRIPE = plastic("#B9BCC2")
_FOIL = metal("#D3D6DB")
_TIN = tin()


def _wedge(r, h):
    half = math.radians(CE_STRIPE / 2.0)
    far = 2.0 * r
    pts = [
        (0, 0),
        (far * math.cos(-half), far * math.sin(-half)),
        (far, 0),
        (far * math.cos(half), far * math.sin(half)),
    ]
    with bd.BuildSketch() as sketch:
        bd.Polygon(*pts, align=None)
    return bd.Pos(0, 0, -0.1) * bd.extrude(sketch.sketch, amount=h + 0.2)


def cap_electrolytic_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x, 0.0, -below)) for lead, x, below in CE_LEADS]


def cap_electrolytic_shape():
    d, h = CE_CAN
    fd, recess = CE_TOP
    can = cyl_z(0, 0, 0, h, d / 2.0) - cyl_z(0, 0, h - recess, h + 0.1, fd / 2.0)
    wedge = _wedge(d / 2.0, h)
    children = [
        paint(can - wedge, "sleeve", _SLEEVE),
        paint(can & wedge, "stripe", _STRIPE),
        paint(cyl_z(0, 0, h - recess, h - recess + 0.05, fd / 2.0 - 0.01), "top", _FOIL),
    ]
    for lead, x, below in CE_LEADS:
        children.append(paint(tht.round_lead(x, 0, -below, 0.0, CE_LEAD_D), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="cap_electrolytic")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        cap_electrolytic_shape(),
        "capacitor-electrolytic-radial",
        cap_electrolytic_anchors(),
        Path(sys.argv[1]),
    )
