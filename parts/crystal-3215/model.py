# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""3.2 × 1.5 two-pad SMD crystal: the 12 MHz crystal beside the Nano's CH340G.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 is the end at -X, lead 2 the one
at +X.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.chip import CAP_PROUD, cap_length, chip_anchors
from cad.color import plastic, tin
from cad.geom import box

SIZE = (3.2, 1.5, 0.7)  # length, width, height  # verify height

_CERAMIC = plastic("#E6D3B0")
_TIN = tin()


def crystal_3215_shape():
    """A ceramic base, a metal lid, and a metal cap at each end.

    The caps match `chip`, so the pad anchors stay. The lid is inset from
    those caps. The overall envelope stays `SIZE`.
    """
    length, width, height = SIZE
    cap = cap_length(length)
    offset = length / 2.0 - cap / 2.0
    base_h = height * 0.4
    lid_l = length - 2.0 * cap - 0.12
    children = [
        paint(box(0, 0, base_h / 2.0, length, width, base_h), "body", _CERAMIC),
        paint(box(0, 0, base_h + (height - base_h) / 2.0, lid_l, width * 0.86, height - base_h), "lid", _TIN),
    ]
    tall = height + CAP_PROUD
    for lead, sign in ((1, -1.0), (2, 1.0)):
        children.append(paint(box(sign * offset, 0, tall / 2.0, cap, width * 0.92, tall), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="crystal_3215")


def crystal_3215_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(crystal_3215_shape(), "crystal-3215", crystal_3215_anchors(), Path(sys.argv[1]))
