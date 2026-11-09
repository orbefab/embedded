# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""A part's GLB and its anchors file, for viewers that wire to its leads.

`write` exports the labelled shape as `<folder>/<part>.glb` and its leads as
`<folder>/<part>.anchors.json`. Both are named by the part's id, because the
viewer looks parts up by id. Faces of one colour in a node join into one
primitive.

The GLB is standard glTF: Y-up in metres, with glTF's -90° turn about X on
its one root node, so the vertex data keeps the part frame's axes. Every
label in the shape is a node name, and each node's mesh holds one primitive
per colour.

The anchors file is in the part frame: millimetres, +Z up, seated at z = 0.
Each anchor is one lead: its number, the node that draws it (and any others
that belong to it), the lead's tip, the direction a lead is inserted, and
its kind (`leg` goes into a hole, `socket` takes a pin, `pad` is soldered).
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Literal
from unittest.mock import patch

import build123d as bd

DOWN = (0.0, 0.0, -1.0)
LINEAR_DEFLECTION = 0.01
ANGULAR_DEFLECTION = 0.3


@dataclass(frozen=True)
class Anchor:
    lead: int
    node: str
    at: tuple[float, float, float]
    axis: tuple[float, float, float] = DOWN
    kind: Literal["leg", "socket", "pad"] = "leg"
    also: tuple[str, ...] = ()

    def as_json(self) -> dict:
        return {
            "lead": self.lead,
            "node": self.node,
            "also": list(self.also),
            "at": [_mm(v) for v in self.at],
            "axis": [_mm(v) for v in self.axis],
            "kind": self.kind,
        }


def _mm(value: float) -> float:
    return round(float(value), 6) + 0.0


def _linear_colors(node) -> None:
    """Give the glTF writer the linear channels `srgb` chose.

    build123d reads a colour's floats as sRGB and linearises them, and `srgb`
    already returns linear values, so without this every GLB colour is
    linearised twice and comes out too dark.
    """
    from OCP.Quantity import Quantity_Color, Quantity_ColorRGBA, Quantity_TypeOfColor

    if node.color is not None:
        rgba = node.color.wrapped
        r, g, b = rgba.GetRGB().Values(Quantity_TypeOfColor.Quantity_TOC_sRGB)
        linear = Quantity_Color(r, g, b, Quantity_TypeOfColor.Quantity_TOC_RGB)
        node.color = bd.Color(Quantity_ColorRGBA(linear, rgba.Alpha()))
    for child in node.children or ():
        _linear_colors(child)


def _writer():
    """OCCT's glTF writer, single-threaded, joining faces of one colour.

    build123d's export_gltf does not expose face merging. Left per face, a
    breadboard's 830 holes write some 40 000 accessors, most of its GLB.
    The parallel path does not repeat the same bytes.
    """
    from OCP.RWGltf import RWGltf_CafWriter

    class Writer(RWGltf_CafWriter):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            self.SetMergeFaces(True)

        def SetParallel(self, *args, **kwargs):
            super().SetParallel(False)

    return Writer


def write(shape, part: str, anchors: list[Anchor] | None, folder: Path) -> None:
    """`anchors` is None for a part with no leads of its own, such as a bare board."""
    leads = sorted(a.lead for a in anchors or ())
    if leads != list(range(1, len(leads) + 1)):
        raise ValueError(f"{part}: leads must be 1 to {len(leads)}, once each: {leads}")
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    glb = folder / f"{part}.glb"
    from build123d import exporters3d

    _linear_colors(shape)
    with patch.object(exporters3d, "RWGltf_CafWriter", _writer()):
        wrote = bd.export_gltf(
            shape,
            str(glb),
            binary=True,
            linear_deflection=LINEAR_DEFLECTION,
            angular_deflection=ANGULAR_DEFLECTION,
        )
    if not wrote:
        raise RuntimeError(f"{part}: export_gltf returned false for {glb.name}")
    if anchors is None:
        print(f"wrote {glb}")
        return
    rows = ",\n".join(
        "    " + json.dumps(a.as_json(), ensure_ascii=False) for a in sorted(anchors, key=lambda a: a.lead)
    )
    path = folder / f"{part}.anchors.json"
    path.write_text(
        f'{{\n  "part": {json.dumps(part)},\n  "units": "mm",\n  "anchors": [\n{rows}\n  ]\n}}\n',
        encoding="utf-8",
        newline="\n",
    )
    print(f"wrote {glb} and {path.name}")
