# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""SG90 micro servo, three leads on 2.54 mm.

Part frame: the case stands on the board, its underside on z = 0, centred
on the Z axis. Seen from the front, lead 1 (GND, brown) is at -X, lead 2
(VCC, red) in the middle and lead 3 (SIGNAL, orange) at +X. The output
shaft is +Z. The horn is drawn at mid travel, pointing +Y, and the joint
`horn` turns it about -Z with the shaft angle.

Tower Pro's page gives the outline as 23 × 12.2 × 29 mm. The case below is
that footprint. The horn, the tabs and the lead length are a kit servo's
shape (`# verify` where the page has no figure).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin
from cad.geom import box, cyl_z

CASE = (23.0, 12.2, 22.0)  # wide, deep, tall to the top of the case
TABS = (32.0, 12.2, 2.5)  # ears, the long way  # verify
SHAFT = (4.8, 4.0)  # diameter, above the case  # verify
HORN = (4.0, 18.0, 1.6)  # along X, along Y, thick  # verify
LEAD = (0.64, 0.64)
BELOW = 6.5  # verify
PITCH = 2.54

_BLUE = plastic("#1F4C8F")
_HORN = plastic("#F4F4F2")
_TIN = tin()


def servo_sg90_anchors():
    return [
        glb.Anchor(lead, f"lead-{lead}", ((lead - 2) * PITCH, CASE[1] / 2.0, -BELOW))
        for lead in (1, 2, 3)
    ]


def servo_sg90_shape():
    wide, deep, tall = CASE
    children = [
        paint(box(0, 0, tall / 2.0, wide, deep, tall), "case", _BLUE),
    ]
    tab_w, _, tab_t = TABS
    children.append(paint(box(0, 0, tab_t / 2.0, tab_w, deep, tab_t), "tabs", _BLUE))
    shaft_d, shaft_h = SHAFT
    children.append(paint(cyl_z(0, 0, tall, tall + shaft_h, shaft_d / 2.0), "shaft", _HORN))
    horn_x, horn_y, horn_t = HORN
    horn_z = tall + shaft_h
    arm = box(0, horn_y / 4.0, horn_z + horn_t / 2.0, horn_x, horn_y, horn_t)
    hub = cyl_z(0, 0, horn_z, horn_z + horn_t, shaft_d / 2.0 + 0.6)
    children.append(bd.Compound(children=[paint(arm + hub, "arm", _HORN)], label="horn"))
    y = deep / 2.0
    for lead in (1, 2, 3):
        children.append(
            paint(
                tht.square_lead((lead - 2) * PITCH, y, -BELOW, tall * 0.15, *LEAD),
                f"lead-{lead}",
                _TIN,
            )
        )
    return bd.Compound(children=children, label="servo-sg90")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(servo_sg90_shape(), "servo-sg90", servo_sg90_anchors(), Path(sys.argv[1]))
