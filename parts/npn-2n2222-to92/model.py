# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2N2222A NPN transistor in TO-92 (PN2222A), leads as bought.

Part frame: the body's underside (the seating plane) on z = 0, the body
standing up +Z, its flat face toward -Y with the marking. Leads on 1.27
along X, seen from the flat face from -X: emitter, base, collector.

From onsemi's PN2222A / P2N2222A datasheet: TO-92 (case 29), pins
1 emitter, 2 base, 3 collector from the left seen from the flat face:
leads 1 to 3, `lead-1` to `lead-3`, from -X. The plastic-case pinout; the
metal-can 2N2222 differs.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import srgb
from cad.mark import text

PINS = ("emitter", "base", "collector")  # from -X, seen from the flat face
MARK = ("P2N", "2222A")

_BODY = srgb("#1B1B1D")
_PRINT = srgb("#E8E8E8")
_TIN = srgb("#C5C8CE")


def to92_2n2222_anchors():
    """Each lead's tip, from the same leads the shape draws."""
    _, leads = tht.to92(PINS)
    anchors = []
    for n, lead in enumerate(leads.values(), 1):
        bb = lead.bounding_box()
        anchors.append(glb.Anchor(n, f"lead-{n}", (bb.center().X, bb.center().Y, bb.min.Z)))
    return anchors


def to92_2n2222_shape():
    body, leads = tht.to92(PINS)
    children = [paint(body, "body", _BODY)]
    children += [paint(lead, f"lead-{n}", _TIN) for n, lead in enumerate(leads.values(), 1)]
    y_flat = body.bounding_box().min.Y
    face = bd.Pos(0, y_flat, 0) * bd.Rot(90, 0, 0)
    for k, line in enumerate(MARK):
        children.append(paint(face * text(0, 3.4 - k * 1.2, 0, line, 0.8), f"mark{k + 1}", _PRINT))
    return bd.Compound(children=children, label="to92_2n2222")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(to92_2n2222_shape(), "npn-2n2222-to92", to92_2n2222_anchors(), Path(sys.argv[1]))
