# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""5 mm through-hole LED (red, diffused), leads as bought.

Part frame: the flange's underside (the seating plane) on z = 0, the LED
standing up +Z on the Z axis. Leads on 2.54 along X: anode at -X (the
longer one, lead 1), cathode at +X (lead 2), where the flange is cut flat.

From Kingbright's L-7113 (T-1¾) outline: body Φ5.0, flange Φ5.8 × 1.0,
8.6 tall, leads 0.5 square on 2.54, at least 25.4 long (the anode drawn
2 longer, `# verify`).
"""

from pathlib import Path

import build123d as bd
from cad.color import srgb

from cad import glb, tht

L5_BODY = (5.0, 5.8, 1.0, 8.6)  # Φ, flange Φ, flange height, overall
L5_FLAT = 2.5  # the flat's x on the cathode side  # verify
L5_LEAD = 0.5
L5_LEADS = ((1, -1.27, 27.4), (2, 1.27, 25.4))  # lead (anode, cathode), x, below z = 0

_RED = srgb("#D8342A")
_TIN = srgb("#C5C8CE")


def _paint(shape, label, color):
    shape.label = label
    shape.color = color
    return shape


def led_5mm_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x, 0.0, -below)) for lead, x, below in L5_LEADS]


def led_5mm_shape():
    d, fd, fh, h = L5_BODY
    children = [_paint(tht.led_body(d, fd, fh, h, flat_x=L5_FLAT), "body", _RED)]
    for lead, x, below in L5_LEADS:
        children.append(_paint(tht.square_lead(x, 0, -below, 0.0, L5_LEAD), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="led_5mm")


if __name__ == "__main__":
    glb.write(led_5mm_shape(), "led-5mm", led_5mm_anchors(), Path(__file__).resolve().parent)
