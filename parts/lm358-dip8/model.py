# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""TI LM358P dual op-amp, PDIP-8.

Part frame: the seating plane (the leads' shoulders) on z = 0, upright,
centred on the origin, the notch at -X. Seen from the top, lead 1 is the
-X end of the -Y row; leads count counter-clockwise (`cad.ic.dip`). Each
leg is anchored at its tip.

From TI's P (R-PDIP-T8) outline, drawing 4040082/E: body 9.02 to 10.16 long
and 6.10 to 6.60 wide, 5.08 above the seating plane at most, standoff 0.51
at least, rows 7.62 to 8.26 apart, pitch 2.54, leads 0.38 to 0.53 wide and
0.25 thick, 3.18 below the seating plane at least. The row is the 7.62
minimum, which is the breadboard's channel. The body's thickness is the
5.08 maximum minus the 0.51 minimum standoff (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

DIP_BODY = (9.59, 6.35, 4.57)  # long, wide, thick  # verify, thick
DIP_STANDOFF = 0.51
DIP_BELOW = 3.3  # drawing minimum 3.18


def _parts():
    return ic.dip(
        8,
        DIP_BODY,
        DIP_STANDOFF,
        row=7.62,
        pitch=2.54,
        below=DIP_BELOW,
        lead=(0.46, 0.25),
        shoulder=1.5,
    )


def lm358_dip8_shape():
    return bd.Compound(children=_parts()[0], label="lm358_dip8")


def lm358_dip8_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        lm358_dip8_shape(),
        "lm358-dip8",
        lm358_dip8_anchors(),
        Path(sys.argv[1]),
    )
