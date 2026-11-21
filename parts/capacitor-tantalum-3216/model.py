# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""3216 tantalum chip capacitor, 3.2 × 1.6: the Nano's 10 µF bulk capacitors.

Part frame: on its pads, body underside on z = 0, length along X, centred
on the origin. Lead 1 (pin pos, the anode) is the end
cap at -X, lead 2 (pin neg) the one at +X.

KEMET T491, case A (EIA 3216-18): 3.2 × 1.6 × 1.6, each ±0.2. The Nano's
two bulk capacitors share this height.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.chip import cap_length, chip, chip_anchors
from cad.color import plastic
from cad.geom import box

SIZE = (3.2, 1.6, 1.6)  # length, width, height: KEMET T491 case A (EIA 3216-18)

_BLACK = plastic("#1A1A1A")
_STRIPE = plastic("#D5D8DE")


def capacitor_tantalum_3216_shape():
    length, width, height = SIZE
    cap = cap_length(length)
    # The anode is lead 1, at -X. The stripe sits on the body just inboard of that cap.
    stripe = box(-length / 2.0 + cap + 0.2, 0, height + 0.015, 0.24, width * 0.7, 0.04)
    return bd.Compound(
        children=[*chip(*SIZE, _BLACK), paint(stripe, "stripe", _STRIPE)],
        label="capacitor_tantalum_3216",
    )


def capacitor_tantalum_3216_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        capacitor_tantalum_3216_shape(),
        "capacitor-tantalum-3216",
        capacitor_tantalum_3216_anchors(),
        Path(sys.argv[1]),
    )
