# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""9 V PP3 alkaline battery (Energizer 522 / ANSI 1604A) with snap clip and 2-pin lead.

Part frame: the battery lies flat on the bench at z = 0, body centred on the
origin along X and Y (46.4 mm long along X, 26.5 mm wide along Y, 17.5 mm tall
along Z). The snap terminals and clip sit on the -X end, and the red (+) and
black (-) wires run along -X into a 2-pin female jumper connector whose mouths
face -X: lead 1 (pos, red) at y = -1.27, lead 2 (neg, black) at y = +1.27.

Dimensions from Energizer 522 Product Datasheet (Form No. 522GL1019):
body max 46.40 mm long (48.50 mm overall with snap terminals), 26.50 mm wide,
17.50 mm thick, snap centre spacing 12.70 mm (12.45 to 12.95 mm). The snap
clip and short jumper lead are a starter-kit battery clip's shape (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import gold, nickel, plastic
from cad.geom import box
from cad.mark import text

BODY_L = 46.4
BODY_W = 26.5
BODY_H = 17.5
GOLD_L = 13.0
CORNER_R = 1.8

CLIP_L = 4.6
CLIP_W = 25.0
CLIP_H = 13.5
CLIP_Z = 2.0

WIRE_D = 1.1
LEAD_RUN = 10.0
CONNECTOR = (10.0, 5.4, 2.8)  # long, wide, tall
SOCKET = (1.0, 5.0)  # square opening, deep
PITCH = 2.54

BODY_X0 = -BODY_L / 2.0
BODY_X1 = BODY_L / 2.0
GOLD_X1 = BODY_X0 + GOLD_L
CLIP_X0 = BODY_X0 - CLIP_L
BACK_X = CLIP_X0 - LEAD_RUN
MOUTH_X = BACK_X - CONNECTOR[0]
SOCKET_Z = CONNECTOR[2] / 2.0

_BLACK_WRAP = plastic("#1D1E22", roughness=0.38)
_GOLD_BAND = gold("#C8902E")
_NICKEL = nickel("#C9CCD2")
_CLIP = plastic("#24252A", roughness=0.45)
_RED = plastic("#C8231E")
_BLACK = plastic("#1A1A1A")
_MARK_DARK = plastic("#181818")
_MARK_LIGHT = plastic("#E8E6DF")


def _lead_y(lead):
    return -PITCH / 2.0 if lead == 1 else PITCH / 2.0


def battery_9v_anchors():
    return [
        glb.Anchor(lead, None, (MOUTH_X, _lead_y(lead), SOCKET_Z), axis=(-1.0, 0.0, 0.0), kind="socket")
        for lead in (1, 2)
    ]


def _rounded_x_prism(x0, x1, w, h, r):
    length = x1 - x0
    with bd.BuildPart() as prism:
        with bd.BuildSketch(bd.Plane.YZ.offset(x0)):
            bd.RectangleRounded(w, h, r)
        bd.extrude(amount=length)
    return bd.Pos(0, 0, h / 2.0) * prism.part


def _snap_clip():
    clip = box(CLIP_X0 + CLIP_L / 2.0, 0, CLIP_Z + CLIP_H / 2.0, CLIP_L, CLIP_W, CLIP_H)
    clip = bd.fillet(clip.edges().filter_by(bd.Axis.X), 1.6)
    # Strain-relief boot on the -X face where the wires leave.
    boot = box(CLIP_X0 - 1.2, 0, 4.5, 2.6, 5.2, 3.4)
    return clip + boot


def _terminals():
    # Visible nickel snap collars peeking at the body/clip seam.
    pos = bd.Pos(BODY_X0 - 0.5, -6.35, BODY_H / 2.0) * bd.Rot(0, 90, 0) * bd.Cylinder(2.6, 1.4)
    neg = bd.Pos(BODY_X0 - 0.5, 6.35, BODY_H / 2.0) * bd.Rot(0, 90, 0) * bd.Cylinder(3.4, 1.4)
    return pos + neg


def _wire(lead):
    y_boot = -1.2 if lead == 1 else 1.2
    y_plug = _lead_y(lead)
    low = WIRE_D / 2.0
    points = [
        (CLIP_X0 - 1.5, y_boot, 4.5),
        (CLIP_X0 - 3.6, y_boot, 4.5),
        (CLIP_X0 - 5.8, y_plug, low),
        (BACK_X + 2.5, y_plug, SOCKET_Z),
        (BACK_X + 0.4, y_plug, SOCKET_Z),
    ]
    path = bd.FilletPolyline(*points, radius=0.9)
    plane = bd.Plane(origin=path @ 0, z_dir=path % 0)
    return bd.sweep(plane * bd.Circle(WIRE_D / 2.0), path)


def _connector():
    long, wide, tall = CONNECTOR
    opening, deep = SOCKET
    housing = box(BACK_X - long / 2.0, 0, tall / 2.0, long, wide, tall)
    for lead in (1, 2):
        housing = housing - box(MOUTH_X + deep / 2.0 - 0.1, _lead_y(lead), SOCKET_Z, deep + 0.2, opening, opening)
    return housing


def battery_9v_shape():
    gold_section = _rounded_x_prism(BODY_X0, GOLD_X1, BODY_W, BODY_H, CORNER_R)
    black_section = _rounded_x_prism(GOLD_X1 - 0.05, BODY_X1, BODY_W, BODY_H, CORNER_R)
    end_plate = box(BODY_X1 - 0.1, 0, BODY_H / 2.0, 0.3, BODY_W - 3.0, BODY_H - 3.0)
    polarity_marks = text(BODY_X0 + 6.5, -6.5, BODY_H, "+", 4.2) + text(BODY_X0 + 6.5, 6.5, BODY_H, "-", 4.2)
    label_mark = text(5.0, 0.0, BODY_H, "9V", 6.5)
    children = [
        paint(gold_section, "gold-band", _GOLD_BAND),
        paint(black_section, "body", _BLACK_WRAP),
        paint(end_plate, "end-plate", _NICKEL),
        paint(_terminals(), "terminals", _NICKEL),
        paint(_snap_clip(), "clip", _CLIP),
        paint(_connector(), "connector", _BLACK),
        paint(_wire(1), "wire-1", _RED),
        paint(_wire(2), "wire-2", _BLACK),
        paint(polarity_marks, "polarity-mark", _MARK_DARK),
        paint(label_mark, "label-mark", _MARK_LIGHT),
    ]
    return bd.Compound(children=children, label="battery_9v")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(battery_9v_shape(), "battery-9v", battery_9v_anchors(), Path(sys.argv[1]))
