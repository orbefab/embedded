# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Two-ended chip parts: 0603 resistors, capacitors and LEDs, tantalums, crystals.

A chip sits on its pads: body underside on z = 0, length along X, centred
on the origin. Each end cap wraps the end from the pad up to just proud of
the top; lead 1 is the cap at -X, lead 2 the cap at +X.
"""

from cad import glb, paint
from cad.color import tin
from cad.geom import box

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


def chip_anchors(length):
    offset = length / 2.0 - cap_length(length) / 2.0
    return [
        glb.Anchor(lead, f"lead-{lead}", (sign * offset, 0.0, 0.0), kind="pad") for lead, sign in ((1, -1.0), (2, 1.0))
    ]
