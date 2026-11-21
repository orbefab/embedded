# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""sRGB hex colours as the linear channels a shape stores, with a declared finish.

build123d reads ``Color`` channels as linear light and encodes sRGB when it
writes a GLB. ``srgb`` returns those linear channels for the hex to show.
The finish is the model's, not a guess from the colour: a painted mark can
be the same grey as a tin lead and still be plastic.

OCCT's glTF writer leaves out ``metallicFactor`` and ``roughnessFactor``.
``glb.write`` stamps the factors this module declares onto every material.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum

_HEX = frozenset("0123456789abcdefABCDEF")


class Finish(Enum):
    """What a surface is. ``METAL`` is tin, steel, nickel or aluminium."""

    PLASTIC = "plastic"
    METAL = "metal"
    GOLD = "gold"
    COPPER = "copper"


# metallicFactor, roughnessFactor. Plastic covers epoxy, ceramic, solder mask,
# paint and glass. Metal is a dull machined or plated surface, gold a smoother
# plate, copper a touch rougher than that plate.
_FACTORS: dict[Finish, tuple[float, float]] = {
    Finish.PLASTIC: (0.0, 0.52),
    Finish.METAL: (1.0, 0.36),
    Finish.GOLD: (1.0, 0.32),
    Finish.COPPER: (1.0, 0.40),
}


@dataclass(frozen=True)
class Surface:
    """Linear RGBA plus the finish ``glb.write`` stamps onto the material."""

    rgba: tuple[float, float, float, float]
    finish: Finish
    roughness: float | None = None

    def factors(self) -> tuple[float, float]:
        metallic, roughness = _FACTORS[self.finish]
        if self.roughness is not None:
            roughness = self.roughness
        return (metallic, roughness)


def _linear(byte: int) -> float:
    """One sRGB byte, 0 to 255, as linear light. IEC 61966-2-1."""
    encoded = byte / 255.0
    if encoded <= 0.04045:
        return encoded / 12.92
    return ((encoded + 0.055) / 1.055) ** 2.4


def srgb(hex_color: str, alpha: float = 1.0, *, finish: Finish, roughness: float | None = None) -> Surface:
    """A surface for an ``#RGB`` or ``#RRGGBB`` hex.

    ``finish`` is required. A call that omits it fails, so a material cannot
    reach the GLB as the glTF default, which is metal.
    """
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
    return Surface((_linear(red), _linear(green), _linear(blue), float(alpha)), finish, roughness)


def plastic(hex_color: str, alpha: float = 1.0, *, roughness: float | None = None) -> Surface:
    """Epoxy, ceramic, solder mask, paint, glass or any other non-metal."""
    return srgb(hex_color, alpha, finish=Finish.PLASTIC, roughness=roughness)


def metal(hex_color: str, alpha: float = 1.0) -> Surface:
    """Tin, steel, nickel or aluminium."""
    return srgb(hex_color, alpha, finish=Finish.METAL)


def tin(hex_color: str = "#C5C8CE", alpha: float = 1.0) -> Surface:
    return metal(hex_color, alpha)


def steel(hex_color: str, alpha: float = 1.0) -> Surface:
    return metal(hex_color, alpha)


def nickel(hex_color: str, alpha: float = 1.0) -> Surface:
    return metal(hex_color, alpha)


def gold(hex_color: str = "#D9B44A", alpha: float = 1.0) -> Surface:
    return srgb(hex_color, alpha, finish=Finish.GOLD)


def copper(hex_color: str = "#B87333", alpha: float = 1.0) -> Surface:
    return srgb(hex_color, alpha, finish=Finish.COPPER)
