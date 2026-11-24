# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Packaged ICs: gull-wing rows, QFN and DIP.

Frame: on the board's top at z = 0, upright (the marked top +Z), centred
on the origin. Seen from the top, the pin rows run along X and lead 1 sits
at the -X end of the -Y row. Leads count counter-clockwise, along the -Y
row toward +X and back along the +Y row. A QFN's lead 1 is the +Y end of
its -X edge, counting down that edge, along -Y, up +X and back along +Y.

Each function returns (children, anchors): painted solids, a `body` and a
`lead-<n>` per lead, and the matching `glb.Anchor`s. An SMD lead's anchor
is the middle of its underside on z = 0. A DIP leg's anchor is its tip.
"""

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box, formed_foot

BLACK = plastic("#1A1A1A")
TIN = tin()
DOT = plastic("#8A8D93")


def dual_row(count, pitch):
    """(lead, x, side) for `count` leads in two rows, counter-clockwise from the -Y row's -X end."""
    half = count // 2
    out = []
    for k in range(half):
        out.append((k + 1, (k - (half - 1) / 2.0) * pitch, -1))
    for k in range(half):
        out.append((half + k + 1, ((half - 1) / 2.0 - k) * pitch, 1))
    return out


def gullwing(body, standoff, span, leads, lead=(0.3, 0.15), dot=True):
    """A moulded body (length, width, height above its standoff) with gull-wing
    leads, each from the body's side out to `span` / 2 across the rows.
    `leads` is (lead, x, side) with side -1 for the -Y row. The foot stays
    the flat pad, so the underside centroid does not move."""
    length, width, height = body
    wide, thick = lead
    children = [paint(box(0, 0, standoff + height / 2.0, length, width, height), "body", BLACK)]
    if dot:
        r = min(length, width) * 0.12
        children.append(
            paint(
                box(
                    -length / 2.0 + 2 * r,
                    -width / 2.0 + 2 * r,
                    standoff + height + 0.02,
                    r,
                    r,
                    0.04,
                ),
                "dot",
                DOT,
            )
        )
    inner, outer = width / 2.0, span / 2.0
    anchors = []
    for n, x, side in leads:
        y = side * (inner + outer) / 2.0
        children.append(
            paint(
                formed_foot(x, y, thick / 2.0, wide, outer - inner, thick, axis="y", sign=-side),
                f"lead-{n}",
                TIN,
            )
        )
        anchors.append(glb.Anchor(n, f"lead-{n}", (x, y, 0.0), kind="pad"))
    return children, anchors


def qfn(count, size, height, pitch, terminal=(0.25, 0.4), thick=0.05, exposed=None):
    """A QFN: a square body `size` on edge with `count` terminals, `count` / 4
    an edge, flush with its sides and under it on z = 0. `exposed` is the
    side of a square exposed pad, the last lead."""
    per = count // 4
    wide, deep = terminal
    half = size / 2.0
    children = [paint(box(0, 0, thick + (height - thick) / 2.0, size, size, height - thick), "body", BLACK)]
    r = size * 0.08
    children.append(paint(box(-half + 2 * r, half - 2 * r, height + 0.02, r, r, 0.04), "dot", DOT))
    anchors = []
    for n in range(1, count + 1):
        edge, k = divmod(n - 1, per)
        along = (k - (per - 1) / 2.0) * pitch
        mid = half - deep / 2.0
        x, y = ((-mid, -along), (along, -mid), (mid, along), (-along, mid))[edge]
        sx, sy = (deep, wide) if edge % 2 == 0 else (wide, deep)
        children.append(paint(box(x, y, thick / 2.0, sx, sy, thick), f"lead-{n}", TIN))
        anchors.append(glb.Anchor(n, f"lead-{n}", (x, y, 0.0), kind="pad"))
    if exposed:
        n = count + 1
        children.append(paint(box(0, 0, thick / 2.0, exposed, exposed, thick), f"lead-{n}", TIN))
        anchors.append(glb.Anchor(n, f"lead-{n}", (0.0, 0.0, 0.0), kind="pad"))
    return children, anchors


def dip(
    count,
    body,
    standoff,
    row=7.62,
    pitch=2.54,
    below=3.3,
    lead=(0.46, 0.25),
    shoulder=1.5,
    face=BLACK,
):
    """A DIP: the body (length, width, height) on its standoff above the
    seating plane z = 0, a notch at -X, legs on `pitch` in two rows `row`
    apart running `below` under z = 0. Leads count like `dual_row`."""
    length, width, height = body
    wide, thick = lead
    top = standoff + height
    # A half-round moulded notch at the -X end, in place of a painted mark.
    radius = min(1.0, width * 0.15)
    notch = bd.Pos(-length / 2.0, 0, top) * bd.Rot(90, 0, 0) * bd.Cylinder(radius, width * 0.5)
    body_solid = box(0, 0, standoff + height / 2.0, length, width, height) - notch
    children = [paint(body_solid, "body", face)]
    mid = standoff + height / 2.0
    anchors = []
    y_leg = row / 2.0
    for n, x, side in dual_row(count, pitch):
        y = side * y_leg
        leg = box(x, y, (mid - below) / 2.0, wide, thick, mid + below)
        leg += box(x, y, (standoff + mid) / 2.0, shoulder, thick, mid - standoff)
        children.append(paint(leg, f"lead-{n}", TIN))
        anchors.append(glb.Anchor(n, f"lead-{n}", (x, y, -below)))
    return children, anchors


# TOPLITE A-5161AS / A-5161BS package outline: body 12.6 x 19.0 x 8.0, rows
# 15.24 apart, digit 14.2 tall. The drawing prints a 2.46 mm pin pitch. The
# row span is six times 2.54 mm, so the leads sit on 2.54 mm and enter a
# breadboard. The drawn leg is the 3.3 mm below the seating plane.
DISPLAY_BODY = (12.6, 19.0, 8.0)
DISPLAY_RED = plastic("#C23B2E")


def seven_segment(face):
    """A 10-pin 0.56 inch single-digit display. `face` paints the package.
    Segment bars sit on the top face, A toward +Y and the decimal point at
    the lower right. Returns (children, anchors), the same shape as `dip`."""
    children, anchors = dip(
        10,
        DISPLAY_BODY,
        0.0,
        row=15.24,
        pitch=2.54,
        below=3.3,
        lead=(0.5, 0.5),
        shoulder=1.2,
        face=face,
    )
    height = DISPLAY_BODY[2]
    digit_h, digit_w, stroke = 14.2, 7.2, 1.4
    z = height + 0.22
    thick = 0.44

    def horizontal(name, y):
        return paint(box(0, y, z, digit_w - stroke, stroke, thick), name, DISPLAY_RED)

    def vertical(name, x, y):
        return paint(box(x, y, z, stroke, digit_h / 2 - stroke, thick), name, DISPLAY_RED)

    half_w = digit_w / 2 - stroke / 2
    half_h = digit_h / 2 - stroke / 2
    children.extend(
        [
            horizontal("seg-a", half_h),
            vertical("seg-b", half_w, digit_h / 4),
            vertical("seg-c", half_w, -digit_h / 4),
            horizontal("seg-d", -half_h),
            vertical("seg-e", -half_w, -digit_h / 4),
            vertical("seg-f", -half_w, digit_h / 4),
            horizontal("seg-g", 0),
            paint(box(digit_w / 2 + 0.9, -half_h, z, 1.15, 1.15, thick), "seg-dp", DISPLAY_RED),
        ]
    )
    return children, anchors
