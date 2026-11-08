# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Through-hole part pieces: square leads, bent axial leads and a round LED body.

Every part built from these sits on its seating plane: the board's or
breadboard's top on z = 0, leads down -Z.
"""

import build123d as bd

from cad.geom import box, cyl_z, fuse

BREADBOARD_PITCH = 2.54
AXIAL_PITCH = 10.16  # 0.4", the usual bend for a ¼ W resistor or a diode


def square_lead(x, y, z0, z1, w, t=None):
    t = w if t is None else t
    return box(x, y, (z0 + z1) / 2.0, w, t, z1 - z0)


def axial_leads(body_length, body_d, lead_d, pitch=AXIAL_PITCH, below=8.0, lift=0.0):
    """Two leads out of a body lying along X, centred on x = 0, its axis at
    z = body_d / 2 + lift, bent down at ±pitch / 2 to `below` under z = 0."""
    zc = body_d / 2.0 + lift
    r = lead_d / 2.0
    leads = []
    for s in (-1, 1):
        x0, x1 = s * body_length / 2.0, s * pitch / 2.0
        run = abs(x1 - x0) + r
        arm = bd.Pos((x0 + x1 + s * r) / 2.0, 0, zc) * bd.Rot(0, 90, 0) * bd.Cylinder(r, run)
        leg = cyl_z(x1, 0, -below, zc, r)
        leads.append(fuse([arm, leg]).clean())
    return leads


def led_body(d, flange_d, flange_h, height, flat_x=None):
    """A round LED standing on z = 0: flange, barrel and dome, `height` to the
    dome's top; the flange cut flat at x = flat_x (the cathode side) if set."""
    r = d / 2.0
    flange = cyl_z(0, 0, 0, flange_h, flange_d / 2.0)
    if flat_x is not None:
        flange &= box(
            (flat_x - flange_d) / 2.0,
            0,
            flange_h / 2.0,
            flange_d + flat_x,
            flange_d + 1.0,
            flange_h + 1.0,
        )
    barrel = cyl_z(0, 0, flange_h, height - r, r)
    dome = bd.Pos(0, 0, height - r) * bd.Sphere(r)
    dome &= cyl_z(0, 0, height - r, height + 0.1, r + 0.1)
    return fuse([flange, barrel, dome]).clean()
