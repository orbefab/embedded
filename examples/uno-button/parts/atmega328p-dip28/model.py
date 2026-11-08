# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Microchip ATmega328P-PU, PDIP-28 (300 mil): the Uno's microcontroller.

Part frame: the seating plane (the leads' shoulders) on z = 0, upright,
centred on the origin, the notch at -X. Seen from the top, lead 1 is the
-X end of the -Y row; leads count counter-clockwise, 1 to 14 along -Y
toward +X and 15 to 28 back along +Y (`cad.ic.dip`). Each leg is anchored at
its tip.

From the ATmega328P datasheet's 28P3 outline: body 34.544 to 34.798 long,
7.112 to 7.493 wide, 0.508 standoff, legs 0.381 to 0.559 wide on 2.54, rows
7.62 apart, 3.048 to 3.556 below the seating plane. The body's thickness is
an estimate (`# verify`). On the Uno it sits in `dip_socket_28`.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

DIP_BODY = (34.67, 7.3, 3.3)  # long, wide, thick  # verify, thick
DIP_STANDOFF = 0.508
DIP_BELOW = 3.3


def _parts():
    return ic.dip(28, DIP_BODY, DIP_STANDOFF, below=DIP_BELOW)


def atmega328p_dip28_shape():
    return bd.Compound(children=_parts()[0], label="atmega328p_dip28")


def atmega328p_dip28_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        atmega328p_dip28_shape(),
        "atmega328p-dip28",
        atmega328p_dip28_anchors(),
        Path(sys.argv[1]),
    )
