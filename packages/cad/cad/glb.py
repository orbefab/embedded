# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""A part's GLB and its anchors file, for viewers that wire to its leads.

`write` exports the labelled shape as `<folder>/<part>.glb` and its leads as
`<folder>/<part>.anchors.json`. Both are named by the part's id, because the
viewer looks parts up by id.

The GLB is standard glTF: Y-up in metres, with glTF's -90° turn about X on
its one root node, so the vertex data keeps the part frame's axes. Every
label in the shape is a node name, and each node's mesh holds one primitive
per colour, not one per B-rep face.

The anchors file is in the part frame: millimetres, +Z up, seated at z = 0.
Each anchor is one lead: its number, the node that draws it (and any others
that belong to it), the lead's tip, the direction a lead is inserted, and
its kind (`leg` goes into a hole, `socket` takes a pin, `pad` is soldered).

A part with a screen names one node `screen` and passes its `Screen`: the
panel's active area in the part frame, its top edge toward +Y. `write`
gives that node's vertices TEXCOORD_0 so that (0, 0) is the active area's
top-left corner, u runs along +X and v along -Y, as a panel's pixels do;
the viewer can lay the panel's pixels on it as a texture.
"""

from __future__ import annotations

import json
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Literal
from unittest.mock import patch

import build123d as bd
import numpy as np

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


@dataclass(frozen=True)
class Screen:
    node: str
    centre: tuple[float, float]  # the active area's middle, part frame
    size: tuple[float, float]  # its width along X and height along Y


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


def _transform(node) -> np.ndarray:
    if "matrix" in node:
        return np.array(node["matrix"], dtype=float).reshape(4, 4).T
    x, y, z, w = node.get("rotation", (0.0, 0.0, 0.0, 1.0))
    m = np.eye(4)
    m[:3, :3] = np.array(
        [
            [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
        ]
    ) @ np.diag(node.get("scale", (1.0, 1.0, 1.0)))
    m[:3, 3] = node.get("translation", (0.0, 0.0, 0.0))
    return m


def _add_screen_uv(path: Path, screen: Screen) -> None:
    """Gives the screen node's vertices TEXCOORD_0 on the active area."""
    data = path.read_bytes()
    json_len = struct.unpack_from("<I", data, 12)[0]
    doc = json.loads(data[20 : 20 + json_len])
    bin_len = struct.unpack_from("<I", data, 20 + json_len)[0]
    blob = bytearray(data[28 + json_len : 28 + json_len + bin_len])

    (root,) = doc["scenes"][doc.get("scene", 0)]["nodes"]
    found = []

    def walk(index, matrix):
        node = doc["nodes"][index]
        world = matrix @ _transform(node)
        if node.get("name") == screen.node and "mesh" in node:
            found.append((node["mesh"], world))
        for child in node.get("children", ()):
            walk(child, world)

    for child in doc["nodes"][root].get("children", ()):
        walk(child, np.eye(4))
    if len(found) != 1:
        raise ValueError(f"{path.name}: screen node {screen.node} must be one mesh")
    mesh, world = found[0]
    (cx, cy), (w, h) = screen.centre, screen.size
    for prim in doc["meshes"][mesh]["primitives"]:
        acc = doc["accessors"][prim["attributes"]["POSITION"]]
        view = doc["bufferViews"][acc["bufferView"]]
        stride = view.get("byteStride", 12)
        start = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
        raw = np.frombuffer(bytes(blob), np.uint8, stride * acc["count"], start)
        local = raw.reshape(acc["count"], stride)[:, :12].copy().view("<f4").reshape(-1, 3)
        mm = (local.astype(float) @ world[:3, :3].T + world[:3, 3]) * 1000.0
        uv = np.stack(
            [(mm[:, 0] - (cx - w / 2.0)) / w, ((cy + h / 2.0) - mm[:, 1]) / h], axis=1
        ).astype("<f4")
        blob.extend(b"\0" * (-len(blob) % 4))
        doc["bufferViews"].append(
            {"buffer": 0, "byteOffset": len(blob), "byteLength": uv.nbytes, "target": 34962}
        )
        blob.extend(uv.tobytes())
        doc["accessors"].append(
            {
                "bufferView": len(doc["bufferViews"]) - 1,
                "componentType": 5126,
                "count": int(acc["count"]),
                "type": "VEC2",
                "min": [float(v) for v in uv.min(axis=0)],
                "max": [float(v) for v in uv.max(axis=0)],
            }
        )
        prim["attributes"]["TEXCOORD_0"] = len(doc["accessors"]) - 1
    blob.extend(b"\0" * (-len(blob) % 4))
    doc["buffers"][0]["byteLength"] = len(blob)
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    out = struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(text) + 8 + len(blob))
    out += struct.pack("<I4s", len(text), b"JSON") + text
    out += struct.pack("<I4s", len(blob), b"BIN\0") + bytes(blob)
    path.write_bytes(out)


def _merged_writer():
    """OCCT's glTF writer with each mesh's faces merged by material.

    build123d's export_gltf does not expose the option. Left per face, a
    breadboard's 830 holes write some 40 000 accessors, most of its GLB.
    The writer also runs single-threaded. Its parallel path does not repeat
    the same bytes for the same shape.
    """
    from OCP.RWGltf import RWGltf_CafWriter

    class MergedWriter(RWGltf_CafWriter):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            self.SetMergeFaces(True)

        def SetParallel(self, *args, **kwargs):
            super().SetParallel(False)

    return MergedWriter


def write(
    shape,
    part: str,
    anchors: list[Anchor] | None,
    folder: Path,
    screen: Screen | None = None,
) -> None:
    """`anchors` is None for a part with no leads of its own, such as a bare board."""
    leads = sorted(a.lead for a in anchors or ())
    if leads != list(range(1, len(leads) + 1)):
        raise ValueError(f"{part}: leads must be 1 to {len(leads)}, once each: {leads}")
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    glb = folder / f"{part}.glb"
    from build123d import exporters3d

    _linear_colors(shape)
    with patch.object(exporters3d, "RWGltf_CafWriter", _merged_writer()):
        bd.export_gltf(
            shape,
            str(glb),
            binary=True,
            linear_deflection=LINEAR_DEFLECTION,
            angular_deflection=ANGULAR_DEFLECTION,
        )
    if screen is not None:
        _add_screen_uv(glb, screen)
    if anchors is None:
        print(f"wrote {glb}")
        return
    rows = ",\n".join(
        "    " + json.dumps(a.as_json(), ensure_ascii=False)
        for a in sorted(anchors, key=lambda a: a.lead)
    )
    path = folder / f"{part}.anchors.json"
    path.write_text(
        f'{{\n  "part": {json.dumps(part)},\n  "units": "mm",\n  "anchors": [\n{rows}\n  ]\n}}\n'
    )
    print(f"wrote {glb} and {path.name}")
