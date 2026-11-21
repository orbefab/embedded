# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""ESP32-S3-DevKitC-1 v1.1, the bare board.

Part frame: centred on the origin, component side +Z, the long axis along
Y and the antenna end at +Y. The board has no leads of its own. Every part
on it is its own model, placed by the part's layout.

DXF_ESP32-S3-DevKitC-1_V1.1_20220429: the outline polyline runs from
x = 0 to 25.40 and y = 0 to 62.865, with 0.5 mm corner arcs. The printed
callouts say 25.40 mm and 62.74 mm. This box is the polyline's bounding
rectangle, so the placed parts share that frame, and the corners are
square. The drawing does not state the PCB thickness, and 1.6 mm is not a
measurement from it. The sheet does not name a solder-mask colour. The
photos in the v1.1 user guide show a black mask, so the PCB is plastic.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box

BOARD_X = 25.40  # the outline polyline, and the printed 25.40 mm callout
BOARD_Y = 62.865  # the outline polyline. The printed callout says 62.74 mm
BOARD_T = 1.6  # the drawing does not state this
_MASK = plastic("#1A1A1A")


def esp32s3_devkitc_1_shape():
    pcb = paint(box(0, 0, BOARD_T / 2.0, BOARD_X, BOARD_Y, BOARD_T), "pcb", _MASK)
    return bd.Compound(children=[pcb], label="esp32s3_devkitc_1")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(esp32s3_devkitc_1_shape(), "esp32s3-devkitc-1", None, Path(sys.argv[1]))
