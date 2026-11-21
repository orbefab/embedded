# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""USB micro-B socket, surface mount.

The schematic that uses this names no maker, so the shell is a representative
right-angle receptacle. Dimensions marked verify are not from one drawing.

Part frame: the shell standing on the board's top at z = 0, centred on the
origin, its mouth toward -X. Five pads leave the back at +X on 0.65 along
Y, lead 1 (VBUS) at -Y to lead 5 (GND) at +Y. Leads 6 to 9 are shell feet,
6 and 7 beside the mouth and 8 and 9 beside the back, odd numbers at -Y.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box

USB_W = 7.5  # verify
USB_H = 2.8  # verify
USB_L = 5.6  # verify
USB_MOUTH = (4.0, 6.4, 1.6)  # depth, width, height  # verify
USB_PAD = (1.2, 0.35, 0.12)  # length out of the back, width, thickness  # verify
USB_PAD_PITCH = 0.65
USB_SHELL = (1.0, 0.9, 0.2)  # x, y, thickness  # verify
USB_SHELLS = (
    (6, -1, -1),
    (7, -1, 1),
    (8, 1, -1),
    (9, 1, 1),
)  # lead, end in x, side in y

_TIN = tin()
_DARK = plastic("#2A2E33")
_TONGUE = plastic("#F2F1EC")


def _pad_xy(lead):
    return USB_L / 2.0 + USB_PAD[0] / 2.0, (lead - 3) * USB_PAD_PITCH


def _shell_xy(sx, sy):
    return sx * (USB_L / 2.0 - 0.7), sy * (USB_W / 2.0 + USB_SHELL[1] / 2.0)


def usb_micro_b_shape():
    z = USB_H / 2.0
    front = -USB_L / 2.0
    shell = box(0, 0, z, USB_L, USB_W, USB_H)
    depth, mouth_w, mouth_h = USB_MOUTH
    cutter = box(front - 0.2 + depth / 2.0, 0, z + 0.15, depth + 0.4, mouth_w, mouth_h)
    children = [
        paint(shell - cutter, "body", _TIN),
        paint(box(front + depth - 0.3, 0, z + 0.15, 0.35, mouth_w - 0.4, mouth_h - 0.3), "mouth", _DARK),
        paint(box(front + 0.8, 0, z - 0.25, 1.8, 5.2, 0.3), "tongue", _TONGUE),
    ]
    length, width, thick = USB_PAD
    for lead in range(1, 6):
        children.append(paint(box(*_pad_xy(lead), thick / 2.0, length, width, thick), f"lead-{lead}", _TIN))
    sx, sy, st = USB_SHELL
    for lead, ex, ey in USB_SHELLS:
        children.append(paint(box(*_shell_xy(ex, ey), st / 2.0, sx, sy, st), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="usb_micro_b")


def usb_micro_b_anchors():
    pads = [glb.Anchor(n, f"lead-{n}", (*_pad_xy(n), 0.0), kind="pad") for n in range(1, 6)]
    feet = [glb.Anchor(n, f"lead-{n}", (*_shell_xy(ex, ey), 0.0), kind="pad") for n, ex, ey in USB_SHELLS]
    return pads + feet


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(usb_micro_b_shape(), "usb-micro-b", usb_micro_b_anchors(), Path(sys.argv[1]))
