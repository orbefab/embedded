# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""4xAA 6 V alkaline battery pack (four Energizer E91 / IEC LR6 cells in a flat holder).

Part frame: the holder sits on its underside at z = 0, centred on the origin
along X and Y (58.0 mm along X, 62.0 mm along Y, 15.5 mm tall). Four AA cells
lie parallel to X in four side-by-side bays with alternating polarity. The red
(+) and black (-) wire leads leave the -X wall into a 2-pin female jumper
connector whose mouths face -X: lead 1 (pos, red) at y = -1.27, lead 2 (neg,
black) at y = +1.27.

Cell dimensions follow Energizer E91 / IEC LR6: diameter 14.2 mm (13.7 to
14.5 mm max), length 50.0 mm (49.5 to 50.5 mm max). Holder shell and short
jumper lead follow a standard 4xAA flat holder (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import gold, nickel, plastic
from cad.geom import box, fuse

HOLDER_L = 58.0
HOLDER_W = 62.0
HOLDER_H = 15.5
FLOOR_T = 1.8
WALL_T = 1.8
RIB_T = 1.0

CELL_D = 14.0
CELL_R = CELL_D / 2.0
CELL_BODY_L = 48.6
NUB_D = 5.0
NUB_L = 1.4
GOLD_L = 12.0
CELL_Z = FLOOR_T + CELL_R
BAY_YS = (-22.5, -7.5, 7.5, 22.5)

WIRE_D = 1.1
LEAD_RUN = 10.0
CONNECTOR = (10.0, 5.4, 2.8)
SOCKET = (1.0, 5.0)
PITCH = 2.54

END_X = -HOLDER_L / 2.0
BACK_X = END_X - LEAD_RUN
MOUTH_X = BACK_X - CONNECTOR[0]
SOCKET_Z = CONNECTOR[2] / 2.0

_HOLDER = plastic("#1F2024", roughness=0.52)
_BLACK_WRAP = plastic("#1D1E22", roughness=0.36)
_GOLD_BAND = gold("#C8902E")
_NICKEL = nickel("#C9CCD2")
_RED = plastic("#C8231E")
_BLACK = plastic("#1A1A1A")


def _lead_y(lead):
    return -PITCH / 2.0 if lead == 1 else PITCH / 2.0


def battery_aa_4x_anchors():
    return [
        glb.Anchor(lead, None, (MOUTH_X, _lead_y(lead), SOCKET_Z), axis=(-1.0, 0.0, 0.0), kind="socket")
        for lead in (1, 2)
    ]


def _holder():
    outer = box(0, 0, HOLDER_H / 2.0, HOLDER_L, HOLDER_W, HOLDER_H)
    cavity = box(
        0,
        0,
        FLOOR_T + (HOLDER_H - FLOOR_T) / 2.0 + 0.5,
        HOLDER_L - 2.0 * WALL_T,
        HOLDER_W - 2.0 * WALL_T,
        HOLDER_H - FLOOR_T + 1.0,
    )
    shell = outer - cavity
    ribs = [
        box(0, y, FLOOR_T + (HOLDER_H - FLOOR_T) * 0.35, HOLDER_L - 2.0 * WALL_T, RIB_T, (HOLDER_H - FLOOR_T) * 0.7)
        for y in (-15.0, 0.0, 15.0)
    ]
    boot = box(END_X - 1.0, 0, 3.6, 2.2, 5.2, 3.4)
    return fuse([shell, *ribs, boot]).clean()


def _cell_x_cyl(x0, x1, y, radius):
    length = x1 - x0
    return bd.Pos((x0 + x1) / 2.0, y, CELL_Z) * bd.Rot(0, 90, 0) * bd.Cylinder(radius, length)


def _cells():
    black_parts = []
    gold_parts = []
    metal_parts = []
    half = CELL_BODY_L / 2.0
    for index, y in enumerate(BAY_YS):
        # Alternate polarity across bays so the four cells sit visually in series.
        pos_at_neg_x = index % 2 == 0
        if pos_at_neg_x:
            gold_parts.append(_cell_x_cyl(-half, -half + GOLD_L, y, CELL_R))
            black_parts.append(_cell_x_cyl(-half + GOLD_L - 0.05, half, y, CELL_R))
            metal_parts.append(_cell_x_cyl(-half - NUB_L, -half + 0.1, y, NUB_D / 2.0))
            metal_parts.append(_cell_x_cyl(half - 0.05, half + 0.25, y, CELL_R - 0.6))
            # Holder end contacts (flat plate at +, spring cylinder at -).
            metal_parts.append(box(-HOLDER_L / 2.0 + WALL_T + 0.6, y, CELL_Z, 0.6, 8.0, 8.0))
            metal_parts.append(_cell_x_cyl(half + 0.2, HOLDER_L / 2.0 - WALL_T, y, 3.2))
        else:
            gold_parts.append(_cell_x_cyl(half - GOLD_L, half, y, CELL_R))
            black_parts.append(_cell_x_cyl(-half, half - GOLD_L + 0.05, y, CELL_R))
            metal_parts.append(_cell_x_cyl(half - 0.1, half + NUB_L, y, NUB_D / 2.0))
            metal_parts.append(_cell_x_cyl(-half - 0.25, -half + 0.05, y, CELL_R - 0.6))
            metal_parts.append(box(HOLDER_L / 2.0 - WALL_T - 0.6, y, CELL_Z, 0.6, 8.0, 8.0))
            metal_parts.append(_cell_x_cyl(-HOLDER_L / 2.0 + WALL_T, -half - 0.2, y, 3.2))
    return fuse(black_parts), fuse(gold_parts), fuse(metal_parts)


def _wire(lead):
    y_boot = -1.2 if lead == 1 else 1.2
    y_plug = _lead_y(lead)
    low = WIRE_D / 2.0
    points = [
        (END_X - 1.2, y_boot, 3.6),
        (END_X - 3.4, y_boot, 3.6),
        (END_X - 5.6, y_plug, low),
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


def battery_aa_4x_shape():
    black_cells, gold_cells, metal_parts = _cells()
    children = [
        paint(_holder(), "holder", _HOLDER),
        paint(black_cells, "cells-black", _BLACK_WRAP),
        paint(gold_cells, "cells-gold", _GOLD_BAND),
        paint(metal_parts, "contacts", _NICKEL),
        paint(_connector(), "connector", _BLACK),
        paint(_wire(1), "wire-1", _RED),
        paint(_wire(2), "wire-2", _BLACK),
    ]
    return bd.Compound(children=children, label="battery_aa_4x")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(battery_aa_4x_shape(), "battery-aa-4x", battery_aa_4x_anchors(), Path(sys.argv[1]))
