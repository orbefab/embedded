# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2.54 mm socket strip, 1 × 6, 8.5 tall: the Uno's analog header.

The shape is `cad.header.socket_strip`. Lead 1 is the contact at -X.
"""

import sys
from pathlib import Path

from cad import glb, header

COUNT = 6


def socket_strip_1x6_shape():
    return header.socket_strip_shape(COUNT)


def socket_strip_1x6_anchors():
    return header.socket_strip_anchors(COUNT)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        socket_strip_1x6_shape(),
        "socket-strip-1x6",
        socket_strip_1x6_anchors(),
        Path(sys.argv[1]),
    )
