# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Ceramic disc capacitor, 100 nF ("104"), leads as bought.

Part frame: the seating plane on z = 0; the disc stands in the X-Z plane
above it, its faces toward ±Y, "104" printed on the -Y face. The leads
run down on 5.0 along X from the disc's edge to below z = 0. The part is
not polarised; lead 1 is the one at -X, lead 2 the one at +X.

From the common 50 V disc (Vishay K-series and resellers' "104"
assortments): disc about Φ5.5 × 2.5, leads Φ0.5 on 5.0. The disc's height
above the seat and the lead lengths are estimates (`# verify`).
"""

import math
import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import srgb
from cad.mark import text

CC_DISC = (5.5, 2.5)  # Φ, thick  # verify
CC_LIFT = 2.0  # the disc's lowest point above the seat  # verify
CC_LEAD_D = 0.5
CC_PITCH = 5.0
CC_BELOW = 20.0  # verify

_DISC = srgb("#C8742B")
_TIN = srgb("#C5C8CE")
_PRINT = srgb("#1A1A1A")


def cap_ceramic_104_anchors():
    return [
        glb.Anchor(lead, f"lead-{lead}", (s * CC_PITCH / 2.0, 0.0, -CC_BELOW))
        for lead, s in ((1, -1), (2, 1))
    ]


def cap_ceramic_104_shape():
    d, t = CC_DISC
    r = d / 2.0
    zc = CC_LIFT + r
    disc = bd.Pos(0, 0, zc) * bd.Rot(90, 0, 0) * bd.Cylinder(r, t)
    children = [paint(disc, "disc", _DISC)]
    for lead, s in ((1, -1), (2, 1)):
        x = s * CC_PITCH / 2.0
        top = zc - math.sqrt(r * r - x * x) + 0.3
        wire = tht.round_lead(x, 0, -CC_BELOW, top, CC_LEAD_D) - disc
        children.append(paint(wire, f"lead-{lead}", _TIN))
    face = bd.Pos(0, -t / 2.0, 0) * bd.Rot(90, 0, 0)
    children.append(paint(face * text(0, zc, 0, "104", 1.4), "mark", _PRINT))
    return bd.Compound(children=children, label="cap_ceramic_104")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        cap_ceramic_104_shape(),
        "capacitor-ceramic-disc",
        cap_ceramic_104_anchors(),
        Path(sys.argv[1]),
    )
