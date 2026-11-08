# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Murata CSTCE 16 MHz ceramic resonator, three terminals: the ATmega328P's clock.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (IN) is the end at -X, lead 2 (GND) the band across
the middle, lead 3 (OUT) the end at +X.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.chip import CAP_PROUD, TIN, cap_length
from cad.color import srgb
from cad.geom import box

CSTCE_SIZE = (3.2, 1.5, 0.7)  # length, width, height  # verify

_GOLD = srgb("#C6A15A")


def _terminal_x():
    length = CSTCE_SIZE[0]
    offset = length / 2.0 - cap_length(length) / 2.0
    return ((1, -offset), (2, 0.0), (3, offset))


def resonator_cstce_shape():
    length, width, height = CSTCE_SIZE
    cap = cap_length(length)
    tall = height + CAP_PROUD
    parts = [paint(box(0, 0, height / 2.0, length, width, height), "body", _GOLD)]
    for lead, x in _terminal_x():
        parts.append(paint(box(x, 0, tall / 2.0, cap, width * 0.92, tall), f"lead-{lead}", TIN))
    return bd.Compound(children=parts, label="resonator_cstce")


def resonator_cstce_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", (x, 0.0, 0.0), kind="pad") for lead, x in _terminal_x()]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(resonator_cstce_shape(), "resonator-cstce", resonator_cstce_anchors(), Path(sys.argv[1]), merge_faces=False)
