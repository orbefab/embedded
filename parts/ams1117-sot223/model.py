# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""AMS1117-5.0 regulator, SOT-223: on the Nano's underside.

Part frame: on the board at z = 0, upright, the body centred on the
origin. The tab is at -X and the three leads at +X. Seen from the top,
lead 1 (GND) is at -Y, lead 2 (OUT) in the middle and lead 3 (IN) at +Y;
the tab is lead 4, OUT again. On the Nano it hangs under the board,
turned half a turn about X.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import srgb
from cad.geom import box

SOT223_BODY = (3.5, 6.5, 1.6)  # x, y, height
SOT223_BODY_Z = 0.05  # body underside above the board
SOT223_TAB = (2.1, 3.3, 0.42, -2.45, 0.07)  # x, y, thick, centre x, underside z
SOT223_LEAD = (1.15, 0.65, 0.2, 2.15, 0.06)  # x, y, thick, centre x, underside z
SOT223_PITCH = 2.3

_BLACK = srgb("#1A1A1A")
_TIN = srgb("#C5C8CE")


def _pads():
    lx, ly, lt, lcx, lz = SOT223_LEAD
    tx, ty, tt, tcx, tz = SOT223_TAB
    leads = [(n, (lcx, (n - 2) * SOT223_PITCH, lz), (lx, ly, lt)) for n in (1, 2, 3)]
    return [*leads, (4, (tcx, 0.0, tz), (tx, ty, tt))]


def ams1117_sot223_shape():
    bx, by, bh = SOT223_BODY
    children = [paint(box(0, 0, SOT223_BODY_Z + bh / 2.0, bx, by, bh), "body", _BLACK)]
    for lead, (x, y, z), (sx, sy, sz) in _pads():
        children.append(paint(box(x, y, z + sz / 2.0, sx, sy, sz), f"lead-{lead}", _TIN))
    return bd.Compound(children=children, label="ams1117_sot223")


def ams1117_sot223_anchors():
    return [glb.Anchor(lead, f"lead-{lead}", at, kind="pad") for lead, at, _ in _pads()]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        ams1117_sot223_shape(),
        "ams1117-sot223",
        ams1117_sot223_anchors(),
        Path(sys.argv[1]),
        merge_faces=False,
    )
