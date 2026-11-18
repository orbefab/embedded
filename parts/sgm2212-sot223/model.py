# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""SG Micro SGM2212-3.3, SOT-223-3.

Part frame: on the board at z = 0, upright, the body centred on the
origin. The tab is at -X and the three leads at +X. Seen from the top,
lead 1 (GND) is at -Y, lead 2 (VOUT) in the middle and lead 3 (VIN) at
+Y. The tab is lead 4, VOUT again. The same seating as `ams1117-sot223`.

From SG Micro SGM2212, JULY 2023, outline TX00048.000. Each size below is
the midpoint of the printed range, except L, which the table prints only
as 0.75 minimum, and e, which is 2.300 basic. E1's midpoint is 7.0 over
the tab and the lead feet, so with the body 3.5 wide and a 0.75 foot the
tab is 2.75 long and its tip is at x = -4.50.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

SOT223_BODY = (3.5, 6.5, 1.6)  # E, D, A2, midpoints
SOT223_BODY_Z = 0.06  # midpoint of A1, 0.020 to 0.100
# x, y, thick, centre x, underside z. L is the printed minimum.
SOT223_LEAD = (0.75, 0.75, 0.29, 2.125, 0.0)  # L, b, c
SOT223_TAB = (2.75, 3.0, 0.29, -3.125, 0.0)  # length, b1, c, centre x
SOT223_PITCH = 2.3

_BLACK = srgb("#1A1A1A")
_TIN = srgb("#C5C8CE")


def _pads():
    lx, ly, lt, lcx, lz = SOT223_LEAD
    tx, ty, tt, tcx, tz = SOT223_TAB
    leads = [(n, (lcx, (n - 2) * SOT223_PITCH, lz), (lx, ly, lt)) for n in (1, 2, 3)]
    return [*leads, (4, (tcx, 0.0, tz), (tx, ty, tt))]


def sgm2212_sot223_shape():
    bx, by, bh = SOT223_BODY
    children = [paint(box(0, 0, SOT223_BODY_Z + bh / 2.0, bx, by, bh), "body", _BLACK)]
    for lead, (x, y, z), (sx, sy, sz) in _pads():
        children.append(paint(box(x, y, z + sz / 2.0, sx, sy, sz), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="sgm2212_sot223")


def sgm2212_sot223_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", at, kind="pad") for lead, at, _ in _pads()]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        sgm2212_sot223_shape(),
        "sgm2212-sot223",
        sgm2212_sot223_anchors(),
        Path(sys.argv[1]),
    )
