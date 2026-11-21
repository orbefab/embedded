# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""28-pin 300 mil DIP socket: the ATmega328P's socket on the Uno.

Part frame: the socket's underside on z = 0, its top at z = 4.3, centred
on the origin, rows along X, its notch at -X. Contacts count like the chip
they take (`cad.ic.dual_row`): 1 at the -X end of the -Y row, 1 to 14
toward +X and 15 to 28 back along +Y. Each contact is a socket: a liner
node `socket-<n>` round its opening, anchored at the mouth on the top face
with its axis +Z, and a tail `tail-<n>` running 3.0 below z = 0, listed
with it.

The common open-frame dual-wipe socket: 35.6 × 10.1 × 4.3, rows 7.62
apart on 2.54 (`# verify` all sizes).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, ic, paint
from cad.color import gold, plastic, tin
from cad.geom import box
from cad.tht import socket_liner

SOCKET_BODY = (35.6, 10.1, 4.3)  # long, wide, tall  # verify
ROW = 7.62
PITCH = 2.54
OPENING = (1.0, 3.8)  # square, deep  # verify
WALL = 0.1
TAIL = (0.5, 3.0)  # square, below z = 0  # verify
WINDOW = (30.0, 3.0)  # the open frame's window, long, wide  # verify

_BLACK = plastic("#1A1A1A")
_GOLD = gold()
_TIN = tin()


def _contacts():
    return [(n, x, side * ROW / 2.0) for n, x, side in ic.dual_row(28, PITCH)]


def dip_socket_28_shape():
    length, width, height = SOCKET_BODY
    opening, deep = OPENING
    tail, below = TAIL
    hole = opening + 2.0 * WALL
    body = box(0, 0, height / 2.0, length, width, height)
    cuts = [box(x, y, height - deep / 2.0, hole, hole, deep) for _, x, y in _contacts()]
    cuts.append(box(0, 0, height / 2.0, *WINDOW, height + 0.2))
    cuts.append(box(-length / 2.0, 0, height - 0.5, 1.2, 1.6, 1.2))
    children = [paint(body.cut(*cuts), "body", _BLACK)]
    for n, x, y in _contacts():
        children.append(paint(socket_liner(x, y, height, opening, deep, WALL), f"socket-{n}", _GOLD))
        children.append(paint(box(x, y, -below / 2.0, tail, tail, below), f"tail-{n}", _TIN))
    return bd.Compound(children=children, label="dip_socket_28")


def dip_socket_28_anchors():
    top = SOCKET_BODY[2]
    return [
        glb.Anchor(n, f"socket-{n}", (x, y, top), axis=(0.0, 0.0, 1.0), kind="socket", also=(f"tail-{n}",))
        for n, x, y in _contacts()
    ]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(dip_socket_28_shape(), "dip-socket-28", dip_socket_28_anchors(), Path(sys.argv[1]))
