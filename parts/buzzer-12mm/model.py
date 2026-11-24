# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Same Sky CEM-1203(42), 12 mm magnetic buzzer transducer.

Part frame: the seating plane on z = 0, the body standing on it, pins down.
Pin 1 is at -X and pin 2 at +X, 6.5 mm apart. The top face has the sound hole.

Same Sky CEM-1203(42) rev 1.05: body Ø12.0 x 8.5, pins Ø0.5 on 6.5.
The lead length below the seat is the breadboard reach, not a sheet figure.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import gold, plastic
from cad.geom import cyl_z

BODY_D = 12.0
BODY_H = 8.5
PIN_D = 0.5
PIN_SPAN = 6.5  # centre to centre, the drawing's 6.5
PIN_BELOW = 8.0
HOLE_D = 2.8
HOLE_DEPTH = 1.2

_BODY = plastic("#1A1A1A")
_GOLD = gold()


def buzzer_12mm_anchors():
    half = PIN_SPAN / 2.0
    return [glb.Anchor(lead, f"lead-{lead}", (sign * half, 0.0, -PIN_BELOW)) for lead, sign in ((1, -1), (2, 1))]


def buzzer_12mm_shape():
    half = PIN_SPAN / 2.0
    body = cyl_z(0, 0, 0, BODY_H, BODY_D / 2.0)
    hole = cyl_z(0, 0, BODY_H - HOLE_DEPTH, BODY_H + 0.2, HOLE_D / 2.0)
    children = [paint(body - hole, "body", _BODY)]
    for lead, sign in ((1, -1), (2, 1)):
        wire = cyl_z(sign * half, 0, -PIN_BELOW, 0.4, PIN_D / 2.0)
        children.append(paint(wire, f"lead-{lead}", _GOLD))
    return bd.Compound(children=children, label="buzzer_12mm")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(buzzer_12mm_shape(), "buzzer-12mm", buzzer_12mm_anchors(), Path(sys.argv[1]))
