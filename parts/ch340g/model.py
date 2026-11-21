# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""WCH CH340G USB to serial, SOIC-16: on the Nano's underside.

Part frame: on the board at z = 0, upright, centred on the origin, the pin
rows along X. Leads count counter-clockwise from the top: 1 to 8 along +Y
from +X, 9 to 16 back along -Y. The dot marks lead 1. On the Nano it hangs
under the board, turned half a turn about X.

SOIC-16 outline: body 9.9 × 3.9, 1.75 tall, leads 0.4 wide and 1.2 long on
1.27, 6.3 tip to tip. Each lead is a gull wing; the foot stays the pad
whose underside the anchor names. The pin-1 mark is paint, not the lead metal.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box, formed_foot

SOIC_L = 9.9
SOIC_W = 3.9
SOIC_H = 1.75
SOIC_PITCH = 1.27
SOIC_LEAD = (0.4, 1.2, 0.18)  # wide, out, thick
SOIC_LEAD_Y = 2.55
SOIC_LEAD_Z = 0.12  # lead centre

_BLACK = plastic("#1A1A1A")
_DOT = plastic("#8A8D93")
_TIN = tin()


def _lead_xy(lead):
    side, k = divmod(lead - 1, 8)
    if side == 0:
        return (3.5 - k) * SOIC_PITCH, SOIC_LEAD_Y
    return (k - 3.5) * SOIC_PITCH, -SOIC_LEAD_Y


def ch340g_shape():
    children = [
        paint(box(0, 0, SOIC_H / 2.0, SOIC_L, SOIC_W, SOIC_H), "body", _BLACK),
        paint(box(4.15, 1.15, SOIC_H + 0.05, 0.5, 0.5, 0.12), "dot", _DOT),
    ]
    wide, out, thick = SOIC_LEAD
    for lead in range(1, 17):
        x, y = _lead_xy(lead)
        # The body is toward y = 0, so a +Y lead bends toward -Y.
        children.append(
            paint(
                formed_foot(
                    x, y, SOIC_LEAD_Z, wide, out, thick, axis="y", sign=-1.0 if y > 0 else 1.0, rise=0.5, tuck=0.32
                ),
                f"lead-{lead}",
                _TIN,
            )
        )
    return bd.Compound(children=children, label="ch340g")


def ch340g_anchors():
    bottom = SOIC_LEAD_Z - SOIC_LEAD[2] / 2.0
    return [glb.Anchor(n, f"lead-{n}", (*_lead_xy(n), bottom), kind="pad") for n in range(1, 17)]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(ch340g_shape(), "ch340g", ch340g_anchors(), Path(sys.argv[1]))
