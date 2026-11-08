# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""TI LMV358IDGKR dual op-amp, VSSOP-8 (MSOP-8): the Uno's U5.

The Uno Rev3 schematic and board both name the LMV358IDGKR, in TI's DGK
package. The comparator is often called the Uno's LM358.

Part frame: on the board's top at z = 0, upright, centred on the origin,
the pin rows along X. Leads count counter-clockwise from the top: 1 to 4
along the -Y row from -X, 5 to 8 back along +Y (`cad.ic.dual_row`). The
dot marks lead 1.

From TI's DGK outline: body 3.0 × 3.0, 1.1 tall at most, leads 0.25–0.38
wide on 0.65, 4.9 tip to tip.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

MSOP_BODY = (3.0, 3.0, 0.95)  # long, wide, tall above the standoff
MSOP_STANDOFF = 0.1
MSOP_SPAN = 4.9
MSOP_PITCH = 0.65
MSOP_LEAD = (0.3, 0.15)  # wide, thick


def _parts():
    return ic.gullwing(MSOP_BODY, MSOP_STANDOFF, MSOP_SPAN, ic.dual_row(8, MSOP_PITCH), MSOP_LEAD)


def lmv358_msop8_shape():
    return bd.Compound(children=_parts()[0], label="lmv358_msop8")


def lmv358_msop8_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        lmv358_msop8_shape(),
        "lmv358-msop8",
        lmv358_msop8_anchors(),
        Path(sys.argv[1]),
        merge_faces=False,
    )
