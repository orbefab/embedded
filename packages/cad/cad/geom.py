# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Axis-aligned boxes and Z cylinders. Boxes and cylinders are centered by build123d."""

import build123d as bd


def box(cx: float, cy: float, cz: float, sx: float, sy: float, sz: float):
    return bd.Pos(cx, cy, cz) * bd.Box(sx, sy, sz)


def cyl_z(cx: float, cy: float, z0: float, z1: float, radius: float):
    height = z1 - z0
    return bd.Pos(cx, cy, z0 + height / 2.0) * bd.Cylinder(radius, height)


def fuse(shapes):
    acc = shapes[0]
    for shape in shapes[1:]:
        acc = acc + shape
    return acc


def formed_foot(cx, cy, cz, sx, sy, sz, *, axis, sign, rise=0.42, tuck=0.28):
    """A flat foot with a gull-wing bend toward the body.

    ``axis`` is ``"x"`` or ``"y"``. ``sign`` is +1 when the body lies in that
    axis's positive direction. The foot is the whole pad, so its underside
    stays where it was. The riser stands on the foot's top, clear of that
    underside, and the shoulder tucks into the body.
    """
    foot = box(cx, cy, cz, sx, sy, sz)
    along = sx if axis == "x" else sy
    bend = min(max(along * 0.42, 0.2), 0.55)
    shift = sign * (along - bend) / 2.0
    rx, ry = (cx + shift, cy) if axis == "x" else (cx, cy + shift)
    top = cz + sz / 2.0
    riser_h = rise + 0.02
    cross = 0.9
    riser = box(
        rx,
        ry,
        top - 0.02 + riser_h / 2.0,
        bend if axis == "x" else sx * cross,
        sy * cross if axis == "x" else bend,
        riser_h,
    )
    sh_t = max(min(sz, rise * 0.55), 0.06)
    tuck_shift = sign * (bend / 2.0 + tuck / 2.0 - 0.02)
    shx, shy = (rx + tuck_shift, ry) if axis == "x" else (rx, ry + tuck_shift)
    shoulder = box(
        shx,
        shy,
        top - 0.02 + riser_h - sh_t / 2.0,
        tuck if axis == "x" else sx * cross,
        sy * cross if axis == "x" else tuck,
        sh_t,
    )
    return fuse([foot, riser, shoulder])
