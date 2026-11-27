# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Adafruit NeoPixel Ring 16 (product 1463): the bare ring PCB with a 4-pin right-angle header.

Part frame: the ring lies flat on z = 0, centred on the origin, LEDs facing +Z.
From Adafruit NeoPixel Ring 16 rev C Eagle board: outer radius 22.225 mm
(44.45 mm OD), inner radius 15.875 mm (31.75 mm ID), PCB thickness 1.6 mm.
A small tab at -X carries a 1x4 right-angle 2.54 mm pin header whose pins point
along -X at z = 2.87 mm (GND, 5V, DIN, DOUT from +Y to -Y), clear of all 16
5050 LEDs so jumpers run flat along the bench to a breadboard.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box, cyl_z, fuse

OUTER_R = 22.225  # Eagle rev C layer 20 circle radius (44.45 mm OD)
INNER_R = 15.875  # Eagle rev C layer 20 circle radius (31.75 mm ID)
PCB_T = 1.6
PITCH = 2.54
PIN_W = 0.64
HDR_X_OUTER = -26.0
HDR_W = 2.54
HDR_Z0 = PCB_T
PIN_Z = HDR_Z0 + HDR_W / 2.0
PIN_TIP_X = HDR_X_OUTER - 6.0  # 6 mm proud of housing
PIN_BEND_X = HDR_X_OUTER + HDR_W + 0.8  # -22.66 mm, cleanly outside LED5 (-21.62 mm)

_PCB = plastic("#141414")
_HOUSING = plastic("#1A1A1A")
_TIN = tin()


def _pin_y(lead):
    # Lead 1 (+Y) = GND, Lead 2 = 5V, Lead 3 = DIN, Lead 4 (-Y) = DOUT
    return (2.5 - lead) * PITCH


def neopixel_ring_16_anchors():
    return [
        glb.Anchor(
            lead,
            f"pin-{lead}",
            (PIN_TIP_X, _pin_y(lead), PIN_Z),
            axis=(-1.0, 0.0, 0.0),
            also=(f"housing-{lead}",),
        )
        for lead in range(1, 5)
    ]


def _board():
    ring = cyl_z(0, 0, 0, PCB_T, OUTER_R) - cyl_z(0, 0, -0.2, PCB_T + 0.2, INNER_R)
    tab = box((HDR_X_OUTER + (-19.0)) / 2.0, 0, PCB_T / 2.0, abs(HDR_X_OUTER - (-19.0)), 4 * PITCH + 1.6, PCB_T)
    return fuse([ring, tab]).clean()


def _ra_pin(y):
    back = PIN_BEND_X + PIN_W / 2.0
    horiz = box((PIN_TIP_X + back) / 2.0, y, PIN_Z, back - PIN_TIP_X, PIN_W, PIN_W)
    vert = box(PIN_BEND_X, y, PIN_Z / 2.0, PIN_W, PIN_W, PIN_Z)
    return fuse([horiz, vert]).clean()


def neopixel_ring_16_shape():
    children = [paint(_board(), "pcb", _PCB)]
    for lead in range(1, 5):
        y = _pin_y(lead)
        children.append(
            paint(box(HDR_X_OUTER + HDR_W / 2.0, y, PIN_Z, HDR_W, PITCH, HDR_W), f"housing-{lead}", _HOUSING)
        )
        children.append(paint(_ra_pin(y), f"pin-{lead}", _TIN))
    return bd.Compound(children=children, label="neopixel_ring_16")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(neopixel_ring_16_shape(), "neopixel-ring-16", neopixel_ring_16_anchors(), Path(sys.argv[1]))
