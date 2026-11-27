# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""32 x 32 x 8 mm 8 x 8 red LED dot matrix package (BL-M12A881 / 1088AS).

Part frame: underside is z = 0, centred on the origin, top face at z = 8.0 mm.
From Betlux BL-M12A881 Rev V.2 p. 4: 32.00 x 32.00 x 8.00 mm body, 8 x 8 array
of 3.0 mm diameter round LED dots on a 4.00 mm pitch.

`glass` is a 32 x 32 mm emissive sheet just beneath a 0.15 mm matte-black
faceplate perforated with 64 3.0 mm circular apertures centred at
x = -14 + 4 * col, y = 14 - 4 * row (row 0 = DIG0 at +Y, col 0 = DP at -X).
Three.js `planarUv` maps the 8 x 8 `DataTexture` 1:1 into those 64 apertures.
"""

import sys
from pathlib import Path

import build123d as bd

from cad import glb, paint
from cad.color import copper, plastic
from cad.geom import box, cyl_z

SIZE = 32.0
HEIGHT = 8.0
PITCH = 4.0
DOT_D = 3.0
FACE_T = 0.15
GLASS_T = 0.05
PAD = (0.8, 1.2)
PAD_T = 0.03
LEADS = tuple(range(1, 8))

_HOUSING = plastic("#D8D8D2", roughness=0.55)
_FACE = plastic("#16171A", roughness=0.45)
_GLASS = plastic("#242222", roughness=0.35)
_COPPER = copper()


def _pad_xy(lead: int) -> tuple[float, float]:
    return ((lead - 4) * 2.54, 0.0)


def _faceplate() -> bd.Shape:
    z0 = HEIGHT - FACE_T
    z1 = HEIGHT
    plate = box(0, 0, (z0 + z1) / 2.0, SIZE, SIZE, FACE_T)
    holes = []
    for row in range(8):
        y = 14.0 - row * PITCH
        for col in range(8):
            x = -14.0 + col * PITCH
            holes.append(cyl_z(x, y, z0 - 0.1, z1 + 0.1, DOT_D / 2.0))
    return plate - bd.Compound(children=holes)


def max7219_8x8_shape() -> bd.Compound:
    glass_z0 = HEIGHT - FACE_T - GLASS_T
    body_h = glass_z0 - PAD_T
    children = [
        paint(box(0, 0, PAD_T + body_h / 2.0, SIZE, SIZE, body_h), "housing", _HOUSING),
        paint(box(0, 0, glass_z0 + GLASS_T / 2.0, SIZE, SIZE, GLASS_T), "glass", _GLASS),
        paint(_faceplate(), "faceplate", _FACE),
    ]
    for lead in LEADS:
        x, y = _pad_xy(lead)
        children.append(paint(box(x, y, PAD_T / 2.0, PAD[0], PAD[1], PAD_T), f"lead-{lead}", _COPPER))
    return bd.Compound(children=children, label="max7219_8x8")


def max7219_8x8_anchors() -> list[glb.Anchor]:
    return [glb.Anchor(lead, f"lead-{lead}", (*_pad_xy(lead), 0.0), kind="pad") for lead in LEADS]


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        max7219_8x8_shape(),
        "max7219-8x8",
        max7219_8x8_anchors(),
        Path(sys.argv[1]),
    )
