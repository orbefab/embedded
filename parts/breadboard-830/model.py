# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Full-size 830-point solderless breadboard, BB830 size.

Part frame: the underside on z = 0, the top at z = 8.5, centred on the
origin in X and Y, columns 1–63 along X from -X. Rows a–e on -Y, f–j on +Y.
Each side has a − rail outside and a + rail inside, 50 holes each in groups
of five, one strip the whole length. `cad.breadboard` places and numbers
the holes: a1 is lead 1, j63 lead 630, then rails W, X, Y and Z of 50 each,
leads 631 to 830.

From the BusBoard BB830 datasheet: 165.1 × 54.6 × 8.5, 630 terminal and 200
rail points on 2.54. The rails' Y, the holes' size and depth and the channel
are estimates (`# verify`, in `cad.breadboard`); the side dovetails and the
adhesive back are not modelled.
"""

import sys
from pathlib import Path

from cad import breadboard, glb

COLUMNS = 63
RAIL_GROUPS = 10
SIZE = (165.1, 54.6, 8.5)

if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        breadboard.shape(COLUMNS, RAIL_GROUPS, SIZE, "breadboard_830"),
        "breadboard-830",
        breadboard.anchors(COLUMNS, RAIL_GROUPS, SIZE[2]),
        Path(sys.argv[1]),
    )
