# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Espressif ESP32-S3-WROOM-1-N16R8.

Part frame: on the board's top at z = 0, centred on the origin, the antenna
toward +Y. Leads are the castellations and the ground pad.

The body is Table 1-1 of the ESP32-S3-WROOM-1 & WROOM-1U Datasheet v1.8:
18.0 x 25.5 x 3.1 mm. Figure 10-1 draws the rest of the outside: a 0.8 mm
PCB, a 15.8 x 17.6 mm shield 1.05 mm from the far end, and 6 mm of bare
PCB at the antenna end. The shield rises to the 3.1 mm top. Each
castellation is a Ø0.55 half hole on the outline with a 0.9 by 0.45 pad on
the PCB's top. The model cuts each hole square, 0.55 wide and 0.275 deep,
so the mesh stays small. The sheet does not give the plating's thickness:
0.04 mm is not a measurement. Espressif's photo of the DevKitC-1 v1.1 shows a black
PCB, a silver shield and gold castellations, so the PCB is plastic and the
shield and the leads are metal.

Pad places are Figure 11-1, the recommended land pattern. Pin 1 is at the
antenna end of the -X edge and pin 40 is opposite it. The pin-1 row is
7.49 mm from the antenna end. Fourteen pins run down each long edge on a
1.27 mm pitch, a span of 16.51 mm, which leaves the last of them 1.5 mm
from the far end (25.5 - 7.49 - 16.51). Twelve pins run along the far end,
the outer ones 2.015 mm in from each long edge, on the same 1.27 mm pitch.
Figure 11-1 spaces the long-edge centres 17.5 mm apart on the 18.0 mm body,
so each centre is 0.25 mm inside the outline. The 0.5 printed beside the
1.5 mm pad is the overhang past the body (1.5 / 2 - 0.25). The same 0.25 mm
inset on the short edge keeps both the 7.49 mm and the 2.015 mm. The pads
are 1.5 by 0.9. Lead 41 is the ground pad. Figure 11-1 draws a thermal grid
(3.7, 7.5, 10.29, 0.9) rather than one rectangle, so lead 41 is a marker at
the origin, not a measured land.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import gold, nickel, plastic
from cad.geom import box, fuse

BODY_X = 18.0
BODY_Y = 25.5
BODY_Z = 3.1
PCB_Z = 0.8  # Figure 10-1, side view
SHIELD_X = 15.8  # Figure 10-1
SHIELD_Y = 17.6  # Figure 10-1
SHIELD_FROM_END = 1.05  # Figure 10-1, the shield's edge to the far end
HOLE_D = 0.55  # Figure 10-1, 40 x Ø0.55, centred on the outline
TOP_PAD_DEEP = 0.45  # Figure 10-1, 40 x 0.45, in from the outline
TOP_PAD_ALONG = 0.9  # Figure 10-1, 40 x 0.9
PLATE = 0.04  # the sheet gives no plating thickness
PAD_ALONG = 0.9
PAD_DEEP = 1.5
PAD_T = 0.05
PITCH = 1.27
ROW_FROM_ANTENNA = 7.49
END_INSET = 2.015
# Figure 11-1: (18.0 - 17.5) / 2. The DevKit DXF lands sit on this inset.
EDGE_INSET = 0.25
PCB = plastic("#1A1A1A")
SHIELD = nickel("#C9CCD0")
PLATING = gold()


def _castellations():
    """(x, y, inward) for leads 1 to 40: the hole's centre on the outline and
    the unit vector from it into the body."""
    y1 = BODY_Y / 2.0 - ROW_FROM_ANTENNA
    holes = []
    for index in range(14):
        holes.append((-BODY_X / 2.0, y1 - index * PITCH, (1.0, 0.0)))
    x15 = -BODY_X / 2.0 + END_INSET
    for index in range(12):
        holes.append((x15 + index * PITCH, -BODY_Y / 2.0, (0.0, 1.0)))
    y27 = y1 - 13 * PITCH
    for index in range(14):
        holes.append((BODY_X / 2.0, y27 + index * PITCH, (-1.0, 0.0)))
    return holes


def _land(x, y, inward):
    """Figure 11-1's pad, 0.25 mm inside the outline, on the seating plane."""
    ix, iy = inward
    cx, cy = x + ix * EDGE_INSET, y + iy * EDGE_INSET
    sx, sy = (PAD_DEEP, PAD_ALONG) if ix else (PAD_ALONG, PAD_DEEP)
    return box(cx, cy, PAD_T / 2.0, sx, sy, PAD_T)


def _slab(x, y, inward, deep, along, z0, z1):
    """A box from the outline at (x, y) `deep` into the body, `along` the edge."""
    ix, iy = inward
    sx, sy = (deep, along) if ix else (along, deep)
    return box(x + ix * deep / 2.0, y + iy * deep / 2.0, (z0 + z1) / 2.0, sx, sy, z1 - z0)


def _hole(x, y, inward):
    """The half hole, cut square, through the PCB and its top pad."""
    return _slab(x, y, inward, HOLE_D / 2.0, HOLE_D, -0.1, PCB_Z + PLATE + 0.1)


def _castellation(x, y, inward):
    """The land, the plated back of the half hole and the pad on the PCB's top."""
    ix, iy = inward
    back = HOLE_D / 2.0
    wall = _slab(x + ix * (back - PLATE), y + iy * (back - PLATE), inward, PLATE, HOLE_D, 0.0, PCB_Z)
    top = _slab(x, y, inward, TOP_PAD_DEEP, TOP_PAD_ALONG, PCB_Z, PCB_Z + PLATE) - _hole(x, y, inward)
    return fuse([_land(x, y, inward), wall, top])


def _parts():
    holes = _castellations()
    pcb = box(0, 0, PCB_Z / 2.0, BODY_X, BODY_Y, PCB_Z)
    for x, y, inward in holes:
        pcb -= _hole(x, y, inward)
    shield_y = -BODY_Y / 2.0 + SHIELD_FROM_END + SHIELD_Y / 2.0
    shield_z = BODY_Z - PCB_Z
    shield = box(0, shield_y, PCB_Z + shield_z / 2.0, SHIELD_X, SHIELD_Y, shield_z)
    children = [paint(pcb, "pcb", PCB), paint(shield, "shield", SHIELD)]
    anchors = []
    for lead, (x, y, inward) in enumerate(holes, start=1):
        ix, iy = inward
        children.append(paint(_castellation(x, y, inward), f"lead-{lead}", PLATING))
        at = (x + ix * EDGE_INSET, y + iy * EDGE_INSET, 0.0)
        anchors.append(glb.Anchor(lead, f"lead-{lead}", at, kind="pad"))
    # The land drawing gives no copper thickness. 0.05 mm is the QFN helper's
    # terminal thickness, so each pad has a volume. It is not a module measurement.
    children.append(paint(box(0.0, 0.0, PAD_T / 2.0, PAD_ALONG, PAD_ALONG, PAD_T), "lead-41", PLATING))
    anchors.append(glb.Anchor(41, "lead-41", (0.0, 0.0, 0.0), kind="pad"))
    return children, anchors


def esp32s3_wroom_1_n16r8_shape():
    return bd.Compound(children=_parts()[0], label="esp32s3_wroom_1_n16r8")


def esp32s3_wroom_1_n16r8_anchors():
    return _parts()[1]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        esp32s3_wroom_1_n16r8_shape(),
        "esp32s3-wroom-1-n16r8",
        esp32s3_wroom_1_n16r8_anchors(),
        Path(sys.argv[1]),
    )
