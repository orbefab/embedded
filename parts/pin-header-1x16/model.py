# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2.54 mm pin header, one row of 16: a 1602 character LCD module's header.

The shape is `cad.header.row_shape`. Lead 1 is the pin at -X.
"""

import sys
from pathlib import Path

from cad import glb, header

COUNT = 16


def pin_header_1x16_shape():
    return header.row_shape(COUNT)


def pin_header_1x16_anchors():
    return header.row_anchors(COUNT)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        pin_header_1x16_shape(),
        "pin-header-1x16",
        pin_header_1x16_anchors(),
        Path(sys.argv[1]),
    )
