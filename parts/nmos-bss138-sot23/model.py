# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""onsemi BSS138 N-channel MOSFET, SOT-23.

Part frame: on the board's top at z = 0, upright, centred on the origin,
the pin rows along X. Leads 1 (gate) and 2 (source) sit on the -Y row,
1 at -X; lead 3 (drain) stands alone on the +Y row. The dot marks lead 1.

onsemi BSS138/D marks pin 1 gate, pin 2 source and pin 3 drain. CASE 318
is 2.90 x 1.30 x 1.00 mm. The body and the lead span below are the
catalog's SOT-23 outline.
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


def bss138_sot23_shape():
    return bd.Compound(children=_parts()[0], label="bss138_sot23")


def bss138_sot23_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        bss138_sot23_shape(),
        "nmos-bss138-sot23",
        bss138_sot23_anchors(),
        Path(sys.argv[1]),
    )
