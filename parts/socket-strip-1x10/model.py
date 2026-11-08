# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2.54 mm socket strip, 1 × 10, 8.5 tall: the Uno's D8-to-SCL header.

The shape is `cad.header.socket_strip_shape`. Lead 1 is the contact at -X.
"""

import sys
from pathlib import Path

from cad import glb, header

COUNT = 10


def socket_strip_1x10_shape():
    return header.socket_strip_shape(COUNT)


def socket_strip_1x10_anchors():
    return header.socket_strip_anchors(COUNT)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        socket_strip_1x10_shape(),
        "socket-strip-1x10",
        socket_strip_1x10_anchors(),
        Path(sys.argv[1]),
    )
