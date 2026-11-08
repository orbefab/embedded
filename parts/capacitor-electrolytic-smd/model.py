# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Panasonic EEE-1EA470WP, 47 µF 25 V SMD aluminium electrolytic, case D: PC1 and PC2 on the Uno.

Part frame: the base plate's underside on z = 0, the can standing up +Z on
the Z axis. Two flat terminals run out under the base along X: lead 1
(positive) at -X, lead 2 (negative) at +X, under the dark mark on the
can's top, as on the radial electrolytic.

From Panasonic's case D table: can Φ6.3 × 5.4 tall, base 6.6 square,
terminals 0.65 wide reaching 7.2 tip to tip. The base's height and the
terminals' gap are estimates (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box, cyl_z

CAN = (6.3, 5.4)  # Φ, overall height
BASE = (6.6, 1.0)  # square, tall  # verify, tall
TERMINAL = (
    0.65,
    7.2,
    0.9,
    0.1,
)  # wide, tip to tip, inner end from the axis, thick  # verify, inner end

_CAN = srgb("#C9CCD1")
_MARK = srgb("#1B1B1D")
_BASE = srgb("#1A1A1A")
_TIN = srgb("#C5C8CE")


def _terminal_x(lead):
    _, span, inner, _ = TERMINAL
    return (-1 if lead == 1 else 1) * (inner + span / 2.0) / 2.0


def capacitor_electrolytic_smd_shape():
    d, h = CAN
    side, base_h = BASE
    wide, span, inner, thick = TERMINAL
    can = cyl_z(0, 0, base_h, h, d / 2.0)
    mark = box(d / 4.0, 0, h + 0.02, d / 2.0 - 0.6, d * 0.5, 0.04)
    children = [
        paint(box(0, 0, thick + (base_h - thick) / 2.0, side, side, base_h - thick), "base", _BASE),
        paint(can, "can", _CAN),
        paint(mark, "mark", _MARK),
    ]
    for lead in (1, 2):
        long = span / 2.0 - inner
        children.append(paint(box(_terminal_x(lead), 0, thick / 2.0, long, wide, thick), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="capacitor_electrolytic_smd")


def capacitor_electrolytic_smd_anchors():
    return [glb.Anchor(n, f"lead-{n}", (_terminal_x(n), 0.0, 0.0), kind="pad") for n in (1, 2)]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        capacitor_electrolytic_smd_shape(),
        "capacitor-electrolytic-smd",
        capacitor_electrolytic_smd_anchors(),
        Path(sys.argv[1]),
        merge_faces=False,
    )
