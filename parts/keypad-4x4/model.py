# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""4x4 matrix membrane keypad pad and 1x8 ribbon header (Parallax #27899).

Part frame: the pad lies flat on z = 0, centred on X = 0, Y = 0, keys facing +Z.
Pad dimensions from Parallax #27899 v1.2 p1: 69.0 x 76.0 mm (2.7 x 3.0 in), 1.0 mm
thick, with a 4x4 grid of white-framed key cells and bold white legends
(1 2 3 A / 4 5 6 B / 7 8 9 C / * 0 # D). Each cell's clickable coloured keycap
(`keypad-4x4-key`) sits at z = 1.0 mm in the lower portion of the cell so the
legend stays unobstructed and clicking the keycap always targets that child switch.
A 21.6 x 14.0 mm shortened ribbon tail exits the -Y edge to a 1x8 2.54 mm header
with pins pointing along -Y: leads 1..4 = R1..R4, leads 5..8 = C1..C4 left to right.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, mark, paint
from cad.color import plastic, steel, tin
from cad.geom import box, fuse

PAD_W = 69.0
PAD_H = 76.0
PAD_T = 1.0
PAD_R = 2.0

COL_XS = (-23.25, -7.75, 7.75, 23.25)
ROW_YS = (24.75, 8.25, -8.25, -24.75)
LEGENDS = (
    ("1", "2", "3", "A"),
    ("4", "5", "6", "B"),
    ("7", "8", "9", "C"),
    ("*", "0", "#", "D"),
)
CELL_W = 14.0
CELL_H = 14.8
CELL_LINE = 0.5
LEGEND_DY = 4.6

PITCH = 2.54
RIBBON_W = 21.6
RIBBON_L = 14.0
RIBBON_T = 0.6
RIBBON_Y0 = -PAD_H / 2.0  # -38.0 mm
RIBBON_Y1 = RIBBON_Y0 - RIBBON_L  # -52.0 mm

HDR_D = 2.54
HDR_H = 2.54
PIN_W = 0.64
PIN_Z = HDR_H / 2.0  # 1.27 mm
PIN_TIP_Y = RIBBON_Y1 - HDR_D - 6.0  # -60.54 mm
PIN_BACK_Y = RIBBON_Y1 + 0.5
PIN_LABELS = ("R1", "R2", "R3", "R4", "C1", "C2", "C3", "C4")

_PAD = plastic("#1A1C20", roughness=0.58)
_RIBBON = plastic("#252830", roughness=0.52)
_HOUSING = plastic("#141414", roughness=0.55)
_TRACE = steel("#C8CDD4")
_SILK = plastic("#F0F2F5")
_TIN = tin()


def _pin_x(lead):
    # Lead 1 (-X) = R1 .. Lead 4 = R4, Lead 5 = C1 .. Lead 8 (+X) = C4
    return (lead - 4.5) * PITCH


def keypad_4x4_anchors():
    return [
        glb.Anchor(
            lead,
            f"pin-{lead}",
            (_pin_x(lead), PIN_TIP_Y, PIN_Z),
            axis=(0.0, -1.0, 0.0),
            also=(f"housing-{lead}",),
        )
        for lead in range(1, 9)
    ]


def _pad():
    with bd.BuildSketch() as sketch:
        bd.RectangleRounded(PAD_W, PAD_H, PAD_R)
    return bd.extrude(sketch.sketch, amount=PAD_T)


def _ribbon():
    return box(0.0, (RIBBON_Y0 + RIBBON_Y1) / 2.0, RIBBON_T / 2.0, RIBBON_W, RIBBON_L, RIBBON_T)


def _traces():
    parts = []
    for lead in range(1, 9):
        x = _pin_x(lead)
        parts.append(box(x, (RIBBON_Y0 + RIBBON_Y1) / 2.0, RIBBON_T + 0.02, 0.8, RIBBON_L - 1.0, 0.04))
    return fuse(parts).clean()


def _frames():
    # Outer graphic border + 16 cell outlines on the top of the pad.
    def rect_ring(cx, cy, w, h, line):
        outer = box(cx, cy, PAD_T + 0.015, w, h, 0.03)
        inner = box(cx, cy, PAD_T + 0.015, w - 2.0 * line, h - 2.0 * line, 0.06)
        return outer - inner

    rings = [rect_ring(0.0, 0.0, PAD_W - 2.4, PAD_H - 2.4, 0.6)]
    for cy in ROW_YS:
        for cx in COL_XS:
            rings.append(rect_ring(cx, cy, CELL_W, CELL_H, CELL_LINE))
    return fuse(rings).clean()


def _legends():
    marks = []
    for r, cy in enumerate(ROW_YS):
        for c, cx in enumerate(COL_XS):
            shape = mark.print_face(cx, cy + LEGEND_DY, PAD_T + 0.02, LEGENDS[r][c], 4.0, bold=True)
            marks.append(paint(shape, f"silk-{len(marks)}", _SILK))
    for lead, label in enumerate(PIN_LABELS, start=1):
        shape = mark.print_face(_pin_x(lead), RIBBON_Y0 - 3.2, RIBBON_T + 0.05, label, 1.5, rotation=90.0, bold=True)
        marks.append(paint(shape, f"silk-{len(marks)}", _SILK))
    return marks


def keypad_4x4_shape():
    children = [
        paint(_pad(), "pad", _PAD),
        paint(_ribbon(), "ribbon", _RIBBON),
        paint(_traces(), "traces", _TRACE),
        paint(_frames(), "frames", _SILK),
        *_legends(),
    ]
    for lead in range(1, 9):
        x = _pin_x(lead)
        children.append(
            paint(
                box(x, RIBBON_Y1 - HDR_D / 2.0, HDR_H / 2.0, PITCH, HDR_D, HDR_H),
                f"housing-{lead}",
                _HOUSING,
            )
        )
        pin_len = PIN_BACK_Y - PIN_TIP_Y
        children.append(
            paint(
                box(x, (PIN_TIP_Y + PIN_BACK_Y) / 2.0, PIN_Z, PIN_W, pin_len, PIN_W),
                f"pin-{lead}",
                _TIN,
            )
        )
    return bd.Compound(children=children, label="keypad_4x4")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(keypad_4x4_shape(), "keypad-4x4", keypad_4x4_anchors(), Path(sys.argv[1]))
