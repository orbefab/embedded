# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""2.54 mm pin headers and socket strips.

A length is a parameter, so a 1×6 and a 1×15 are one shape. Every part
sits with its plastic underside on z = 0, centred on the origin, contacts
along X and lead 1 at -X, except the 2×3 block, whose pins stand up from
the top of one housing.
"""

import build123d as bd

from cad import glb, paint
from cad.color import gold, plastic, tin
from cad.geom import box, cyl_z
from cad.tht import socket_liner

PITCH = 2.54
PIN = 0.64
PLASTIC = 2.54  # verify
HOUSING_R = 1.12
PIN_BELOW = 8.0  # verify, tip below the plastic's underside
PIN_ABOVE = 3.0  # verify, the tail above the plastic, through a 1.6 board
ICSP_PIN_ABOVE = 3.2  # verify

STRIP_HEIGHT = 8.5
STRIP_SOCKET = (1.0, 6.0)  # square opening, deep  # verify
STRIP_WALL = 0.1
STRIP_TAIL = (0.64, 3.0)  # square, below z = 0  # verify

_BLACK = plastic("#1A1A1A")
_TIN = tin()
_GOLD = gold()


def _x(lead, count):
    return (lead - 1 - (count - 1) / 2.0) * PITCH


def row_shape(count):
    """One round housing per pin. Each pin runs `PIN_BELOW` under the
    plastic and stands `PIN_ABOVE` proud of its top."""
    top = PLASTIC + PIN_ABOVE
    children = []
    for lead in range(1, count + 1):
        x = _x(lead, count)
        children.append(paint(cyl_z(x, 0, 0, PLASTIC, HOUSING_R), f"housing-{lead}", _BLACK))
        children.append(
            paint(
                box(x, 0, (top - PIN_BELOW) / 2.0, PIN, PIN, top + PIN_BELOW),
                f"pin-{lead}",
                _TIN,
            )
        )
    return bd.Compound(children=children, label=f"pin_header_1x{count}")


def row_anchors(count):
    return [
        glb.Anchor(lead, f"pin-{lead}", (_x(lead, count), 0.0, -PIN_BELOW), also=(f"housing-{lead}",))
        for lead in range(1, count + 1)
    ]


def _icsp_xy(lead):
    """Leads 1 and 2 across the +Y row (1 at -X), then 3 and 4, then 5 and 6 at -Y."""
    col, row = (lead - 1) % 2, (lead - 1) // 2
    return (col - 0.5) * PITCH, (1 - row) * PITCH


def block_2x3_shape():
    """One block. Pins stand `ICSP_PIN_ABOVE` above its top."""
    children = [paint(box(0, 0, PLASTIC / 2.0, 2 * PITCH, 3 * PITCH, PLASTIC), "housing", _BLACK)]
    for lead in range(1, 7):
        children.append(
            paint(
                box(*_icsp_xy(lead), PLASTIC + ICSP_PIN_ABOVE / 2.0, PIN, PIN, ICSP_PIN_ABOVE),
                f"pin-{lead}",
                _TIN,
            )
        )
    return bd.Compound(children=children, label="pin_header_2x3")


def block_2x3_anchors():
    return [
        glb.Anchor(
            lead,
            f"pin-{lead}",
            (*_icsp_xy(lead), PLASTIC + ICSP_PIN_ABOVE),
            axis=(0.0, 0.0, 1.0),
        )
        for lead in range(1, 7)
    ]


def socket_strip_shape(count):
    """A housing `STRIP_HEIGHT` tall. Each contact is a liner `socket-<n>`
    and a tail `tail-<n>` through the board."""
    opening, deep = STRIP_SOCKET
    tail, below = STRIP_TAIL
    hole = opening + 2.0 * STRIP_WALL
    housing = box(0, 0, STRIP_HEIGHT / 2.0, count * PITCH, PITCH, STRIP_HEIGHT)
    housing = housing.cut(
        *[box(_x(n, count), 0, STRIP_HEIGHT - deep / 2.0, hole, hole, deep) for n in range(1, count + 1)]
    )
    children = [paint(housing, "housing", _BLACK)]
    for n in range(1, count + 1):
        x = _x(n, count)
        children.append(paint(socket_liner(x, 0, STRIP_HEIGHT, opening, deep, STRIP_WALL), f"socket-{n}", _GOLD))
        children.append(paint(box(x, 0, -below / 2.0, tail, tail, below), f"tail-{n}", _TIN))
    return bd.Compound(children=children, label=f"socket_strip_1x{count}")


def socket_strip_anchors(count):
    return [
        glb.Anchor(
            n,
            f"socket-{n}",
            (_x(n, count), 0.0, STRIP_HEIGHT),
            axis=(0.0, 0.0, 1.0),
            kind="socket",
            also=(f"tail-{n}",),
        )
        for n in range(1, count + 1)
    ]
