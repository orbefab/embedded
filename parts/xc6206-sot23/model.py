# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Torex XC6206P332MR 3.3 V regulator, SOT-23.

Part frame: on the board's top at z = 0, upright, centred on the origin,
the pin rows along X. Leads 1 (VSS) and 2 (VOUT) sit on the -Y row, 1 at -X.
Lead 3 (VIN) stands alone on the +Y row.

The body is the common SOT-23 outline the catalog's L8050 model takes from
Leshan's section 7: D 2.90, E1 1.30, A2 0.95, A1 midpoint 0.055, E midpoint
2.37, e 0.95, b1 0.40 by c1 0.10. Torex's own package drawing is not read
here (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

SOT_BODY = (2.90, 1.30, 0.95)  # verify against Torex's SOT-23 drawing
SOT_STANDOFF = 0.055
SOT_SPAN = 2.37
SOT_LEAD = (0.40, 0.10)
# lead, x, side. e = 0.95 puts leads 1 and 2 at ±0.475. Lead 3 is centred.
SOT_LEADS = ((1, -0.475, -1), (2, 0.475, -1), (3, 0.0, 1))


def _parts():
    return ic.gullwing(SOT_BODY, SOT_STANDOFF, SOT_SPAN, SOT_LEADS, SOT_LEAD)


def xc6206_sot23_shape():
    return bd.Compound(children=_parts()[0], label="xc6206_sot23")


def xc6206_sot23_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(xc6206_sot23_shape(), "xc6206-sot23", xc6206_sot23_anchors(), Path(sys.argv[1]))
