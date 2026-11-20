# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""0805 chip LED, 2.0 × 1.25: the Uno's L, TX, RX and ON LEDs.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (the anode) is the end cap at -X, lead 2 (the
cathode) the one at +X, as on the 0603 LED. The body node is the lens, so
a viewer can light and tint it; the shape takes the lens colour.

The common 0805 chip LED: 2.0 × 1.25 × 0.8 (`# verify` height).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import plastic

SIZE = (2.0, 1.25, 0.8)  # length, width, height  # verify height

_LENS = plastic("#E8C56A")


def led_0805_shape(lens=_LENS):
    return bd.Compound(children=chip(*SIZE, lens), label="led_0805")


def led_0805_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(led_0805_shape(), "led-0805", led_0805_anchors(), Path(sys.argv[1]))
