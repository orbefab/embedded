# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Songle SRD-05VDC-SL-C. The body is the sheet's outline, drawn at 19 x 15.5 x 15.5 mm.

Part frame: the seating face is z = 0, the body standing on it, length along X.
The coil pins are lead 1 and lead 2 toward -X. COM, NO and NC are leads 3, 4
and 5 toward +X. The pins are a PCB footprint, so their anchors point up and
the part does not seat in a breadboard.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box

BODY_L = 19.0
BODY_W = 15.5
BODY_H = 15.5
PIN = 0.6
PIN_BELOW = 3.0

_BLUE = plastic("#24558C")
_TIN = tin()
UP = (0.0, 0.0, 1.0)

# Bottom view, looking at the pins: coil pair at -X, contact row at +X.
_PINS = (
    (1, -6.0, -3.8),
    (2, -6.0, 3.8),
    (3, 6.0, 0.0),
    (4, 6.0, 5.0),
    (5, 6.0, -5.0),
)


def relay_srd_05vdc_sl_c_anchors():
    return [glb.Anchor(lead, f"pin-{lead}", (x, y, 0.0), axis=UP) for lead, x, y in _PINS]


def relay_srd_05vdc_sl_c_shape():
    children = [
        paint(box(0, 0, BODY_H / 2.0, BODY_L, BODY_W, BODY_H), "body", _BLUE),
    ]
    for lead, x, y in _PINS:
        children.append(paint(box(x, y, -PIN_BELOW / 2.0, PIN, PIN, PIN_BELOW), f"pin-{lead}", _TIN))
    return bd.Compound(children=children, label="relay_srd_05vdc_sl_c")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        relay_srd_05vdc_sl_c_shape(),
        "relay-srd-05vdc-sl-c",
        relay_srd_05vdc_sl_c_anchors(),
        Path(sys.argv[1]),
    )
