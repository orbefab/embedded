# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""TI LP2985-33DBVR 3.3 V regulator, SOT-23-5 (DBV): the Uno's 3.3 V supply.

Part frame: on the board's top at z = 0, upright, centred on the origin,
the pin rows along X. Leads 1 (IN), 2 (GND) and 3 (ON/OFF) run along the -Y
row from -X; 4 (BYPASS) and 5 (OUT) sit on the +Y row, 4 at +X. The dot
marks lead 1.

From TI's DBV outline: body 2.9 × 1.6, 1.45 tall at most, leads 0.3–0.5
wide on 0.95, 2.8 tip to tip.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

SOT_BODY = (2.9, 1.6, 1.1)  # long, wide, tall above the standoff
SOT_STANDOFF = 0.1
SOT_SPAN = 2.8
SOT_PITCH = 0.95
SOT_LEAD = (0.4, 0.13)  # wide, thick
SOT_LEADS = ((1, -1, -1), (2, 0, -1), (3, 1, -1), (4, 1, 1), (5, -1, 1))  # lead, x / pitch, side


def _parts():
    leads = [(n, k * SOT_PITCH, side) for n, k, side in SOT_LEADS]
    return ic.gullwing(SOT_BODY, SOT_STANDOFF, SOT_SPAN, leads, SOT_LEAD)


def lp2985_sot23_5_shape():
    return bd.Compound(children=_parts()[0], label="lp2985_sot23_5")


def lp2985_sot23_5_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        lp2985_sot23_5_shape(),
        "lp2985-sot23-5",
        lp2985_sot23_5_anchors(),
        Path(sys.argv[1]),
        merge_faces=False,
    )
