# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Bourns MF-MSMF050-2 resettable fuse (PTC), 1812: the Uno's USB fuse F1.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 is the end at -X, lead 2 the one at +X; the part is
not polarised.

From Bourns' MF-MSMF datasheet, MF-MSMF050: 4.37 to 4.73 long, 3.07 to 3.41
wide, 0.55 to 0.85 tall; drawn at the middle of each range.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import plastic

SIZE = (4.55, 3.24, 0.7)  # length, width, height

_BODY = plastic("#C9A35A")


def ptc_fuse_1812_shape():
    return bd.Compound(children=[*chip(*SIZE, _BODY)], label="ptc_fuse_1812")


def ptc_fuse_1812_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(ptc_fuse_1812_shape(), "ptc-fuse-1812", ptc_fuse_1812_anchors(), Path(sys.argv[1]))
