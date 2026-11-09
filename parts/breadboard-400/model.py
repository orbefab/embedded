# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Half-size 400-point solderless breadboard.

Part frame: the underside on z = 0, the top at z = 8.5, centred on the
origin in X and Y, columns 1–30 along X from -X. Rows a–e on -Y, f–j on +Y.
Beside row a the outer rail is + and the inner −; beside row j the inner
is + and the outer −, as the BB830 drawing has them. Each rail has 25 holes in
groups of five. `cad.breadboard` places and numbers the holes: a1 is lead 1, j30
lead 300, then rails W, X, Y and Z of 25 each, leads 301 to 400.

From half-size breadboard listings (3.25" × 2.15", 82.55 × 54.6 × 8.5), 300
terminal and 100 rail points on 2.54; the length is an estimate. The rails'
Y, the holes' size and depth and the channel are estimates (`# verify`, in
`cad.breadboard`); the side dovetails and the adhesive back are not modelled.
"""

import sys
from pathlib import Path

from cad import breadboard, glb

COLUMNS = 30
RAIL_GROUPS = 5
SIZE = (82.55, 54.6, 8.5)  # verify — length

if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        breadboard.shape(COLUMNS, RAIL_GROUPS, SIZE, "breadboard_400"),
        "breadboard-400",
        breadboard.anchors(COLUMNS, RAIL_GROUPS, SIZE[2]),
        Path(sys.argv[1]),
    )
