# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""SG90 micro servo, with its lead and a 3-pin female connector.

Part frame: the case stands on its underside at z = 0. The origin is on the
output shaft, which sits at the -X end of the case, so the horn turns about
the Z axis. The horn is drawn at mid travel, pointing +Y, and the joint
`horn` turns it about -Z with the shaft angle.

The lead leaves the case's -X end low down, drops to the table and ends in a
connector whose three sockets face -X: lead 1 (GND, brown) at -Y, lead 2
(VCC, red) in the middle and lead 3 (SIGNAL, orange) at +Y. Each anchor is a
socket mouth, where a jumper's pin goes in.

Tower Pro's SG90 drawing gives the case as 22.2 long and 11.8 wide, the ears
32.2 end to end with their underside 16 above the base, and 31 to the top of
the horn. The case height, the gear tower, the horn, the ears' thickness and
the connector are a kit servo's shape (`# verify`). The real lead is about
25 cm long. The drawn one is short so the servo sits next to a breadboard.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic
from cad.geom import box, cyl_z

CASE = (22.2, 11.8, 22.7)  # long, wide, tall to the top of the case  # verify the height
SHAFT_FROM_END = 5.9  # the shaft over the centre of the gear tower, tangent to the end
EARS = (32.2, 2.5, 15.9)  # end to end, thick, underside above the base  # verify the thickness
EAR_HOLE = (2.0, 2.3)  # diameter, centre in from the ear's tip  # verify
TOWER = (11.8, 4.0)  # diameter, above the case  # verify
BOSS = (5.6, 2.4, 6.4)  # diameter, above the case, centre from the shaft  # verify
SPLINE = (4.8, 28.2)  # diameter, top  # verify
HORN_TOP = 31.0
HORN = (7.0, 3.5, 1.5, 15.0, 4.0)  # hub d, hub tall, arm thick, arm reach, tip width  # verify
WIRE_D = 1.0
LEAD_Z = 3.5  # where the lead leaves the case  # verify
CONNECTOR = (14.0, 7.9, 2.8)  # long, wide, tall  # verify
SOCKET = (1.0, 6.0)  # square opening, deep
PITCH = 2.54
LEAD_RUN = 18.0  # from the case end to the connector's back

_BLUE = plastic("#2D62C8", alpha=0.82, roughness=0.25)
_HORN = plastic("#F4F4F2")
_MOTOR = plastic("#2B2C31")
_BLACK = plastic("#1A1A1A")
_WIRES = (plastic("#6B3A1E"), plastic("#C8231E"), plastic("#F28C1C"))

END_X = -SHAFT_FROM_END
FAR_X = END_X + CASE[0]
MID_X = (END_X + FAR_X) / 2.0
BACK_X = END_X - LEAD_RUN
MOUTH_X = BACK_X - CONNECTOR[0]
SOCKET_Z = CONNECTOR[2] / 2.0


def _lead_y(lead):
    return (lead - 2) * PITCH


def servo_sg90_anchors():
    return [
        glb.Anchor(lead, None, (MOUTH_X, _lead_y(lead), SOCKET_Z), axis=(-1.0, 0.0, 0.0), kind="socket")
        for lead in (1, 2, 3)
    ]


def _case():
    long, wide, tall = CASE
    body = box(MID_X, 0, tall / 2.0, long, wide, tall)
    ear_long, ear_t, ear_z = EARS
    ears = box(MID_X, 0, ear_z + ear_t / 2.0, ear_long, wide, ear_t)
    hole_d, hole_in = EAR_HOLE
    for sign in (-1.0, 1.0):
        x = MID_X + sign * (ear_long / 2.0 - hole_in)
        ears = ears - cyl_z(x, 0, ear_z - 0.1, ear_z + ear_t + 0.1, hole_d / 2.0)
        ears = ears - box(x + sign * hole_in / 2.0, 0, ear_z + ear_t / 2.0, hole_in, 1.0, ear_t + 0.2)
    tower_d, tower_h = TOWER
    tower = cyl_z(0, 0, tall - 0.01, tall + tower_h, tower_d / 2.0)
    boss_d, boss_h, boss_x = BOSS
    boss = cyl_z(boss_x, 0, tall - 0.01, tall + boss_h, boss_d / 2.0)
    return body + ears + tower + boss


def _inside():
    """The motor, seen through the case."""
    long, _, _ = CASE
    return cyl_z(FAR_X - 6.0, 0, 2.0, 14.5, 3.0)


def _horn():
    hub_d, hub_h, arm_t, reach, tip_w = HORN
    top = HORN_TOP
    with bd.BuildPart() as arm:
        with bd.BuildSketch(bd.Plane.XY.offset(top - arm_t)):
            bd.Circle(hub_d / 2.0 - 0.2)
            with bd.Locations((0, reach)):
                bd.Circle(tip_w / 2.0)
            bd.Polygon((-hub_d / 2.0 + 0.4, 0), (hub_d / 2.0 - 0.4, 0), (tip_w / 2.0, reach), (-tip_w / 2.0, reach))
        bd.extrude(amount=arm_t)
    shape = arm.part + cyl_z(0, 0, top - hub_h, top, hub_d / 2.0)
    for y in (6.0, 8.5, 11.0, 13.5):
        shape = shape - cyl_z(0, y, top - arm_t - 0.1, top + 0.1, 0.45)
    shape = shape - cyl_z(0, 0, top - 1.0, top + 0.1, 1.0)
    return shape


def _wire_path(lead):
    y_case = (lead - 2) * WIRE_D
    y_plug = _lead_y(lead)
    low = WIRE_D / 2.0
    points = [
        (END_X + 0.8, y_case, LEAD_Z),
        (END_X - 2.5, y_case, LEAD_Z),
        (END_X - 7.0, y_case, low),
        (BACK_X + 7.0, y_case, low),
        (BACK_X + 3.0, y_plug, SOCKET_Z),
        (BACK_X + 0.5, y_plug, SOCKET_Z),
    ]
    return bd.FilletPolyline(*points, radius=1.2)


def _wire(lead):
    path = _wire_path(lead)
    plane = bd.Plane(origin=path @ 0, z_dir=path % 0)
    return bd.sweep(plane * bd.Circle(WIRE_D / 2.0), path)


def _connector():
    long, wide, tall = CONNECTOR
    opening, deep = SOCKET
    housing = box(BACK_X - long / 2.0, 0, tall / 2.0, long, wide, tall)
    for lead in (1, 2, 3):
        housing = housing - box(MOUTH_X + deep / 2.0 - 0.1, _lead_y(lead), SOCKET_Z, deep + 0.2, opening, opening)
    # The latch ramp on top.
    return housing + box(BACK_X - long * 0.55, 0, tall + 0.3, 3.0, 2.2, 0.6)


def servo_sg90_shape():
    children = [
        paint(_case(), "case", _BLUE),
        paint(_inside(), "motor", _MOTOR),
        paint(cyl_z(0, 0, CASE[2] + TOWER[1] - 0.01, SPLINE[1], SPLINE[0] / 2.0), "spline", _HORN),
        bd.Compound(children=[paint(_horn(), "arm", _HORN)], label="horn"),
        paint(_connector(), "connector", _BLACK),
    ]
    for lead in (1, 2, 3):
        children.append(paint(_wire(lead), f"wire-{lead}", _WIRES[lead - 1]))
    return bd.Compound(children=children, label="servo-sg90")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(servo_sg90_shape(), "servo-sg90", servo_sg90_anchors(), Path(sys.argv[1]))
