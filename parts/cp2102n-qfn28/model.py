# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Silicon Labs CP2102N-A02-GQFN28, QFN-28, 5 x 5.

Part frame: on the board's top at z = 0, centred on the origin, edges along
X and Y. Leads count counter-clockwise from the top: 1 to 7 down the -X
edge from +Y, then along -Y, up +X and back along +Y (`cad.ic.qfn`). The
exposed pad is lead 29 and is GND. The dot marks lead 1.

From Silicon Labs CP2102N Data Sheet Rev. 1.3, Table 6.1. The body is
5.00 mm, 0.75 tall, on a 0.50 pitch. Terminals are 0.25 wide and 0.55 long,
the nominals of b and L. The exposed pad is 3.25, the nominal of D2 and E2.
The terminal thickness is the default 0.05, which is A1 maximum.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

QFN_SIZE = 5.0
QFN_HEIGHT = 0.75
QFN_PITCH = 0.5
QFN_TERMINAL = (0.25, 0.55)  # b, L, nominal
QFN_EXPOSED = 3.25


def _parts():
    return ic.qfn(28, QFN_SIZE, QFN_HEIGHT, QFN_PITCH, QFN_TERMINAL, exposed=QFN_EXPOSED)


def cp2102n_qfn28_shape():
    return bd.Compound(children=_parts()[0], label="cp2102n_qfn28")


def cp2102n_qfn28_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        cp2102n_qfn28_shape(),
        "cp2102n-qfn28",
        cp2102n_qfn28_anchors(),
        Path(sys.argv[1]),
    )
