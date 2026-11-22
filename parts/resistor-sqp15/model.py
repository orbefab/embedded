# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Yageo SQP15A 15 W cement resistor, leads bent down at the body ends.

Part frame: the seating plane on z = 0, the ceramic body lying on it along X,
centred on the origin. The leads leave the centre of each end face, run 2 mm
past the ceramic, then bend down and run 8 mm below z = 0. Lead 1 is at -X.

From Yageo SQP/NSP: the SQP15A body is 48 long, 12.5 wide and 12.5 high, and
the leads are Φ0.8. The bend is the body plus a 2 mm shoulder, not the 0.4
inch pitch of a ¼ W resistor. orbe seats the leads 20 holes apart. The body
runs hot, so a scene usually sets it beside the board and jumpers to the leads.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin
from cad.geom import box

SQP_L = 48.0
SQP_W = 12.5
SQP_H = 12.5
SQP_LEAD_D = 0.8
SQP_PITCH = 52.0  # 2 mm shoulder past each face
SQP_BELOW = 8.0

_BODY = plastic("#E4E0D4")
_TIN = tin()


def resistor_sqp15_anchors():
    x = SQP_PITCH / 2.0
    return [glb.Anchor(lead, f"lead-{lead}", (side * x, 0.0, -SQP_BELOW)) for lead, side in ((1, -1), (2, 1))]


def resistor_sqp15_shape():
    body = paint(box(0, 0, SQP_H / 2.0, SQP_L, SQP_W, SQP_H), "body", _BODY)
    anode, cathode = tht.axial_leads(SQP_L, SQP_H, SQP_LEAD_D, pitch=SQP_PITCH, below=SQP_BELOW)
    leads = [paint(anode, "lead-1", _TIN), paint(cathode, "lead-2", _TIN)]
    return bd.Compound(children=[body, *leads], label="resistor_sqp15")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(resistor_sqp15_shape(), "resistor-sqp15", resistor_sqp15_anchors(), Path(sys.argv[1]))
