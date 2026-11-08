# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Printed markings on a part. The letters are Liberation Sans outlines."""

import build123d as bd

SANS = "Liberation Sans"
PRINT_T = 0.02


def text(x, y, z, s, size, rotation=0.0, bold=True, thickness=PRINT_T):
    """Text centred on (x, y), standing `thickness` up from z."""
    with bd.BuildSketch() as sketch:
        bd.Text(
            s,
            font_size=size,
            font=SANS,
            font_style=bd.FontStyle.BOLD if bold else bd.FontStyle.REGULAR,
            rotation=rotation,
        )
    return bd.Pos(x, y, z) * bd.extrude(sketch.sketch, amount=thickness)
