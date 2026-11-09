# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""¼ W carbon-film resistor, 10 kΩ, leads bent for a breadboard.

Part frame: the seating plane (the board's top) on z = 0, the body lying
on it along X, centred on the origin; the leads bend down at x = ±5.08
(0.4") and run 8 below z = 0, lead 1 at -X. Bands brown, black, orange, gold (10 kΩ,
5 %) read from -X.

From Yageo's CFR-25 outline: body about 6.3 long and 2.4 across at the
end caps, leads Φ0.54 (`# verify` for the neck and band widths).
"""

import itertools
import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import srgb

RQ_LENGTH = 6.3
RQ_CAP_D = 2.4
RQ_NECK_D = 2.15  # verify
RQ_LEAD_D = 0.54
RQ_BELOW = 8.0
RQ_BANDS = (  # x0, x1, colour, label
    (-2.75, -2.3, "#7A4A21", "band1-brown"),
    (-1.85, -1.4, "#151515", "band2-black"),
    (-1.0, -0.55, "#E67E22", "band3-orange"),
    (2.1, 2.55, "#B8902E", "band4-gold"),
)

_BODY = srgb("#D9C49A")
_TIN = srgb("#C5C8CE")


def _diameter(x0, x1):
    return RQ_CAP_D if (x1 <= -1.9 or x0 >= 1.9) else RQ_NECK_D


def _segment(x0, x1):
    d = _diameter(x0, x1)
    return bd.Pos((x0 + x1) / 2.0, 0, RQ_CAP_D / 2.0) * bd.Rot(0, 90, 0) * bd.Cylinder(d / 2.0, x1 - x0)


def _body():
    edges = [-RQ_LENGTH / 2.0, -1.9, 1.9, RQ_LENGTH / 2.0]
    for x0, x1, _, _ in RQ_BANDS:
        edges += [x0, x1]
    edges = sorted(set(edges))
    bands = {(x0, x1): (color, label) for x0, x1, color, label in RQ_BANDS}
    parts, k = [], 0
    for x0, x1 in itertools.pairwise(edges):
        if (x0, x1) in bands:
            color, label = bands[(x0, x1)]
            parts.append(paint(_segment(x0, x1), label, srgb(color)))
        else:
            k += 1
            parts.append(paint(_segment(x0, x1), f"body{k}", _BODY))
    return parts


def resistor_qw_anchors():
    return [
        glb.Anchor(lead, f"lead-{lead}", (side * tht.AXIAL_PITCH / 2.0, 0.0, -RQ_BELOW))
        for lead, side in ((1, -1), (2, 1))
    ]


def resistor_qw_shape():
    leads = tht.axial_leads(RQ_LENGTH, RQ_CAP_D, RQ_LEAD_D, below=RQ_BELOW)
    children = [*_body(), *(paint(lead, f"lead-{k + 1}", _TIN) for k, lead in enumerate(leads))]
    return bd.Compound(children=children, label="resistor_qw")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(resistor_qw_shape(), "resistor-axial", resistor_qw_anchors(), Path(sys.argv[1]))
