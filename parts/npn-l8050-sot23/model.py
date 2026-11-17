# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Leshan L8050QLT1G NPN, SOT-23.

Part frame: on the board's top at z = 0, upright, centred on the origin,
the pin rows along X. Leads 1 (base) and 2 (emitter) sit on the -Y row,
1 at -X. Lead 3 (collector) stands alone on the +Y row. The dot marks lead 1.

From Leshan Radio L8050QLT1G Rev.D Jun. 2023, section 7. Body D 2.90, E1 1.30
and A2 0.95 are the nominals. The standoff is the midpoint of A1, 0.01 to
0.10. E has no nominal (2.10 to 2.64), so the span is that midpoint, 2.37,
tip to tip. Leads 1 and 2 are e = 0.95 apart. The lead is b1 0.40 by c1 0.10,
both nominal.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

SOT_BODY = (2.90, 1.30, 0.95)  # D, E1, A2, nominal
SOT_STANDOFF = 0.055  # midpoint of A1, 0.01 to 0.10
SOT_SPAN = 2.37  # midpoint of E, 2.10 to 2.64
SOT_LEAD = (0.40, 0.10)  # b1 and c1, nominal
# lead, x, side. e = 0.95 puts pins 1 and 2 at ±0.475. Pin 3 is centred.
SOT_LEADS = ((1, -0.475, -1), (2, 0.475, -1), (3, 0.0, 1))


def _parts():
    return ic.gullwing(SOT_BODY, SOT_STANDOFF, SOT_SPAN, SOT_LEADS, SOT_LEAD)


def npn_l8050_sot23_shape():
    return bd.Compound(children=_parts()[0], label="npn_l8050_sot23")


def npn_l8050_sot23_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        npn_l8050_sot23_shape(),
        "npn-l8050-sot23",
        npn_l8050_sot23_anchors(),
        Path(sys.argv[1]),
    )
