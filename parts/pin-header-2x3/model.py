# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2.54 mm pin header, 2 × 3: the ICSP header.

The shape is `cad.header.block_2x3_shape`. Leads 1 and 2 sit on the +Y row.
"""

import sys
from pathlib import Path

from cad import glb, header


def pin_header_2x3_shape():
    return header.block_2x3_shape()


def pin_header_2x3_anchors():
    return header.block_2x3_anchors()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        pin_header_2x3_shape(),
        "pin-header-2x3",
        pin_header_2x3_anchors(),
        Path(sys.argv[1]),
    )
