# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""1N4007 rectifier diode (DO-41), leads bent for a breadboard.

Part frame: the seating plane on z = 0, the body lying on it along X,
centred on the origin; the cathode band toward +X. The leads bend down at
x = ±5.08 (0.4") and run 8 below z = 0: the anode at -X is lead 1, the
cathode at +X lead 2.

From the DO-41 outline (Diodes Inc. and onsemi 1N4001 to 1N4007 datasheets):
body up to 5.2 long and Φ2.7, leads up to Φ0.86; drawn at those maxima.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import srgb

D_LENGTH = 5.2
D_DIA = 2.7
D_LEAD_D = 0.8
D_BAND = (0.8, 0.4)  # wide, in from the +X end  # verify
D_BELOW = 8.0

_BODY = srgb("#1A1A1A")
_BAND = srgb("#C9CCD1")
_TIN = srgb("#C5C8CE")


def _segment(x0, x1):
    return bd.Pos((x0 + x1) / 2.0, 0, D_DIA / 2.0) * bd.Rot(0, 90, 0) * bd.Cylinder(D_DIA / 2.0, x1 - x0)


def diode_1n4007_anchors():
    x = tht.AXIAL_PITCH / 2.0
    return [glb.Anchor(lead, f"lead-{lead}", (s * x, 0.0, -D_BELOW)) for lead, s in ((1, -1), (2, 1))]


def diode_1n4007_shape():
    end = D_LENGTH / 2.0
    w, inset = D_BAND
    b0, b1 = end - inset - w, end - inset
    children = [
        paint(_segment(-end, b0), "body", _BODY),
        paint(_segment(b0, b1), "band-cathode", _BAND),
        paint(_segment(b1, end), "body-end", _BODY),
    ]
    anode, cathode = tht.axial_leads(D_LENGTH, D_DIA, D_LEAD_D, below=D_BELOW)
    children += [paint(anode, "lead-1", _TIN), paint(cathode, "lead-2", _TIN)]
    return bd.Compound(children=children, label="diode_1n4007")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(diode_1n4007_shape(), "diode-1n4007", diode_1n4007_anchors(), Path(sys.argv[1]))
