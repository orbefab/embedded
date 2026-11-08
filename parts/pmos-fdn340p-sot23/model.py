# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""onsemi FDN340P P-channel MOSFET, SuperSOT-3 (SOT-23): the Uno's T1.

Part frame: on the board's top at z = 0, upright, centred on the origin,
the pin rows along X. Leads 1 (gate) and 2 (source) sit on the -Y row,
1 at -X; lead 3 (drain) stands alone on the +Y row. The dot marks lead 1.

From onsemi's FDN340P datasheet: SuperSOT-3, pins 1 G, 2 S, 3 D. The body
2.9 × 1.4 × 1.0 and the 2.5 span are the common SOT-23 outline (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

SOT_BODY = (2.9, 1.4, 0.9)  # long, wide, tall above the standoff  # verify
SOT_STANDOFF = 0.1
SOT_SPAN = 2.5  # verify
SOT_PITCH = 0.95
SOT_LEAD = (0.4, 0.13)  # wide, thick
SOT_LEADS = ((1, -1, -1), (2, 1, -1), (3, 0, 1))  # lead, x / pitch, side


def _parts():
    leads = [(n, k * SOT_PITCH, side) for n, k, side in SOT_LEADS]
    return ic.gullwing(SOT_BODY, SOT_STANDOFF, SOT_SPAN, leads, SOT_LEAD)


def fdn340p_sot23_shape():
    return bd.Compound(children=_parts()[0], label="fdn340p_sot23")


def fdn340p_sot23_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        fdn340p_sot23_shape(),
        "pmos-fdn340p-sot23",
        fdn340p_sot23_anchors(),
        Path(sys.argv[1]),
    )
