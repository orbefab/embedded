# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Helpers for part models."""

from cad.color import Surface


def paint(shape, label, color):
    """Name a solid and give it a declared finish. Returns the solid.

    ``color`` is a ``Surface`` from ``cad.color``. A bare tuple has no finish,
    and the export refuses it rather than writing glTF's default metal.
    """
    if not isinstance(color, Surface):
        raise TypeError(f"{label}: colour needs a declared finish, such as plastic() or tin()")
    shape.label = label
    shape.color = color.rgba
    shape.finish = color.finish
    shape.orbe_rgba = color.rgba
    shape.orbe_factors = color.factors()
    return shape
