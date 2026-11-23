# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""TOPLITE A-5161BS, a 0.56 inch common-anode display. Gray face."""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic
from cad.color import plastic

PART = "display-7seg-ca"
FACE = plastic("#8A8A8A")


def display_7seg_ca_shape():
    return bd.Compound(children=ic.seven_segment(FACE)[0], label=PART.replace("-", "_"))


def display_7seg_ca_anchors():
    return ic.seven_segment(FACE)[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(display_7seg_ca_shape(), PART, display_7seg_ca_anchors(), Path(sys.argv[1]))
