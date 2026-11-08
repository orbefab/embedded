# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""DIP packages.

Frame: on the board's top at z = 0, upright (the marked top +Z), centred
on the origin. Seen from the top, the pin rows run along X and lead 1 sits
at the -X end of the -Y row. Leads count counter-clockwise, along the -Y
row toward +X and back along the +Y row.

`dip` returns (children, anchors): painted solids, a `body` and a
`lead-<n>` per lead, and the matching `glb.Anchor`s. A leg's anchor is its tip.
"""

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

BLACK = srgb("#1A1A1A")
TIN = srgb("#C5C8CE")
DOT = srgb("#8A8D93")


def dual_row(count, pitch):
    """(lead, x, side) for `count` leads in two rows, counter-clockwise from the -Y row's -X end."""
    half = count // 2
    out = []
    for k in range(half):
        out.append((k + 1, (k - (half - 1) / 2.0) * pitch, -1))
    for k in range(half):
        out.append((half + k + 1, ((half - 1) / 2.0 - k) * pitch, 1))
    return out


def dip(count, body, standoff, row=7.62, pitch=2.54, below=3.3, lead=(0.46, 0.25), shoulder=1.5):
    """A DIP: the body (length, width, height) on its standoff above the
    seating plane z = 0, a notch at -X, legs on `pitch` in two rows `row`
    apart running `below` under z = 0. Leads count like `dual_row`."""
    length, width, height = body
    wide, thick = lead
    top = standoff + height
    children = [paint(box(0, 0, standoff + height / 2.0, length, width, height), "body", BLACK)]
    children.append(paint(box(-length / 2.0 + 0.6, 0, top + 0.02, 1.2, 1.6, 0.04), "notch", DOT))
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
