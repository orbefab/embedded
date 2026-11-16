# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""5 mm CdS photoresistor (GL5528), leads as bought.

Part frame: the disc's underside (the seating plane) on z = 0, its face up
+Z, centred on the Z axis. The two leads run down on 3.0 along X, lead 1 at
-X. The disc is cut flat across Y.

From Senba's GL55 manual, p3: disc Φ5.1, 4.3 across the flats, 2.1 thick,
leads Φ0.50 on 3.0, 34 from the face to the lead ends. The cadmium-sulphide
track is a meander of five runs between the leads (`# verify`, the drawing
gives no track size).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import srgb
from cad.geom import box, cyl_z, fuse

LDR_DISC = (5.1, 4.3, 2.1)  # Φ, across the flats, thick
LDR_LEAD = 0.5
LDR_LEADS = ((1, -1.5), (2, 1.5))  # lead, x
LDR_BELOW = 34.0 - LDR_DISC[2]
LDR_TRACK = (5, 2.2, 0.3, 0.02)  # runs, long, wide, raised  # verify

_CERAMIC = srgb("#ECE6D6")
_CDS = srgb("#B5462A")
_TIN = srgb("#C5C8CE")


def ldr_5mm_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x, 0.0, -LDR_BELOW)) for lead, x in LDR_LEADS]


def _track(top):
    runs, long, wide, raised = LDR_TRACK
    pitch = (LDR_DISC[1] - 2.0 * wide) / runs
    z = top + raised / 2.0
    ys = [(k - (runs - 1) / 2.0) * pitch for k in range(runs)]
    pieces = [box(0, y, z, long, wide, raised) for y in ys]
    for k in range(runs - 1):
        x = (long - wide) / 2.0 * (1 if k % 2 == 0 else -1)
        pieces.append(box(x, (ys[k] + ys[k + 1]) / 2.0, z, wide, pitch, raised))
    return fuse(pieces).clean()


def ldr_5mm_shape():
    d, flats, thick = LDR_DISC
    disc = cyl_z(0, 0, 0, thick, d / 2.0) & box(0, 0, thick / 2.0, d + 1.0, flats, thick + 1.0)
    children = [paint(disc, "body", _CERAMIC), paint(_track(thick), "track", _CDS)]
    for lead, x in LDR_LEADS:
        children.append(paint(tht.round_lead(x, 0, -LDR_BELOW, 0.0, LDR_LEAD), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="ldr_5mm")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(ldr_5mm_shape(), "ldr-5mm", ldr_5mm_anchors(), Path(sys.argv[1]))
