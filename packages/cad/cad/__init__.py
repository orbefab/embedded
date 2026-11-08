# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Helpers for part models."""


def paint(shape, label, color):
    """Name a solid and give it a colour. Returns the solid."""
    shape.label = label
    shape.color = color
    return shape
