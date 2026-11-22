# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2N2222A NPN transistor in TO-92 (P2N2222A), leads formed for a breadboard.

Part frame: the seating plane on z = 0, the body stood off it and up +Z,
its flat face toward -Y with the marking. The leads leave the body on 1.27
and splay to 2.54 along X, seen from the flat face from -X: collector, base,
emitter.

From onsemi's P2N2222A datasheet: TO-92 (case 29, style 17), pins
1 collector, 2 base, 3 emitter from the left seen from the flat face:
leads 1 to 3, `lead-1` to `lead-3`, from -X. A PN2222A has them the other
way round. The metal-can 2N2222 differs from both.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin
from cad.mark import text

PINS = ("collector", "base", "emitter")  # from -X, seen from the flat face
MARK = ("P2N", "2222A")
FORMED = tht.BREADBOARD_PITCH

_BODY = plastic("#1B1B1D")
_PRINT = plastic("#E8E8E8")
_TIN = tin()


def to92_2n2222_anchors():
    """Each lead's tip. A formed lead's box centre is not the tip."""
    tips = tht.to92_tips(PINS, formed=FORMED)
    return [glb.Anchor(n, f"lead-{n}", tips[name]) for n, name in enumerate(PINS, 1)]


def to92_2n2222_shape():
    body, leads = tht.to92(PINS, formed=FORMED, standoff=FORMED)
    children = [paint(body, "body", _BODY)]
    children += [paint(lead, f"lead-{n}", _TIN) for n, lead in enumerate(leads.values(), 1)]
    y_flat = body.bounding_box().min.Y
    z0 = body.bounding_box().min.Z
    face = bd.Pos(0, y_flat, 0) * bd.Rot(90, 0, 0)
    for k, line in enumerate(MARK):
        children.append(paint(face * text(0, z0 + 3.4 - k * 1.2, 0, line, 0.8), f"mark{k + 1}", _PRINT))
    return bd.Compound(children=children, label="to92_2n2222")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(to92_2n2222_shape(), "npn-2n2222-to92", to92_2n2222_anchors(), Path(sys.argv[1]))
