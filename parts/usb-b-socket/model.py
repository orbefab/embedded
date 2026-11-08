# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""USB 2.0 type-B socket, through-hole, right angle: the Uno's USB socket.

Part frame: the socket's underside (the seating plane) on z = 0, centred on
the origin in X and Y, the cable opening on the -X face. Leads 1 (VBUS),
2 (D-), 3 (D+) and 4 (GND) are the USB pin numbers. Leads 5 and 6 are the
shell's two tabs, 5 at +Y. Each leg is anchored at its tip, 3.0 below the
seat.

The signal pins sit on a 2.5 × 2.0 grid at the back, and the shell tabs
are 12.0 apart. The body, 15.75 × 12.0 × 10.9, and the opening are the
common type-B socket (`# verify`).
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

BODY = (15.75, 12.0, 10.9)  # long, wide, tall  # verify
OPENING = (8.45, 7.78, 9.0)  # wide, tall, deep  # verify
TONGUE = (3.0, 4.2, 8.0)  # wide, tall, deep  # verify
LEGS = {  # lead: x, y, (wide along x, wide along y)
    1: (6.985, -1.25, (0.6, 0.6)),
    2: (6.985, 1.25, (0.6, 0.6)),
    3: (4.985, 1.25, (0.6, 0.6)),
    4: (4.985, -1.25, (0.6, 0.6)),
    5: (2.275, 6.0, (1.6, 0.3)),
    6: (2.275, -6.0, (1.6, 0.3)),
}
BELOW = 3.0

_SHELL = srgb("#C9CCD1")
_PLASTIC = srgb("#F2F1EC")
_TIN = srgb("#C5C8CE")


def usb_b_socket_shape():
    length, width, height = BODY
    opening_w, opening_h, opening_d = OPENING
    tongue_w, tongue_h, tongue_d = TONGUE
    zc = height / 2.0
    shell = box(0, 0, zc, length, width, height) - box(
        -length / 2.0 + opening_d / 2.0 - 0.05, 0, zc, opening_d + 0.1, opening_w, opening_h
    )
    tongue = box(-length / 2.0 + opening_d - tongue_d / 2.0, 0, zc, tongue_d, tongue_w, tongue_h)
    children = [paint(shell, "housing", _SHELL), paint(tongue, "tongue", _PLASTIC)]
    for lead, (x, y, (sx, sy)) in LEGS.items():
        children.append(paint(box(x, y, -BELOW / 2.0, sx, sy, BELOW), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="usb_b_socket")


def usb_b_socket_anchors():
    return [glb.Anchor(n, f"lead-{n}", (x, y, -BELOW)) for n, (x, y, _) in LEGS.items()]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(usb_b_socket_shape(), "usb-b-socket", usb_b_socket_anchors(), Path(sys.argv[1]))
