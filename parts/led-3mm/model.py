# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""3 mm through-hole LED (green, diffused), leads as bought.

Part frame: the flange's underside (the seating plane) on z = 0, the LED
standing up +Z on the Z axis. Leads on 2.54 along X: anode at -X (the
longer one, lead 1), cathode at +X (lead 2), where the flange is cut flat.

From Kingbright's L-934 (T-1) outline: body Φ3.0, about 5.3 tall, leads
0.5 square on 2.54, at least 25.4 long. The flange (Φ3.8 × 0.8) and the
anode drawn 2 longer are estimates (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint, tht
from cad.color import srgb

L3_BODY = (3.0, 3.8, 0.8, 5.3)  # Φ, flange Φ, flange height, overall  # verify, flange
L3_FLAT = 1.6  # the flat's x on the cathode side  # verify
L3_LEAD = 0.5
L3_LEADS = ((1, -1.27, 27.4), (2, 1.27, 25.4))  # lead (anode, cathode), x, below z = 0

_GREEN = srgb("#3FAE49")
_TIN = srgb("#C5C8CE")


def led_3mm_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x, 0.0, -below)) for lead, x, below in L3_LEADS]


def led_3mm_shape():
    d, fd, fh, h = L3_BODY
    children = [paint(tht.led_body(d, fd, fh, h, flat_x=L3_FLAT), "body", _GREEN)]
    for lead, x, below in L3_LEADS:
        children.append(paint(tht.square_lead(x, 0, -below, 0.0, L3_LEAD), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="led_3mm")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(led_3mm_shape(), "led-3mm", led_3mm_anchors(), Path(sys.argv[1]))
