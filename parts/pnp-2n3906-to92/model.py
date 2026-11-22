# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2N3906 PNP transistor in TO-92, leads formed for a breadboard.

Part frame: the seating plane on z = 0, the body stood off it and up +Z,
its flat face toward -Y with the marking. The leads leave the body on 1.27
and splay to 2.54 along X, seen from the flat face from -X: emitter, base,
collector.

From onsemi's 2N3906 datasheet: TO-92 (case 29, style 1), pins
1 emitter, 2 base, 3 collector from the left seen from the flat face:
leads 1 to 3, `lead-1` to `lead-3`, from -X.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import plastic, tin
from cad.mark import text

PINS = ("emitter", "base", "collector")  # from -X, seen from the flat face
MARK = ("2N", "3906")
FORMED = tht.BREADBOARD_PITCH

_BODY = plastic("#1B1B1D")
_PRINT = plastic("#E8E8E8")
_TIN = tin()


def pnp_2n3906_anchors():
    """Each lead's tip. A formed lead's box centre is not the tip."""
    tips = tht.to92_tips(PINS, formed=FORMED)
    return [glb.Anchor(n, f"lead-{n}", tips[name]) for n, name in enumerate(PINS, 1)]


def pnp_2n3906_shape():
    body, leads = tht.to92(PINS, formed=FORMED, standoff=FORMED)
    children = [paint(body, "body", _BODY)]
    children += [paint(lead, f"lead-{n}", _TIN) for n, lead in enumerate(leads.values(), 1)]
    y_flat = body.bounding_box().min.Y
    z0 = body.bounding_box().min.Z
    face = bd.Pos(0, y_flat, 0) * bd.Rot(90, 0, 0)
    for k, line in enumerate(MARK):
        mark = text(0, z0 + 3.4 - k * 1.2, 0, line, 0.8)
        children.append(paint(face * mark, f"mark{k + 1}", _PRINT))
    return bd.Compound(children=children, label="to92_2n3906")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(pnp_2n3906_shape(), "pnp-2n3906-to92", pnp_2n3906_anchors(), Path(sys.argv[1]))
