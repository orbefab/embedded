# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""TI SN74HC595N, PDIP-16 (300 mil).

Part frame: the seating plane (the leads' shoulders) on z = 0, upright,
centred on the origin, the notch at -X. Seen from the top, lead 1 is the
-X end of the -Y row; leads count counter-clockwise, 1 to 8 along -Y
toward +X and 9 to 16 back along +Y (`cad.ic.dip`). Each leg is anchored at
its tip.

The body outline is the datasheet's package table for SN74HC595N, PDIP (16),
19.31 mm by 6.35 mm. The body's thickness and the standoff are estimates
(`# verify`). Rows are 7.62 mm apart on a 2.54 mm pitch, and the legs run
3.3 mm below the seating plane, inside the N package's lead length.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

DIP_BODY = (19.31, 6.35, 3.3)  # long, wide, thick  # verify, thick
DIP_STANDOFF = 0.381  # verify
DIP_BELOW = 3.3


def _parts():
    return ic.dip(16, DIP_BODY, DIP_STANDOFF, below=DIP_BELOW)


def hc595_dip16_shape():
    return bd.Compound(children=_parts()[0], label="hc595-dip16")


def hc595_dip16_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(hc595_dip16_shape(), "hc595-dip16", hc595_dip16_anchors(), Path(sys.argv[1]))
