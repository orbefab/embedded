# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""0603 chip resistor, 1.6 × 0.8: the Nano's 1 kΩ and 10 kΩ resistors.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (pin a) is the end cap at -X,
lead 2 (pin b) the one at +X.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import srgb

SIZE = (1.6, 0.8, 0.45)  # length, width, height  # verify height

_BLACK = srgb("#1A1A1A")


def resistor_0603_shape():
    return bd.Compound(children=chip(*SIZE, _BLACK), label="resistor_0603")


def resistor_0603_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(resistor_0603_shape(), "resistor-0603", resistor_0603_anchors(), Path(sys.argv[1]), merge_faces=False)
