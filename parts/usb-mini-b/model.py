# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""USB mini-B socket, surface mount: the Nano's USB connector.

Part frame: the shell standing on the board's top at z = 0, centred on the
origin, its mouth toward -X. Five pads leave the back at +X on 0.8 along
Y, lead 1 (VBUS) at -Y to lead 5 (GND) at +Y.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

USB_W = 7.7  # verify
USB_H = 4.0  # verify
USB_L = 8.0  # verify
USB_MOUTH = (4.4, 6.1, 2.2)  # depth, width, height
USB_PAD = (1.2, 0.5, 0.1)  # length out of the back, width, thickness  # verify
USB_PAD_PITCH = 0.8  # verify

_TIN = srgb("#C5C8CE")
_DARK = srgb("#2A2E33")


def _pad_xy(lead):
    return USB_L / 2.0 + USB_PAD[0] / 2.0, (lead - 3) * USB_PAD_PITCH


def usb_mini_b_shape():
    z = USB_H / 2.0
    front = -USB_L / 2.0
    shell = box(0, 0, z, USB_L, USB_W, USB_H)
    for sign in (-1.0, 1.0):
        shell += box(front + 1.6, sign * (USB_W / 2.0 + 0.15), 0.35, 2.2, 0.8, 0.7)
    depth, mouth_w, mouth_h = USB_MOUTH
    cutter = box(front - 0.2 + depth / 2.0, 0, z + 0.2, depth + 0.4, mouth_w, mouth_h)
    children = [
        paint(shell - cutter, "body", _TIN),
        paint(box(front + depth - 0.35, 0, z + 0.2, 0.4, mouth_w - 0.5, mouth_h - 0.4), "mouth", _DARK),
        paint(box(front + 0.9, 0, z - 0.35, 2.2, 3.6, 0.4), "tongue", _TIN),
    ]
    length, width, thick = USB_PAD
    for lead in range(1, 6):
        children.append(paint(box(*_pad_xy(lead), thick / 2.0, length, width, thick), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="usb_mini_b")


def usb_mini_b_anchors():
    return [glb.Anchor(n, f"lead-{n}", (*_pad_xy(n), 0.0), kind="pad") for n in range(1, 6)]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(usb_mini_b_shape(), "usb-mini-b", usb_mini_b_anchors(), Path(sys.argv[1]))
