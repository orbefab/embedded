# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Espressif ESP32-S3-WROOM-1-N16R8.

Part frame: on the board's top at z = 0, centred on the origin, the antenna
toward +Y. Leads are the castellations and the ground pad.

The body is Table 1-1 of the ESP32-S3-WROOM-1 & WROOM-1U Datasheet v1.8:
18.0 x 25.5 x 3.1 mm. Pad places are Figure 11-1, the recommended land
pattern. Pin 1 is at the antenna end of the -X edge and pin 40 is opposite
it. The pin-1 row is 7.49 mm from the antenna end. Fourteen pins run down
each long edge on a 1.27 mm pitch, a span of 16.51 mm, which leaves the
last of them 1.5 mm from the far end (25.5 - 7.49 - 16.51). Twelve pins
run along the far end, the outer ones 2.015 mm in from each long edge, on
the same 1.27 mm pitch. Figure 11-1 spaces the long-edge centres 17.5 mm
apart on the 18.0 mm body, so each centre is 0.25 mm inside the outline.
The 0.5 printed beside the 1.5 mm pad is the overhang past the body
(1.5 / 2 - 0.25). The same 0.25 mm inset on the short edge keeps both the
7.49 mm and the 2.015 mm. The pads are 1.5 by 0.9. Lead 41 is the ground
pad. Figure 11-1 draws a thermal grid (3.7, 7.5, 10.29, 0.9) rather than
one rectangle, so lead 41 is a marker at the origin, not a measured land.
The sheet does not name a body colour.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

BODY_X = 18.0
BODY_Y = 25.5
BODY_Z = 3.1
PAD_ALONG = 0.9
PAD_DEEP = 1.5
PAD_T = 0.05
PITCH = 1.27
ROW_FROM_ANTENNA = 7.49
END_INSET = 2.015
# Figure 11-1: (18.0 - 17.5) / 2. The DevKit DXF lands sit on this inset.
EDGE_INSET = 0.25
TIN = srgb("#C5C8CE")
CAN = srgb("#1A1A1A")


def _pads():
    y1 = BODY_Y / 2.0 - ROW_FROM_ANTENNA
    x_side = BODY_X / 2.0 - EDGE_INSET
    y_end = -(BODY_Y / 2.0 - EDGE_INSET)
    pads = []
    for index in range(14):
        y = y1 - index * PITCH
        pads.append((-x_side, y, PAD_DEEP, PAD_ALONG))
    x15 = -BODY_X / 2.0 + END_INSET
    for index in range(12):
        pads.append((x15 + index * PITCH, y_end, PAD_ALONG, PAD_DEEP))
    y27 = y1 - 13 * PITCH
    for index in range(14):
        pads.append((x_side, y27 + index * PITCH, PAD_DEEP, PAD_ALONG))
    pads.append((0.0, 0.0, PAD_ALONG, PAD_ALONG))
    return pads


def _parts():
    # The land drawing gives no copper thickness. 0.05 mm is the QFN helper's
    # terminal thickness, so each pad has a volume. It is not a module measurement.
    body = paint(box(0, 0, BODY_Z / 2.0, BODY_X, BODY_Y, BODY_Z), "body", CAN)
    children = [body]
    anchors = []
    for lead, (x, y, sx, sy) in enumerate(_pads(), start=1):
        children.append(paint(box(x, y, PAD_T / 2.0, sx, sy, PAD_T), f"lead-{lead}", TIN))
        anchors.append(glb.Anchor(lead, f"lead-{lead}", (x, y, 0.0), kind="pad"))
    return children, anchors


def esp32s3_wroom_1_n16r8_shape():
    return bd.Compound(children=_parts()[0], label="esp32s3_wroom_1_n16r8")


def esp32s3_wroom_1_n16r8_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        esp32s3_wroom_1_n16r8_shape(),
        "esp32s3-wroom-1-n16r8",
        esp32s3_wroom_1_n16r8_anchors(),
        Path(sys.argv[1]),
    )
