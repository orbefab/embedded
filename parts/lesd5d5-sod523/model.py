# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Leshan LESD5D5.0CT1G, SOD-523.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 is the end at -X. The sheet's nominal body is
1.20 x 0.80 x 0.60 (D, E, A). HE, the length over the leads, is 1.60.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import plastic

SIZE = (1.20, 0.80, 0.60)  # D, E, A, nominal

_BLACK = plastic("#1A1A1A")


def lesd5d5_sod523_shape():
    return bd.Compound(children=chip(*SIZE, _BLACK), label="lesd5d5_sod523")


def lesd5d5_sod523_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(lesd5d5_sod523_shape(), "lesd5d5-sod523", lesd5d5_sod523_anchors(), Path(sys.argv[1]))
