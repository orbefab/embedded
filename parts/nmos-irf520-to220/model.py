# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""IRF520 N-channel MOSFET in TO-220, leads as bought.

Part frame: the body's underside (the seating plane) on z = 0, standing up
+Z; the marked plastic face toward -Y, the tab's back flush with the
body's at +Y. Leads on 2.54 along X, seen from the face from -X:
gate, drain, source.

From Vishay's IRF520 datasheet: TO-220AB, pins 1 gate, 2 drain, 3 source
from the left seen from the face: leads 1 to 3, `lead-1` to `lead-3`, from
-X. The tab is the drain too, lead 4, anchored on its top edge.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import nickel, plastic, tin
from cad.mark import text

PINS = ("gate", "drain", "source")  # from -X, seen from the face
MARK = ("IRF520",)

_BODY = plastic("#1B1B1D")
_TAB = nickel("#C9CCD1")
_PRINT = plastic("#E8E8E8")
_TIN = tin()


def to220_irf520_anchors():
    """Each lead's tip, from the same leads the shape draws, then the middle of the tab's top edge."""
    _, tab, leads = tht.to220(PINS)
    anchors = []
    for n, lead in enumerate(leads.values(), 1):
        bb = lead.bounding_box()
        anchors.append(glb.Anchor(n, f"lead-{n}", (bb.center().X, bb.center().Y, bb.min.Z)))
    tb = tab.bounding_box()
    anchors.append(glb.Anchor(4, "tab", (tb.center().X, tb.center().Y, tb.max.Z), axis=(0.0, 0.0, 1.0), kind="pad"))
    return anchors


def to220_irf520_shape():
    body, tab, leads = tht.to220(PINS)
    children = [paint(body, "body", _BODY), paint(tab, "tab", _TAB)]
    children += [paint(lead, f"lead-{n}", _TIN) for n, lead in enumerate(leads.values(), 1)]
    y_face = body.bounding_box().min.Y
    face = bd.Pos(0, y_face, 0) * bd.Rot(90, 0, 0)
    for k, line in enumerate(MARK):
        children.append(paint(face * text(0, 6.5 - k * 1.8, 0, line, 1.2), f"mark{k + 1}", _PRINT))
    return bd.Compound(children=children, label="to220_irf520")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(to220_irf520_shape(), "nmos-irf520-to220", to220_irf520_anchors(), Path(sys.argv[1]))
