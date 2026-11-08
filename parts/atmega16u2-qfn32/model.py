# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Microchip ATmega16U2-MU, QFN-32 (MLF, 5 × 5): the Uno's USB bridge.

Part frame: on the board's top at z = 0, centred on the origin, edges along
X and Y. Leads count counter-clockwise from the top: 1 to 8 down the -X
edge from +Y, 9 to 16 along -Y, 17 to 24 up +X, 25 to 32 back along +Y
(`cad.ic.qfn`). The exposed pad under the middle is lead 33, as the Uno
Rev3 footprint numbers it; it is soldered to GND. The dot marks lead 1.

From the ATmega16U2 datasheet's 32M1-A outline: 5.00 × 5.00, 0.80–1.00
tall, terminals 0.18–0.30 wide and 0.30–0.50 long on 0.5, exposed pad
2.95–3.25 square.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

QFN_SIZE = 5.0
QFN_HEIGHT = 0.9
QFN_PITCH = 0.5
QFN_TERMINAL = (0.25, 0.4)  # wide, long
QFN_EXPOSED = 3.1


def _parts():
    return ic.qfn(32, QFN_SIZE, QFN_HEIGHT, QFN_PITCH, QFN_TERMINAL, exposed=QFN_EXPOSED)


def atmega16u2_qfn32_shape():
    return bd.Compound(children=_parts()[0], label="atmega16u2_qfn32")


def atmega16u2_qfn32_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        atmega16u2_qfn32_shape(),
        "atmega16u2-qfn32",
        atmega16u2_qfn32_anchors(),
        Path(sys.argv[1]),
        merge_faces=False,
    )
