# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Stock bits of hobby module boards: screw terminals, pin headers, SMD chips.

Every helper builds in a local frame with the board's top face on z = 0
and returns plain solids; the caller places, labels and paints them. Pins
and leads stop `tail` below the board's bottom; the caller cuts them out of
its board.
"""

import build123d as bd

from cad.geom import box, cyl_z, fuse

PITCH = 2.54
PIN = 0.64
SANS = "Liberation Sans"
PRINT_T = 0.02  # silkscreen and part markings

# KF301 / DG301 5.0 mm screw terminal, as on the L298N module.
TERMINAL_PITCH = 5.0
TERMINAL_DEPTH = 7.75
TERMINAL_H = 10.0
TERMINAL_SCREW_IN = 3.38  # screw axis from the wire-entry face


def text(x, y, z, s, size, rotation=0.0, bold=True, thickness=PRINT_T):
    """Printed text centred on (x, y), standing `thickness` up from z."""
    with bd.BuildSketch() as sketch:
        bd.Text(
            s,
            font_size=size,
            font=SANS,
            font_style=bd.FontStyle.BOLD if bold else bd.FontStyle.REGULAR,
            rotation=rotation,
        )
    return bd.Pos(x, y, z) * bd.extrude(sketch.sketch, amount=thickness)


def screw_terminal(
    poles,
    board_t,
    tail=1.5,
    pitch=TERMINAL_PITCH,
    depth=TERMINAL_DEPTH,
    height=TERMINAL_H,
    screw_in=TERMINAL_SCREW_IN,
    pin_d=1.0,
):
    """Poles along X, centred on x = 0; screw axes on y = 0, wire entries on the -Y face.

    Returns (body, screws, pins), screws and pins in pole order from -X.
    """
    d, h = depth, height
    length = poles * pitch
    profile = bd.Polygon(
        (0, 0), (d, 0), (d, h - 3.0), (d - 2.0, h), (1.0, h), (0, h - 3.0), align=None
    )
    body = bd.Pos(-length / 2.0, -screw_in, 0) * bd.extrude(bd.Plane.YZ * profile, length)
    screws, pins = [], []
    for k in range(poles):
        x = (k - (poles - 1) / 2.0) * pitch
        body -= box(
            x,
            -screw_in + (d - 1.0) / 2.0 - 0.05,
            2.45,
            min(4.5, pitch - 0.5),
            d - 0.9,
            3.7,
        )
        body -= cyl_z(x, 0.0, h - 2.5, h + 0.1, 1.75)
        screw = cyl_z(x, 0.0, h - 2.5, h - 0.5, 1.65)
        screw -= box(x, 0.0, h - 0.75, 2.9, 0.5, 0.6) + box(x, 0.0, h - 0.75, 0.5, 2.9, 0.6)
        screws.append(screw)
        pins.append(cyl_z(x, 0.0, -board_t - tail, 0.0, pin_d / 2.0))
        if k:
            body -= box(x - pitch / 2.0, 0.0, h - 1.5, 0.5, 4.5, 3.1)
    return body, screws, pins


def pin(x, y, board_t, top, tail=1.5):
    """A square header pin from `tail` under the board to `top`."""
    z0 = -board_t - tail
    return box(x, y, (z0 + top) / 2.0, PIN, PIN, top - z0)


def header(positions, board_t, plastic=2.5, above=6.0, tail=1.5, pitch=PITCH):
    """A straight male header: one plastic block per (x, y) position, pins up.

    Returns (housings, pins) in the order given; each housing has its pin cut out.
    """
    housings, pins = [], []
    for x, y in positions:
        p = pin(x, y, board_t, plastic + above, tail)
        housing = box(x, y, plastic / 2.0, pitch, pitch, plastic) - p
        housings.append(housing)
        pins.append(p)
    return fuse(housings).clean(), pins


def header_row(n, board_t, plastic=2.5, above=6.0, tail=1.5, pitch=PITCH):
    """n pins along X, centred on the origin."""
    xs = [(k - (n - 1) / 2.0) * pitch for k in range(n)]
    return header([(x, 0.0) for x in xs], board_t, plastic, above, tail, pitch)


def right_angle_row(n, board_t, reach=6.0, tail=1.5, pitch=PITCH, inset=1.0):
    """n right-angle pins along X at y = 0, pointing toward -Y past the plastic.

    The plastic stands on the board between y = -inset - pitch and y = -inset.
    Returns (housing, pins), pins from -X.
    """
    zc = pitch / 2.0
    y0, y1 = -inset - pitch, -inset
    length = n * pitch
    housing = box(0.0, (y0 + y1) / 2.0, zc, length, pitch, pitch)
    pins = []
    h = PIN / 2.0
    for k in range(n):
        x = (k - (n - 1) / 2.0) * pitch
        arm = box(x, (y0 - reach + h) / 2.0, zc, PIN, h - (y0 - reach), PIN)
        leg = box(x, 0.0, (-board_t - tail + zc + h) / 2.0, PIN, PIN, zc + h + board_t + tail)
        whole = fuse([arm, leg]).clean()
        housing -= whole
        pins.append(whole)
    return housing, pins


def jumper_cap(x0, y0, x1, y1, plastic=2.5, height=6.5):
    """A jumper cap over two adjacent pins, sitting on their plastic. Cut the pins out of it."""
    along_x = abs(x1 - x0) > abs(y1 - y0)
    sx, sy = (5.0, 2.4) if along_x else (2.4, 5.0)
    return box((x0 + x1) / 2.0, (y0 + y1) / 2.0, plastic + height / 2.0, sx, sy, height)


def chip(x, y, length, width, height, along_x=True, cap=None):
    """A two-terminal SMD part (resistor, capacitor, LED, diode): (body, ends)."""
    cap = min(0.35, length * 0.2) if cap is None else cap
    ends = []
    if along_x:
        body = box(x, y, height / 2.0, length - 2 * cap, width, height)
        for s in (-1, 1):
            ends.append(box(x + s * (length - cap) / 2.0, y, height / 2.0, cap, width, height))
    else:
        body = box(x, y, height / 2.0, width, length - 2 * cap, height)
        for s in (-1, 1):
            ends.append(box(x, y + s * (length - cap) / 2.0, height / 2.0, width, cap, height))
    return body, ends


def gullwing(x, y, length, width, height, pitch, per_side, span, standoff=0.1, lead_w=None):
    """A gull-wing IC (SOIC, SOP, SOT, TSSOP), rows of leads along X: (body, leads).

    `length` × `width` is the body, `span` the width over the leads' feet.
    """
    lead_w = pitch * 0.4 if lead_w is None else lead_w
    body = box(x, y, standoff + height / 2.0, length, width, height)
    shoulder = standoff + min(0.9, height / 2.0)
    leads = []
    for k in range(per_side):
        lx = x + (k - (per_side - 1) / 2.0) * pitch
        for s in (-1, 1):
            leads.append(_gull_lead(lx, y, s, width, span, shoulder, lead_w))
    return body, leads


def sot23(x, y, along_x=True):
    """SOT-23: leads 1 and 2 on the -Y side 1.9 apart, lead 3 alone on +Y: (body, leads)."""
    body = box(x, y, 0.1 + 0.5, 2.9, 1.3, 1.0)
    leads = [
        _gull_lead(x + dx, y, s, 1.3, 2.4, 0.6, 0.4, foot=0.4)
        for dx, s in ((-0.95, -1), (0.95, -1), (0.0, 1))
    ]
    if not along_x:
        turn = bd.Pos(x, y, 0) * bd.Rot(0, 0, 90) * bd.Pos(-x, -y, 0)
        body, leads = turn * body, [turn * lead for lead in leads]
    return body, leads


def sot223(x, y):
    """SOT-223 (AMS1117 and the like): three leads on 2.3 toward -Y, the tab toward +Y: (body, leads, tab)."""
    body = box(x, y, 0.1 + 0.8, 6.5, 3.5, 1.6)
    leads = [_gull_lead(x + dx, y, -1, 3.5, 7.0, 0.9, 0.7, foot=0.9) for dx in (-2.3, 0.0, 2.3)]
    tab = _gull_lead(x, y, 1, 3.5, 7.0, 0.9, 3.0, foot=0.9)
    return body, leads, tab


def _gull_lead(lx, y, s, width, span, shoulder, lead_w, foot=0.6):
    edge, tip = y + s * width / 2.0, y + s * span / 2.0
    knee = tip - s * foot
    arm = box(
        lx, (edge + knee) / 2.0 + s * 0.1, shoulder + 0.1, lead_w, abs(knee - edge) + 0.2, 0.2
    )
    drop = box(lx, knee + s * 0.1, (0.15 + shoulder + 0.01) / 2.0, lead_w, 0.2, shoulder - 0.14)
    sole = box(lx, (knee + tip) / 2.0, 0.1, lead_w, abs(tip - knee), 0.2)
    return fuse([arm, drop, sole]).clean()


def dip(
    x,
    y,
    pins_per_side,
    row=7.62,
    length=None,
    width=6.35,
    height=3.3,
    standoff=0.5,
    tail=1.5,
    board_t=1.6,
):
    """A DIP package on the board, rows along X, `row` apart: (body, leads), leads from pin 1.

    Pin 1 is at -X on the -Y row; numbering runs counterclockwise seen from above.
    """
    length = pins_per_side * PITCH if length is None else length
    body = box(x, y, standoff + height / 2.0, length, width, height)
    body -= cyl_z(x - length / 2.0, y, standoff + height - 0.4, standoff + height + 0.1, 0.8)
    leads = []
    order = [(k, -1) for k in range(pins_per_side)] + [
        (k, 1) for k in reversed(range(pins_per_side))
    ]
    for k, s in order:
        lx = x + (k - (pins_per_side - 1) / 2.0) * PITCH
        ly = y + s * row / 2.0
        shoulder = box(
            lx,
            (y + s * width / 2.0 + ly) / 2.0,
            standoff + 1.0,
            1.5,
            abs(ly - y - s * width / 2.0) + 0.25,
            0.25,
        )
        leg = box(
            lx,
            ly,
            (standoff + 1.125 - board_t - tail) / 2.0,
            0.5,
            0.25,
            standoff + 1.125 + board_t + tail,
        )
        leads.append(fuse([shoulder, leg]).clean())
    return body, leads
