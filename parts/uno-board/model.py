# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""The Uno R3 compatible board's bare PCB: the outline, its holes and the pin legend.

Part frame: the board's underside on z = 0 and its top at z = UNO_PCB_T, the
outline's bounding box centred on the origin, seen from the top with the
USB and power jacks at -X and the digital header along +Y. Every part on it
is its own model, placed by the board's layout. The board has no leads of
its own.

Lengths below are (u, w): u from the jack edge toward the notched edge, w
from the power and analog edge toward the digital edge. RobotDyn's UNO R3
CH340G dimension diagram (14 Jun 2017) gives the board, 68.58 x 53.34, the
four Φ3.2 holes, and the header rows: NC at u = 27.95, A0 to A5 from
50.81 to 63.51, SCL at 18.76, a 4.10 gap between D8 and D7, D0 at 63.50.
The diagram prints the header rows 2.55 from the long edges. This model
places them 2.54 mm in, one pitch. It dimensions the w of the two holes at
the notched end, 7.63 and 35.57, and `MOUNTING` uses those. It does not
dimension the w of the two jack-end holes or the u of the two notched-end
holes, the notched edge, or the second row of holes inside each header.
Those are measured (`# measured`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import cyl_z
from cad.mark import print_face

UNO_L = 68.58
UNO_W = 53.34
UNO_PCB_T = 1.6
UNO_SILK_LIFT = 0.02
PITCH = 2.54

OUTLINE = (  # measured: the notched edge's steps and corners
    (0.0, 0.0),
    (66.04, 0.0),
    (66.04, 2.54),
    (UNO_L, 5.08),
    (UNO_L, 38.1),
    (66.7, 40.0),
    (66.7, 50.8),
    (64.16, UNO_W),
    (0.0, UNO_W),
)
MOUNTING_D = 3.2
# (u, w). The diagram dimensions the jack-end u (13.97, 15.24) and the notched-end w (7.63, 35.57).
MOUNTING = ((13.97, 2.54), (15.24, 50.8), (66.04, 7.63), (66.04, 35.57))

INNER_DIGITAL_W = 48.3  # measured
INNER_POWER_W = 6.0  # measured
VIA_D = 1.0

DIGITAL_HIGH = ("SCL", "SDA", "AREF", "GND", "13", "12", "~11", "~10", "~9", "8")
DIGITAL_LOW = ("7", "~6", "~5", "4", "~3", "2", "TX 1", "RX 0")
POWER = ("", "IOREF", "RESET", "3V3", "5V", "GND", "GND", "VIN")
ANALOG = ("A0", "A1", "A2", "A3", "A4", "A5")
LEGEND_SIZE = 0.9
LEDS = (("ON", 42.2), ("RX", 35.8), ("TX", 29.4), ("L", 23.2))  # measured, beside the LED column
LED_LEGEND_U = 27.9

_BLUE = plastic("#1458A8")
_WHITE = plastic("#F4F1E8")


def _xy(u, w):
    return u - UNO_L / 2.0, w - UNO_W / 2.0


def _row(u0, count):
    return [u0 + k * PITCH for k in range(count)]


DIGITAL_HIGH_U = _row(18.76, len(DIGITAL_HIGH))
DIGITAL_LOW_U = _row(63.50 - 7 * PITCH, len(DIGITAL_LOW))
POWER_U = _row(27.95, len(POWER))
ANALOG_U = _row(50.81, len(ANALOG))


def _board():
    with bd.BuildSketch() as sketch:
        bd.Polygon(*[_xy(u, w) for u, w in OUTLINE], align=None)
    pcb = bd.extrude(sketch.sketch, amount=UNO_PCB_T)
    holes = [(u, w, MOUNTING_D) for u, w in MOUNTING]
    for row_w, us in (
        (INNER_DIGITAL_W, DIGITAL_HIGH_U + DIGITAL_LOW_U),
        (INNER_POWER_W, POWER_U + ANALOG_U),
    ):
        holes += [(u, row_w, VIA_D) for u in us]
    for u, w, d in holes:
        x, y = _xy(u, w)
        pcb -= cyl_z(x, y, -0.4, UNO_PCB_T + 0.4, d / 2.0)
    return paint(pcb, "pcb", _BLUE)


def _legend():
    top = UNO_PCB_T + UNO_SILK_LIFT
    marks = []

    def mark(u, w, word, rotation):
        x, y = _xy(u, w)
        shape = print_face(x, y, top, word, LEGEND_SIZE, rotation=rotation)
        marks.append(paint(shape, f"silk-{len(marks)}", _WHITE))

    digital = zip(DIGITAL_HIGH_U + DIGITAL_LOW_U, DIGITAL_HIGH + DIGITAL_LOW, strict=True)
    for u, word in digital:
        mark(u, INNER_DIGITAL_W - 1.0 - len(word) * LEGEND_SIZE * 0.35, word, 90)
    for u, word in zip(POWER_U + ANALOG_U, POWER + ANALOG, strict=True):
        if word:
            mark(u, INNER_POWER_W + 1.0 + len(word) * LEGEND_SIZE * 0.35, word, 90)
    for word, w in LEDS:
        mark(LED_LEGEND_U, w, word, 0)
    mark(65.36, 33.0, "ICSP", 0)
    return marks


def uno_board_shape():
    return bd.Compound(children=[_board(), *_legend()], label="uno_board")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(uno_board_shape(), "uno-board", None, Path(sys.argv[1]))
