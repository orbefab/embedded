# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""0805 chip inductor or ferrite bead, 2.0 × 1.25: the Uno's L1 (BLM21) and L2 (10 µH).

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 is the end cap at -X, lead 2 the one at +X; the part
is not polarised.

From Murata's BLM21 outline: 2.0 × 1.25 × 0.85. The 10 µH part on the
same 0805 footprint is drawn the same (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import srgb

SIZE = (2.0, 1.25, 0.85)  # length, width, height

_BODY = srgb("#3A3A3C")


def inductor_0805_shape():
    return bd.Compound(children=[*chip(*SIZE, _BODY)], label="inductor_0805")


def inductor_0805_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(inductor_0805_shape(), "inductor-0805", inductor_0805_anchors(), Path(sys.argv[1]))
