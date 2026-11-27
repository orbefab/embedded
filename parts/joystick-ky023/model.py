# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""KY-023 2-axis thumb joystick breakout module (Joy-IT COM-KY023JM).

Part frame: PCB lies flat on z = 0, centred on X = 0, Y = 0. Dimensions from
Joy-IT COM-KY023JM datasheet (34 x 39 x 26 mm overall including header pins and
thumbstick cap): PCB 34.0 x 26.0 x 1.6 mm with four 3.0 mm corner mounting holes,
metal gimbal housing (16 x 16 x 11.5 mm) at x = +3.0 mm, and a 1x5 right-angle
2.54 mm header at the -X edge pointing along -X:
lead 1 (GND), lead 2 (VCC / +5V), lead 3 (VRX), lead 4 (VRY), lead 5 (SW).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import gold, plastic, steel, tin
from cad.geom import box, cyl_z, fuse

PCB_L = 34.0
PCB_W = 26.0
PCB_T = 1.6
HOLE_X = 13.5
HOLE_Y = 9.5
HOLE_R = 1.5

GIMBAL_X = 3.0
GIMBAL_W = 16.0
GIMBAL_H = 11.5

PITCH = 2.54
PIN_W = 0.64
HDR_X_OUTER = -PCB_L / 2.0  # -17.0 mm
HDR_W = 2.54
PIN_Z = PCB_T + HDR_W / 2.0  # 2.87 mm
PIN_TIP_X = HDR_X_OUTER - 5.5  # -22.5 mm (~39.5 mm total length matching 39 mm)
PIN_BEND_X = HDR_X_OUTER + HDR_W + 0.8  # -13.66 mm

_PCB = plastic("#181A1E")
_GOLD = gold("#C8A850")
_METAL = steel("#B8BDC4")
_HOUSING = plastic("#1A1A1A")
_TIN = tin()


def _pin_y(lead):
    # Lead 1 (+Y) = GND, 2 = VCC, 3 = VRX, 4 = VRY, 5 (-Y) = SW
    return (3.0 - lead) * PITCH


def joystick_ky023_anchors():
    return [
        glb.Anchor(
            lead,
            f"pin-{lead}",
            (PIN_TIP_X, _pin_y(lead), PIN_Z),
            axis=(-1.0, 0.0, 0.0),
            also=(f"housing-{lead}",),
        )
        for lead in range(1, 6)
    ]


def _pcb():
    pcb = box(0, 0, PCB_T / 2.0, PCB_L, PCB_W, PCB_T)
    for sx in (-1.0, 1.0):
        for sy in (-1.0, 1.0):
            pcb -= cyl_z(sx * HOLE_X, sy * HOLE_Y, -0.2, PCB_T + 0.2, HOLE_R)
    return pcb


def _mounting_rings():
    rings = []
    for sx in (-1.0, 1.0):
        for sy in (-1.0, 1.0):
            ring = cyl_z(sx * HOLE_X, sy * HOLE_Y, PCB_T - 0.02, PCB_T + 0.04, HOLE_R + 0.9) - cyl_z(
                sx * HOLE_X, sy * HOLE_Y, PCB_T - 0.1, PCB_T + 0.2, HOLE_R
            )
            rings.append(ring)
    return fuse(rings).clean()


def _gimbal():
    # Hollow metal gimbal cage sitting on the PCB top (z = PCB_T..PCB_T + GIMBAL_H).
    outer = box(GIMBAL_X, 0, PCB_T + GIMBAL_H / 2.0, GIMBAL_W, GIMBAL_W, GIMBAL_H)
    inner = box(GIMBAL_X, 0, PCB_T + GIMBAL_H / 2.0 + 1.0, GIMBAL_W - 2.4, GIMBAL_W - 2.4, GIMBAL_H)
    return outer - inner


def _ra_pin(y):
    back = PIN_BEND_X + PIN_W / 2.0
    horiz = box((PIN_TIP_X + back) / 2.0, y, PIN_Z, back - PIN_TIP_X, PIN_W, PIN_W)
    vert = box(PIN_BEND_X, y, PIN_Z / 2.0, PIN_W, PIN_W, PIN_Z)
    return fuse([horiz, vert]).clean()


def joystick_ky023_shape():
    children = [
        paint(_pcb(), "pcb", _PCB),
        paint(_mounting_rings(), "pads", _GOLD),
        paint(_gimbal(), "gimbal", _METAL),
    ]
    for lead in range(1, 6):
        y = _pin_y(lead)
        children.append(
            paint(box(HDR_X_OUTER + HDR_W / 2.0, y, PIN_Z, HDR_W, PITCH, HDR_W), f"housing-{lead}", _HOUSING)
        )
        children.append(paint(_ra_pin(y), f"pin-{lead}", _TIN))
    return bd.Compound(children=children, label="joystick_ky023")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(joystick_ky023_shape(), "joystick-ky023", joystick_ky023_anchors(), Path(sys.argv[1]))
