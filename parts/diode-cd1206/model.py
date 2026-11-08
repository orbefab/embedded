# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Bourns CD1206-S01575 switching diode, 1206: the Uno's D2 and D3.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (the cathode, under the band) is the end at -X,
lead 2 (the anode) the one at +X, as on the SOD-123.

From Bourns' CD1206-S01575 datasheet: 3.00 to 3.40 long, 1.30 to 1.70 wide,
0.75 to 0.95 tall, a cathode band. The Uno Rev3 board gives it MiniMELF
pads.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.chip import CAP_PROUD, cap_length, chip, chip_anchors
from cad.color import srgb
from cad.geom import box

SIZE = (3.2, 1.5, 0.85)  # length, width, height
BAND = 0.5  # wide, beside the cathode cap

_BODY = srgb("#1A1A1A")
_BAND = srgb("#C9CCD1")


def _band():
    length, width, height = SIZE
    x = -length / 2.0 + cap_length(length) + BAND / 2.0 + 0.1
    return paint(box(x, 0, height + CAP_PROUD / 2.0, BAND, width * 0.9, CAP_PROUD), "band", _BAND)


def diode_cd1206_shape():
    return bd.Compound(children=[*chip(*SIZE, _BODY), _band()], label="diode_cd1206")


def diode_cd1206_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(diode_cd1206_shape(), "diode-cd1206", diode_cd1206_anchors(), Path(sys.argv[1]), merge_faces=False)
