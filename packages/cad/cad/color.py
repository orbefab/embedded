# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""sRGB hex colours as the linear channels a shape stores.

build123d reads ``Color`` channels as linear light and encodes sRGB when it
writes a GLB. ``srgb`` returns those linear channels for the hex to show.
Assign the result to ``shape.color``.
"""

from __future__ import annotations

_HEX = frozenset("0123456789abcdefABCDEF")


def _linear(byte: int) -> float:
    """One sRGB byte, 0 to 255, as linear light. IEC 61966-2-1."""
    encoded = byte / 255.0
    if encoded <= 0.04045:
        return encoded / 12.92
    return ((encoded + 0.055) / 1.055) ** 2.4


def srgb(hex_color: str, alpha: float = 1.0) -> tuple[float, float, float, float]:
    """Linear red, green, blue and alpha for an ``#RGB`` or ``#RRGGBB`` hex."""
    text = hex_color.strip()
    if text.startswith("#"):
        text = text[1:]
    if len(text) == 3 and all(ch in _HEX for ch in text):
        text = "".join(ch * 2 for ch in text)
    if len(text) != 6 or any(ch not in _HEX for ch in text):
        raise ValueError(f"{hex_color!r} is not an #RGB or #RRGGBB colour")
    red = int(text[0:2], 16)
    green = int(text[2:4], 16)
    blue = int(text[4:6], 16)
    return (_linear(red), _linear(green), _linear(blue), float(alpha))
