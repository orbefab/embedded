# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Surface-mount tactile switch, five pads: RESET on the Uno.

Part frame: on the board's top at z = 0, centred on the footprint, the
long side along X, the button up +Z. Leads follow the footprint's pads:
1 at (-X, +Y), 2 at (+X, +Y), 3 at (-X, -Y), 4 at (+X, -Y), and 5 in the
middle of the -X side. Leads 1 and 2 are joined, and so are 3 and 4. Pressing joins the
pairs. The button is its own node, `button`, and presses 0.2 down -Z.

The body, 5.95 × 4.4 from the footprint's outline, and its height and
button are estimates (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import nickel, plastic, tin
from cad.geom import box, cyl_z

BODY = (5.95, 4.4, 1.4, -0.125)  # long, wide, tall, x of its centre
COVER = 0.3  # the metal top's thickness  # verify
BUTTON = (2.6, 1.0)  # diameter, above the cover  # verify
TRAVEL = 0.2  # verify
PADS = {1: (-4.2, 2.25), 2: (4.2, 2.25), 3: (-4.2, -2.25), 4: (4.2, -2.25), 5: (-4.2, 0.0)}
TAB = (0.8, 0.15)  # wide, thick

_BLACK = plastic("#1A1A1A")
_COVER = nickel("#C9CCD1")
_TIN = tin()


def _tab(lead):
    x, y = PADS[lead]
    length, _, _, cx = BODY
    inner = cx + (length / 2.0 if x > 0 else -length / 2.0)
    outer = x + (0.4 if x > 0 else -0.4)
    return (inner + outer) / 2.0, y, abs(outer - inner)


def tactile_switch_ts42_shape():
    length, width, tall, cx = BODY
    diameter, above = BUTTON
    top = tall + COVER
    pocket = cyl_z(cx, 0, tall - TRAVEL - 0.05, top + 0.1, diameter / 2.0 + 0.1)
    children = [
        paint(box(cx, 0, tall / 2.0, length, width, tall) - pocket, "body", _BLACK),
        paint(box(cx, 0, tall + COVER / 2.0, length, width, COVER) - pocket, "cover", _COVER),
        paint(cyl_z(cx, 0, tall - 0.05, top + above, diameter / 2.0), "button", _BLACK),
    ]
    wide, thick = TAB
    for lead in PADS:
        x, y, run = _tab(lead)
        children.append(paint(box(x, y, thick / 2.0, run, wide, thick), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="tactile_switch_ts42")


def tactile_switch_ts42_anchors():
    return [glb.Anchor(n, f"lead-{n}", (_tab(n)[0], _tab(n)[1], 0.0), kind="pad") for n in PADS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        tactile_switch_ts42_shape(),
        "tactile-switch-ts42",
        tactile_switch_ts42_anchors(),
        Path(sys.argv[1]),
    )
