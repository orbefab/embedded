# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""SOD-123 Schottky diode: the Nano's diode from USB VBUS to the 5 V rail.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (the cathode) is the end at -X,
lead 2 (the anode) the one at +X.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import srgb

SIZE = (2.6, 1.5, 1.05)  # length, width, height  # verify height

_BLACK = srgb("#1A1A1A")


def diode_sod123_shape():
    return bd.Compound(children=chip(*SIZE, _BLACK), label="diode_sod123")


def diode_sod123_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(diode_sod123_shape(), "diode-sod123", diode_sod123_anchors(), Path(sys.argv[1]), merge_faces=False)
