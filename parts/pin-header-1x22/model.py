# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2.54 mm pin header, one row of 22.

The shape is `cad.header.row_shape`. Lead 1 is the pin at -X.
"""

import sys
from pathlib import Path

from cad import glb, header

COUNT = 22


def pin_header_1x22_shape():
    return header.row_shape(COUNT)


def pin_header_1x22_anchors():
    return header.row_anchors(COUNT)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        pin_header_1x22_shape(),
        "pin-header-1x22",
        pin_header_1x22_anchors(),
        Path(sys.argv[1]),
    )
