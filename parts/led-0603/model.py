# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""0603 chip LED, 1.6 × 0.8: the Nano's L, PWR, TX and RX LEDs.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (the anode) is the end cap at -X,
lead 2 (the cathode) the one at +X. The body node is the lens, so a
viewer can light and tint it; the shape takes the lens colour.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import srgb

SIZE = (1.6, 0.8, 0.55)  # length, width, height  # verify height

_LENS = srgb("#E8C56A")


def led_0603_shape(lens=_LENS):
    return bd.Compound(children=chip(*SIZE, lens), label="led_0603")


def led_0603_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(led_0603_shape(), "led-0603", led_0603_anchors(), Path(sys.argv[1]), merge_faces=False)
