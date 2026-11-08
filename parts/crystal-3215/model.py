# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""3.2 × 1.5 two-pad SMD crystal: the 12 MHz crystal beside the Nano's CH340G.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 is the end at -X, lead 2 the one
at +X.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import srgb

SIZE = (3.2, 1.5, 0.7)  # length, width, height  # verify height

_TIN = srgb("#C5C8CE")


def crystal_3215_shape():
    return bd.Compound(children=chip(*SIZE, _TIN), label="crystal_3215")


def crystal_3215_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(crystal_3215_shape(), "crystal-3215", crystal_3215_anchors(), Path(sys.argv[1]), merge_faces=False)
