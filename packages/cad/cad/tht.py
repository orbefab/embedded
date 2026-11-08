# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Through-hole part pieces: leads, bent axial leads, TO-92 and TO-220.

Every part built from these sits on its seating plane: the board's or
breadboard's top on z = 0, leads down -Z.
"""

import build123d as bd

from cad.geom import box, cyl_z, fuse

BREADBOARD_PITCH = 2.54
AXIAL_PITCH = 10.16  # 0.4", the usual bend for a ¼ W resistor or a diode


def round_lead(x, y, z0, z1, d):
    return cyl_z(x, y, z0, z1, d / 2.0)


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


def socket_liner(x, y, z_mouth, opening, depth, wall, down=False):
    """A square socket's wall: a tube `wall` thick round a square `opening`,
    `depth` deep from its mouth at z_mouth, going down -Z (or up +Z when the
    socket opens downward). Cut the body by the tube's outside so the two
    meet without overlapping."""
    zc = z_mouth + depth / 2.0 if down else z_mouth - depth / 2.0
    outer = box(x, y, zc, opening + 2.0 * wall, opening + 2.0 * wall, depth)
    return outer - box(x, y, zc, opening, opening, depth + 0.2)


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


def to92(names, width=4.58, thick=3.6, height=4.58, lead=0.46, pitch=1.27, below=12.7):
    """TO-92: a round body cut flat on its -Y face, `thick` from the flat to
    the back; leads in a row along X on `pitch`, names from -X seen from the
    flat face: (body, {name: lead}). The round's axis is the Z axis."""
    r = width / 2.0
    y_flat = r - thick
    body = cyl_z(0, 0, 0, height, r) & box(
        0, (y_flat + r) / 2.0, height / 2.0, width + 1.0, r - y_flat, height + 1.0
    )
    n = len(names)
    y_lead = (y_flat + r) / 2.0
    leads = {
        name: square_lead((k - (n - 1) / 2.0) * pitch, y_lead, -below, 0.0, lead, lead * 0.85)
        for k, name in enumerate(names)
    }
    return body, leads


def to220(
    names,
    body=(10.0, 4.5, 9.15),
    tab=(10.0, 1.3, 6.5),
    hole=(3.7, 2.8),
    lead=(0.8, 0.5),
    pitch=2.54,
    below=13.0,
):
    """TO-220 standing up: the plastic's face toward -Y, the tab's back flush
    with the body's at +Y, the hole through the tab; leads along X, names from
    -X seen from the face: (body, tab, {name: lead})."""
    bx, by, bh = body
    tx, tt, th = tab
    hd, hole_down = hole
    y_back = by / 2.0
    plastic = box(0, 0, bh / 2.0, bx, by, bh)
    metal = box(0, y_back - tt / 2.0, bh + th / 2.0, tx, tt, th)
    hz = bh + th - hole_down
    metal -= bd.Pos(0, y_back - tt / 2.0, hz) * bd.Rot(90, 0, 0) * bd.Cylinder(hd / 2.0, tt + 1.0)
    lw, lt = lead
    n = len(names)
    leads = {
        name: square_lead((k - (n - 1) / 2.0) * pitch, y_back - tt / 2.0 - 0.6, -below, 0.0, lw, lt)
        for k, name in enumerate(names)
    }
    return plastic, metal, leads
