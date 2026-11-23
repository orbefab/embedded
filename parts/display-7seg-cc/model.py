# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""TOPLITE A-5161AS, a 0.56 inch common-cathode display. Black face."""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic
from cad.color import plastic

PART = "display-7seg-cc"
FACE = plastic("#1A1A1A")


def display_7seg_cc_shape():
    return bd.Compound(children=ic.seven_segment(FACE)[0], label=PART.replace("-", "_"))


def display_7seg_cc_anchors():
    return ic.seven_segment(FACE)[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(display_7seg_cc_shape(), PART, display_7seg_cc_anchors(), Path(sys.argv[1]))
