# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""3-position PCB screw terminal on a 5.0 mm pitch.

Part frame: the block's underside is z = 0, centred on the origin, the clamps
along X with lead 1 at -X. Each wire entry faces +Y and each screw is on top.
The anchor is the mouth of a clamp, where a wire or a jumper's pin goes in.
The pins under the block are not drawn: on a module they are under the board.
The sizes are a common 5.0 mm terminal block's (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import nickel, plastic
from cad.geom import box, cyl_z

COUNT = 3
PITCH = 5.0
BODY = (COUNT * PITCH, 7.6, 10.0)  # along X, deep, tall  # verify
ENTRY = (3.2, 3.2, 3.6)  # wide, tall, centre above the underside  # verify
CLAMP = (1.6, 4.0)  # square opening, deep
SCREW = (4.6, 3.8, 1.6)  # well diameter, head diameter, head below the top  # verify

_BLUE = plastic("#1E64C8")
_METAL = nickel("#B9BEC6")


def _x(lead):
    return (lead - (COUNT + 1) / 2.0) * PITCH


def screw_terminal_3p_5mm_anchors():
    _, deep, _ = BODY
    return [
        glb.Anchor(n, f"clamp-{n}", (_x(n), deep / 2.0, ENTRY[2]), axis=(0.0, 1.0, 0.0), kind="socket")
        for n in range(1, COUNT + 1)
    ]


def _clamp(lead):
    """A square tube from the entry's mouth into the block, lined in metal."""
    _, deep, _ = BODY
    wide, tall, z = ENTRY
    opening, depth = CLAMP
    y = deep / 2.0 - depth / 2.0
    outer = box(_x(lead), y, z, wide, depth, tall)
    return outer - box(_x(lead), y, z, opening, depth + 0.2, opening)


def screw_terminal_3p_5mm_shape():
    long, deep, tall = BODY
    wide, high, z = ENTRY
    _, depth = CLAMP
    well, head, sink = SCREW
    body = box(0, 0, tall / 2.0, long, deep, tall)
    for n in range(1, COUNT + 1):
        body = body - box(_x(n), deep / 2.0 - depth / 2.0, z, wide, depth + 0.01, high)
        body = body - cyl_z(_x(n), -0.6, tall - sink - 0.5, tall + 0.1, well / 2.0)
    # Grooves between the positions, as on a block of single terminals.
    for n in range(1, COUNT):
        body = body - box(_x(n) + PITCH / 2.0, 0, tall - 1.0, 0.6, deep + 0.2, 2.2)
    children = [paint(body, "body", _BLUE)]
    for n in range(1, COUNT + 1):
        screw = cyl_z(_x(n), -0.6, tall - sink - 0.5, tall - sink, head / 2.0)
        screw = screw - box(_x(n), -0.6, tall - sink - 0.2, head + 0.2, 0.7, 0.7)
        children.append(paint(screw, f"screw-{n}", _METAL))
        children.append(paint(_clamp(n), f"clamp-{n}", _METAL))
    return bd.Compound(children=children, label="screw_terminal_3p_5mm")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        screw_terminal_3p_5mm_shape(),
        "screw-terminal-3p-5mm",
        screw_terminal_3p_5mm_anchors(),
        Path(sys.argv[1]),
    )
