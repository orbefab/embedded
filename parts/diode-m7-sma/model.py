# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""M7 (1N4007) rectifier, SMA (DO-214AC): the Uno's D1, CGRA4007-G in its BOM.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (the cathode, under the band) is the end at -X,
lead 2 (the anode) the one at +X, as on the SOD-123.

From the DO-214AC outline: body 4.06 to 4.57 long, 2.29 to 2.92 wide,
1.91 to 2.41 tall, a cathode band (`# verify` against Comchip's sheet).
The Uno Rev3 board gives it SMB pads.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.chip import CAP_PROUD, cap_length, chip, chip_anchors
from cad.color import srgb
from cad.geom import box

SIZE = (4.3, 2.6, 2.15)  # length, width, height
BAND = 0.5  # wide, beside the cathode cap

_BODY = srgb("#1A1A1A")
_BAND = srgb("#C9CCD1")


def _band():
    length, width, height = SIZE
    x = -length / 2.0 + cap_length(length) + BAND / 2.0 + 0.1
    return paint(box(x, 0, height + CAP_PROUD / 2.0, BAND, width * 0.9, CAP_PROUD), "band", _BAND)


def diode_m7_sma_shape():
    return bd.Compound(children=[*chip(*SIZE, _BODY), _band()], label="diode_m7_sma")


def diode_m7_sma_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(diode_m7_sma_shape(), "diode-m7-sma", diode_m7_sma_anchors(), Path(sys.argv[1]))
