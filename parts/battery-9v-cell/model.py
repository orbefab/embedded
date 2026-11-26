# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Internal cell block seated inside battery-9v."""

import sys
from pathlib import Path

import build123d as bd

from cad import glb
from cad.chip import chip, chip_anchors
from cad.color import plastic

SIZE = (4.0, 2.0, 2.0)

_CELL = plastic("#2A2C30")


def battery_9v_cell_shape():
    return bd.Compound(children=chip(*SIZE, _CELL), label="battery_9v_cell")


def battery_9v_cell_anchors():
    return chip_anchors(SIZE[0])


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    glb.write(
        battery_9v_cell_shape(),
        "battery-9v-cell",
        battery_9v_cell_anchors(),
        Path(sys.argv[1]),
    )
