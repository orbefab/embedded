# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""1N4733A 5.1 V zener diode (DO-41), leads bent for a breadboard.

Part frame: the seating plane on z = 0, the body lying on it along X,
centred on the origin; the cathode band toward +X. The leads bend down at
x = ±5.08 (0.4") and run 8 below z = 0: the anode at -X is lead 1, the
cathode at +X lead 2.

From Vishay's DO-41 outline for the 1N4728A to 1N4761A series: body up to
4.1 long and Φ2.6, leads up to Φ0.86; drawn at the body's maxima and at a
0.8 lead, inside that maximum.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin

Z_LENGTH = 4.1
Z_DIA = 2.6
Z_LEAD_D = 0.8
Z_BAND = (0.6, 0.3)  # wide, in from the +X end
Z_BELOW = 8.0

_BODY = plastic("#C4622D")
_BAND = plastic("#1A1A1A")
_TIN = tin()


def _segment(x0, x1):
    return bd.Pos((x0 + x1) / 2.0, 0, Z_DIA / 2.0) * bd.Rot(0, 90, 0) * bd.Cylinder(Z_DIA / 2.0, x1 - x0)


def zener_1n4733a_anchors():
    x = tht.AXIAL_PITCH / 2.0
    return [glb.Anchor(lead, f"lead-{lead}", (s * x, 0.0, -Z_BELOW)) for lead, s in ((1, -1), (2, 1))]


def zener_1n4733a_shape():
    end = Z_LENGTH / 2.0
    w, inset = Z_BAND
    b0, b1 = end - inset - w, end - inset
    children = [
        paint(_segment(-end, b0), "body", _BODY),
        paint(_segment(b0, b1), "band-cathode", _BAND),
        paint(_segment(b1, end), "body-end", _BODY),
    ]
    anode, cathode = tht.axial_leads(Z_LENGTH, Z_DIA, Z_LEAD_D, below=Z_BELOW)
    children += [paint(anode, "lead-1", _TIN), paint(cathode, "lead-2", _TIN)]
    return bd.Compound(children=children, label="zener_1n4733a")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(zener_1n4733a_shape(), "zener-1n4733a", zener_1n4733a_anchors(), Path(sys.argv[1]))
