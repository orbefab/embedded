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
A socket cut into the body, such as a breadboard's hole, has no node.
"""

from __future__ import annotations

import json
import struct
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
    node: str | None
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


def _declared(node, found: list[tuple[tuple[float, float, float], tuple[float, float]]]) -> None:
    """The finish each coloured solid declared. A coloured solid with none fails."""
    if node.color is not None:
        factors = getattr(node, "orbe_factors", None)
        rgba = getattr(node, "orbe_rgba", None)
        if factors is None or rgba is None:
            raise RuntimeError(f"{getattr(node, 'label', None) or 'unlabelled'}: material has no declared finish")
        found.append((rgba[:3], factors))
    for child in node.children or ():
        _declared(child, found)


def _finish_table(rows: list[tuple[tuple[float, float, float], tuple[float, float]]]):
    """Map a linear base colour to its factors. The same colour with two finishes fails."""
    table: dict[tuple[float, float, float], tuple[float, float]] = {}
    for rgb, factors in rows:
        key = tuple(round(channel, 5) for channel in rgb)
        previous = table.get(key)
        if previous is not None and previous != factors:
            raise RuntimeError(f"one colour {key} declares two finishes, {previous} and {factors}")
        table[key] = factors
    return table


def _nearest(color, table):
    best = None
    best_gap = None
    for key, factors in table.items():
        gap = max(abs(color[i] - key[i]) for i in range(3))
        if best_gap is None or gap < best_gap:
            best, best_gap = factors, gap
    if best is None or best_gap is None or best_gap > 2e-3:
        return None
    return best


def _stamp_factors(glb: Path, table) -> None:
    """Write metallicFactor and roughnessFactor onto every material.

    OCCT's writer omits both, and a missing metallicFactor is 1 in glTF, so
    every surface would load as metal. The colour bytes stay as the writer
    emitted them; only the two factors are added.
    """
    data = glb.read_bytes()
    if data[:4] != b"glTF":
        raise RuntimeError(f"{glb.name}: not a GLB")
    json_len, json_type = struct.unpack_from("<II", data, 12)
    if json_type != 0x4E4F534A:
        raise RuntimeError(f"{glb.name}: first chunk is not JSON")
    doc = json.loads(data[20 : 20 + json_len])
    materials = doc.get("materials") or []
    if not materials:
        raise RuntimeError(f"{glb.name}: GLB has no materials")
    for mat in materials:
        pbr = mat.setdefault("pbrMetallicRoughness", {})
        raw = pbr.get("baseColorFactor", [1.0, 1.0, 1.0, 1.0])
        factors = _nearest(raw, table)
        if factors is None:
            raise RuntimeError(f"{glb.name}: material {mat.get('name')!r} {raw} has no declared finish")
        pbr["metallicFactor"], pbr["roughnessFactor"] = factors
    new_json = json.dumps(doc, separators=(",", ":")).encode()
    pad = (4 - (len(new_json) % 4)) % 4
    new_json += b" " * pad
    rest = data[20 + json_len :]
    total = 12 + 8 + len(new_json) + len(rest)
    out = struct.pack("<III", 0x46546C67, 2, total)
    out += struct.pack("<II", len(new_json), 0x4E4F534A)
    out += new_json
    out += rest
    glb.write_bytes(out)


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
    declared: list[tuple[tuple[float, float, float], tuple[float, float]]] = []
    _declared(shape, declared)
    if not declared:
        raise RuntimeError(f"{part}: no coloured solid declared a finish")
    _stamp_factors(glb, _finish_table(declared))
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
