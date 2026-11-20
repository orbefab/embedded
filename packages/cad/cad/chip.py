# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Two-ended chip parts: 0603 resistors, capacitors and LEDs, tantalums, crystals.

A chip sits on its pads: body underside on z = 0, length along X, centred
on the origin. Each end cap wraps the end from the pad up to just proud of
the top; lead 1 is the cap at -X, lead 2 the cap at +X.
"""

from cad import glb, paint
from cad.color import plastic, tin
from cad.geom import box, cyl_z

TIN = tin()
CAP_PROUD = 0.03


def cap_length(length: float) -> float:
    return min(0.32, length * 0.28)


def chip(length, width, height, body, ends=TIN):
    """[body, lead-1, lead-2], painted and labelled."""
    cap = cap_length(length)
    offset = length / 2.0 - cap / 2.0
    tall = height + CAP_PROUD
    parts = [paint(box(0, 0, height / 2.0, length, width, height), "body", body)]
    for lead, sign in ((1, -1.0), (2, 1.0)):
        parts.append(
            paint(
                box(sign * offset, 0, tall / 2.0, cap, width * 0.92, tall),
                f"lead-{lead}",
                ends,
            )
        )
    return parts


_CERAMIC = plastic("#F3E6C4")
_DOME = plastic("#F7F5F2", alpha=0.4, roughness=0.08)


def led(length, width, height, lens):
    """A ceramic chip, a die in `lens`, and a clear dome named `body`.

    The end caps match `chip`, so the pad anchors stay. The dome is the
    lens a viewer tints and lights, and it is clear so the die shows through.
    """
    cap = cap_length(length)
    offset = length / 2.0 - cap / 2.0
    tall = height + CAP_PROUD
    ceramic_h = height * 0.62
    die_l = min(length * 0.28, 0.45)
    die_w = width * 0.36
    die_h = height * 0.22
    gap = length - 2.0 * cap
    radius = min(width * 0.42, gap * 0.42, width / 2.0 - 0.04)
    parts = [
        paint(box(0, 0, ceramic_h / 2.0, length, width, ceramic_h), "package", _CERAMIC),
        paint(box(0, 0, ceramic_h + die_h / 2.0, die_l, die_w, die_h), "die", lens),
        paint(cyl_z(0, 0, ceramic_h + die_h * 0.35, height, radius), "body", _DOME),
    ]
    for lead, sign in ((1, -1.0), (2, 1.0)):
        parts.append(paint(box(sign * offset, 0, tall / 2.0, cap, width * 0.92, tall), f"lead-{lead}", TIN))
    return parts


def chip_anchors(length):
    offset = length / 2.0 - cap_length(length) / 2.0
    return [
        glb.Anchor(lead, f"lead-{lead}", (sign * offset, 0.0, 0.0), kind="pad") for lead, sign in ((1, -1.0), (2, 1.0))
    ]
