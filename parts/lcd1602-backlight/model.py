# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""The LED backlight of a 1602 character LCD module, between pins A and K.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (the anode, A) is the end cap at -X, lead 2 (the
cathode, K) the one at +X. The module's LEDs sit under its bezel and light
the glass from behind; this one chip LED stands in for them there, out of
sight, as the electrical part.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip_anchors, led
from cad.color import plastic

SIZE = (1.6, 0.8, 0.55)  # length, width, height  # verify, the stand-in's size

_LENS = plastic("#F4F6FA")


def lcd1602_backlight_shape():
    return bd.Compound(children=led(*SIZE, _LENS), label="lcd1602_backlight")


def lcd1602_backlight_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(lcd1602_backlight_shape(), "lcd1602-backlight", lcd1602_backlight_anchors(), Path(sys.argv[1]))
