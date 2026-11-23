# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Songle SRD-05VDC-SL-C. The body is the sheet's outline, drawn at 19 x 15.5 x 15.5 mm.

Part frame: the seating face is z = 0, the body standing on it, length along X.
The drilling drawing, read from above: COM (lead 1) stands alone at -X. The
coil leads are 2 mm toward +X, at +6 and -6 mm (leads 2 and 5). NO and NC are
12.2 mm further, at +6 and -6 mm (leads 3 and 4). Lead 4 meets lead 1 with the
coil off, so it is NC. The pins are a PCB footprint, so their anchors point up
and the part does not seat in a breadboard.
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

# COM is 1 mm inside the -X end of the 19 mm body. The sheet's rows then follow.
_COM_X = -8.5
_COIL_X = _COM_X + 2.0
_CONTACT_X = _COIL_X + 12.2
_PINS = (
    (1, _COM_X, 0.0),
    (2, _COIL_X, 6.0),
    (3, _CONTACT_X, 6.0),
    (4, _CONTACT_X, -6.0),
    (5, _COIL_X, -6.0),
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
