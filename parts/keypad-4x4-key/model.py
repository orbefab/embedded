# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Single embossed membrane dome keycap inside keypad-4x4.

Part frame: z = 0 sits on the top surface of the keypad-4x4 membrane pad.
The clickable node `cap` is an embossed rectangular dome (12.2 x 8.8 x 0.85 mm)
tinted blue or red per key by `view.tint`, sliding -0.3 mm along -Z when pressed.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box

CAP_W = 12.2
CAP_H = 8.8
CAP_Z = 0.85

_CAP = plastic("#1D65B8", roughness=0.45)
_TIN = tin()


def _cap():
    with bd.BuildSketch() as sketch:
        bd.RectangleRounded(CAP_W, CAP_H, 1.0)
    solid = bd.extrude(sketch.sketch, amount=CAP_Z)
    return bd.fillet(solid.edges().filter_by(lambda e: e.center().Z > CAP_Z - 0.1), 0.35)


def keypad_4x4_key_shape():
    children = [
        paint(_cap(), "cap", _CAP),
        paint(box(-3.0, 0.0, 0.05, 0.6, 0.6, 0.1), "lead-1", _TIN),
        paint(box(3.0, 0.0, 0.05, 0.6, 0.6, 0.1), "lead-2", _TIN),
    ]
    return bd.Compound(children=children, label="keypad_4x4_key")


def keypad_4x4_key_anchors():
    return [
        glb.Anchor(1, "lead-1", (-3.0, 0.0, 0.0), kind="pad"),
        glb.Anchor(2, "lead-2", (3.0, 0.0, 0.0), kind="pad"),
    ]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        keypad_4x4_key_shape(),
        "keypad-4x4-key",
        keypad_4x4_key_anchors(),
        Path(sys.argv[1]),
    )
