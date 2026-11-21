# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Espressif ESP32-S3, QFN-56, 7 x 7.

Part frame: on the board's top at z = 0, centred on the origin, edges along
X and Y. Leads count counter-clockwise from the top: 1 to 14 down the -X
edge from +Y, then along -Y, up +X and back along +Y (`cad.ic.qfn`). The
datasheet numbers the pins the same way, anti-clockwise from pin 1 in the
top view. The exposed pad is lead 57 and is GND. The dot marks lead 1.

From ESP32-S3 Series Datasheet v2.2, Figure 7-1, the QFN56 (7x7 mm)
package, not Figure 7-2. Nominals: A 0.85, D and E 7.00, D2 and E2 4.00,
e 0.40, b 0.20, L 0.40. The terminal thickness is the default 0.05, which
is A1 maximum.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic

QFN_SIZE = 7.0
QFN_HEIGHT = 0.85
QFN_PITCH = 0.4
QFN_TERMINAL = (0.20, 0.40)  # b, L, nominal
QFN_EXPOSED = 4.00


def _parts():
    return ic.qfn(56, QFN_SIZE, QFN_HEIGHT, QFN_PITCH, QFN_TERMINAL, exposed=QFN_EXPOSED)


def esp32s3_qfn56_shape():
    return bd.Compound(children=_parts()[0], label="esp32s3_qfn56")


def esp32s3_qfn56_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        esp32s3_qfn56_shape(),
        "esp32s3-qfn56",
        esp32s3_qfn56_anchors(),
        Path(sys.argv[1]),
    )
