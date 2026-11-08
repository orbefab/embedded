# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Colours for part models.

build123d treats a ``Color``'s channels as linear RGB and converts them to
sRGB on the way to the screen. ``Color(0.5, 0.5, 0.5)`` does not display as
``#808080``. It displays as roughly ``#BCBCBC``.

``srgb`` returns the linear channel tuple for the hex you want to see.
Assign it to ``shape.color``. Style every leaf. A colour on a group compound
never reaches the mesh.
"""

from __future__ import annotations

__all__ = ["srgb", "srgb_to_linear", "linear_to_srgb"]


def srgb_to_linear(channel: float) -> float:
    """Convert one 0..1 sRGB channel to linear RGB."""
    if channel <= 0.04045:
        return channel / 12.92
    return ((channel + 0.055) / 1.055) ** 2.4


def linear_to_srgb(channel: float) -> float:
    """Inverse of :func:`srgb_to_linear`."""
    if channel <= 0.0031308:
        return channel * 12.92
    return 1.055 * (channel ** (1 / 2.4)) - 0.055


def _parse_hex(value: str) -> tuple[int, int, int]:
    text = value.strip().lstrip("#")
    if len(text) == 3:
        text = "".join(ch * 2 for ch in text)
    if len(text) != 6:
        raise ValueError(f"expected a #rgb or #rrggbb colour, got {value!r}")
    try:
        return int(text[0:2], 16), int(text[2:4], 16), int(text[4:6], 16)
    except ValueError as exc:
        raise ValueError(f"expected a hex colour, got {value!r}") from exc


def srgb(value: str, alpha: float = 1.0) -> tuple[float, float, float, float]:
    """The linear RGBA channels for the sRGB hex to show.

    ``value`` is ``#rgb`` or ``#rrggbb``. ``alpha`` is 0..1. Assign the result
    to ``shape.color``. The result is a plain tuple, so a palette constant can
    be spelled at import without loading the geometry kernel.
    """
    red, green, blue = _parse_hex(value)
    return (
        srgb_to_linear(red / 255.0),
        srgb_to_linear(green / 255.0),
        srgb_to_linear(blue / 255.0),
        float(alpha),
    )
