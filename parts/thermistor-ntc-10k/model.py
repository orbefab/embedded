# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Radial 10 kohm NTC thermistor (Vishay NTCLE100E3103JB0), leads as bought.

Part frame: the seating plane on z = 0; the epoxy bead stands above it centred
on the Z axis, wide along X and flatter along Y, with a blue dot on each broad
face per Vishay Doc. No. 29049 p1 ("gray base coating with a blue colored dot").
The two tin-plated copper leads run down on 2.54 mm along X, lead 1 at -X.

From Vishay Document Number 29049, p2 Physical Dimensions (330 ohm to 470 kohm):
B = 3.3 +/- 0.5 mm, Tmax = 3.0 mm, H1 = 1.0 to 3.0 mm, H2max = 6.0 mm,
P = 2.54 mm, d = 0.6 +/- 0.06 mm, L = 24 +/- 1.5 mm.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin
from cad.geom import box, cyl_z, fuse

PITCH = 2.54
LEAD_D = 0.6
LEAD_BELOW = 24.0
BEAD_W = 3.6  # within 3.3 +/- 0.5 mm
BEAD_T = 2.8  # <= Tmax 3.0 mm
BEAD_H1 = 1.2  # within 1.0 to 3.0 mm
BEAD_H2 = 5.6  # <= H2max 6.0 mm
MENISCUS_Z = 0.2  # coating meniscus above seating plane
DOT_D = 1.2
DOT_RAISED = 0.06

_EPOXY = plastic("#6E737A", roughness=0.32)
_BLUE = plastic("#1F5FC4", roughness=0.30)
_TIN = tin()


def thermistor_ntc_10k_anchors():
    return [
        glb.Anchor(lead, f"lead-{lead}", (sign * PITCH / 2.0, 0.0, -LEAD_BELOW)) for lead, sign in ((1, -1.0), (2, 1.0))
    ]


def _bead():
    height = BEAD_H2 - BEAD_H1
    zc = BEAD_H1 + height / 2.0
    raw = box(0, 0, zc, BEAD_W, BEAD_T, height)
    body = bd.fillet(raw.edges(), min(BEAD_W, BEAD_T, height) * 0.42)
    # Epoxy coating drops down each lead to the meniscus level above z = 0.
    drops = [cyl_z(sign * PITCH / 2.0, 0, MENISCUS_Z, BEAD_H1 + 1.0, LEAD_D / 2.0 + 0.35) for sign in (-1.0, 1.0)]
    return fuse([body, *drops]).clean()


def _dots():
    zc = BEAD_H1 + (BEAD_H2 - BEAD_H1) / 2.0
    dots = []
    for sign in (-1.0, 1.0):
        y = sign * (BEAD_T / 2.0)
        dot = bd.Pos(0, y, zc) * bd.Rot(90, 0, 0) * bd.Cylinder(DOT_D / 2.0, DOT_RAISED * 2.0)
        dots.append(dot)
    return fuse(dots).clean()


def thermistor_ntc_10k_shape():
    bead = _bead()
    children = [paint(bead, "body", _EPOXY), paint(_dots(), "dot", _BLUE)]
    for lead, sign in ((1, -1.0), (2, 1.0)):
        x = sign * PITCH / 2.0
        wire = tht.round_lead(x, 0, -LEAD_BELOW, BEAD_H1 + 0.8, LEAD_D) - bead
        children.append(paint(wire, f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="thermistor_ntc_10k")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        thermistor_ntc_10k_shape(),
        "thermistor-ntc-10k",
        thermistor_ntc_10k_anchors(),
        Path(sys.argv[1]),
    )
