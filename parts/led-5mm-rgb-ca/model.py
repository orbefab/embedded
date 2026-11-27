# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""5 mm 4-pin common-anode RGB LED (Kingbright WP154A4SUREQBFZGW/CA), leads formed for a breadboard.

Same T-1 3/4 4-pin package outline as WP154A4SUREQBFZGW (Kingbright DSAK0727
Rev V.4A p1): body dia 5.0 mm, flange dia 5.9 mm x 1.0 mm, overall body height
8.6 mm, flat on the Pin 1 (Red) side, Pin 2 longest (Common Anode), Pin 3 Blue,
Pin 4 Green. Leads emerge on 1.27 mm and form to 2.54 mm breadboard pitch above
z = 0, stood off by 3.5 mm.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin
from cad.geom import box, cyl_z, fuse

BODY_D = 5.0
FLANGE_D = 5.9
FLANGE_H = 1.0
BODY_H = 8.6
STANDOFF = 3.5
JOG_Z = 1.0
LEAD_W = 0.5
BODY_PITCH = 1.27
FORMED_PITCH = tht.BREADBOARD_PITCH

LEADS = (
    (1, -1.5 * BODY_PITCH, -1.5 * FORMED_PITCH, 8.0),  # R (flat edge side)
    (2, -0.5 * BODY_PITCH, -0.5 * FORMED_PITCH, 10.5),  # COM (longest, +2.5 mm)
    (3, 0.5 * BODY_PITCH, 0.5 * FORMED_PITCH, 8.0),  # B
    (4, 1.5 * BODY_PITCH, 1.5 * FORMED_PITCH, 8.0),  # G
)

_DOME = plastic("#F0F2F5", alpha=0.42, roughness=0.35)
_DIE_R = plastic("#E82626")
_DIE_G = plastic("#1ED43A")
_DIE_B = plastic("#2468FF")
_TIN = tin()


def _formed_lead(x_body, x_tip, z_top, z_tip, w):
    stub = tht.square_lead(x_body, 0.0, JOG_Z, z_top, w)
    span = abs(x_tip - x_body) + w
    jog = box((x_body + x_tip) / 2.0, 0.0, JOG_Z, span, w, w)
    leg = tht.square_lead(x_tip, 0.0, z_tip, JOG_Z, w)
    return fuse([stub, jog, leg]).clean()


def _dome():
    r = BODY_D / 2.0
    flange = cyl_z(0, 0, 0, FLANGE_H, FLANGE_D / 2.0)
    flat_x = -BODY_D / 2.0
    flange &= box(
        (FLANGE_D + flat_x) / 2.0,
        0,
        FLANGE_H / 2.0,
        FLANGE_D - flat_x,
        FLANGE_D + 1.0,
        FLANGE_H + 1.0,
    )
    barrel = cyl_z(0, 0, FLANGE_H, BODY_H - r, r)
    top = bd.Pos(0, 0, BODY_H - r) * bd.Sphere(r)
    top &= cyl_z(0, 0, BODY_H - r, BODY_H + 0.1, r + 0.1)
    return bd.Pos(0, 0, STANDOFF) * fuse([flange, barrel, top]).clean()


def rgb_led_ca_shape():
    children = [
        paint(_dome(), "body", _DOME),
        paint(box(0, 0, STANDOFF + 2.0, 3.0, 1.4, 0.4), "frame", _TIN),
        paint(cyl_z(-0.95, 0.0, STANDOFF + 2.2, STANDOFF + 5.2, 0.58), "die-red", _DIE_R),
        paint(cyl_z(0.0, 0.55, STANDOFF + 2.2, STANDOFF + 5.2, 0.58), "die-green", _DIE_G),
        paint(cyl_z(0.95, -0.25, STANDOFF + 2.2, STANDOFF + 5.2, 0.58), "die-blue", _DIE_B),
    ]
    for lead, x_body, x_tip, below in LEADS:
        children.append(paint(_formed_lead(x_body, x_tip, STANDOFF + 1.8, -below, LEAD_W), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="led_5mm_rgb_ca")


def rgb_led_ca_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x_tip, 0.0, -below)) for lead, _, x_tip, below in LEADS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(rgb_led_ca_shape(), "led-5mm-rgb-ca", rgb_led_ca_anchors(), Path(sys.argv[1]))
