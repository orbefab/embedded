# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Solderless breadboards: the body with its holes, rail stripes and print.

Frame: the breadboard's underside on z = 0 and its top at z = height,
centred on the origin in X and Y, columns along X. Columns count from 1 at
-X. Rows a–e sit on the -Y side of the centre channel, f–j on +Y, e and f
7.62 apart. Each side has two power rails, the outer one −, the inner one +,
with a blue line outside and a red line inside. The rails are rows W (−,
beside a), X (+, beside a), Y (+, beside j) and Z (−, beside j). A rail hole
lines up with a column and takes its number, so X20 sits beside a20; every
sixth column has no rail hole.

Every hole is a socket, lead n, cut into the body with no node of its own,
its anchor at the hole's mouth on the top face, axis +Z (out of the hole):

- terminal hole at row r and column c: n = (index of r in a–j) × columns + c,
  so a1 is 1, a2 is 2 and j<columns> is 10 × columns;
- then the rails W, X, Y and Z, each from -X: rail k (0 to 3), its hole i
  from -X (1-based) is 10 × columns + k × rail holes + i.
"""

import build123d as bd

from cad import glb, mark, paint
from cad.color import srgb
from cad.geom import box

PITCH = 2.54
ROWS = "abcdefghij"
RAILS = (("W", -1, "-"), ("X", -1, "+"), ("Y", 1, "+"), ("Z", 1, "-"))
CHANNEL = 7.62  # e to f
RAIL_INNER = 21.59  # |y| of the + rail  # verify
RAIL_OUTER = 24.13  # |y| of the − rail  # verify
HOLE = 1.0  # square opening  # verify
HOLE_DEPTH = 8.0  # how far a leg goes in  # verify
GROOVE = (2.6, 1.5)  # centre channel wide, deep  # verify
STRIPE = (0.6, 0.05)  # wide, tall
PRINT_SIZE = 1.2

_WHITE = srgb("#F2F1EC")
_RED = srgb("#D0342A")
_BLUE = srgb("#2F5FD0")
_PRINT = srgb("#5A5A5A")


def terminal_xy(columns, column, row):
    """Centre of the terminal-strip hole at `column` (1-based) and `row` (a–j)."""
    x = (column - (columns + 1) / 2.0) * PITCH
    k = ROWS.index(row)
    y = (k - 4) * PITCH - CHANNEL / 2.0 if k < 5 else (k - 5) * PITCH + CHANNEL / 2.0
    return x, y


def rail_columns(columns, groups):
    """The column each rail hole lines up with: groups of five with one empty
    column between, centred on the board, on the columns' grid."""
    span = groups * 6 - 1
    start = (columns - span) // 2 + 1
    return [start + g * 6 + k for g in range(groups) for k in range(5)]


def holes(columns, rail_groups):
    """(lead, name, x, y) of every hole, in lead order (see the module docstring)."""
    out = [
        (k * columns + c, f"{r}{c}", *terminal_xy(columns, c, r))
        for k, r in enumerate(ROWS)
        for c in range(1, columns + 1)
    ]
    rail = rail_columns(columns, rail_groups)
    for k, (row, side, polarity) in enumerate(RAILS):
        y = side * (RAIL_INNER if polarity == "+" else RAIL_OUTER)
        for i, c in enumerate(rail):
            x, _ = terminal_xy(columns, c, "a")
            out.append((10 * columns + k * len(rail) + i + 1, f"{row}{c}", x, y))
    return out


def anchors(columns, rail_groups, height):
    return [
        glb.Anchor(lead, None, (x, y, height), axis=(0.0, 0.0, 1.0), kind="socket")
        for lead, _, x, y in holes(columns, rail_groups)
    ]


def _print(x, y, z, text, size):
    """Printed text as one node of flat faces, lying `PRINT_T` above `z` so it never
    fights the top for depth, in a fixed order: a glyph in two pieces (i, j)
    otherwise comes out in either order, and the GLB with it."""
    faces = sorted(mark.print_face(x, y, z + mark.PRINT_T, text, size).faces(), key=lambda f: tuple(f.center()))
    return bd.Compound(faces) if len(faces) > 1 else faces[0]


def breadboard(columns, rail_groups, length, width, height):
    """Raw solids by name: the body with its holes, the stripes and the print.
    The caller paints them."""
    body = box(0, 0, height / 2.0, length, width, height)
    z = height - HOLE_DEPTH / 2.0
    cuts = [box(x, y, z, HOLE, HOLE, HOLE_DEPTH) for _, _, x, y in holes(columns, rail_groups)]
    gw, gd = GROOVE
    cuts.append(box(0, 0, height - gd / 2.0 + 0.05, length - 6.0, gw, gd + 0.1))
    shapes = {"body": body.cut(*cuts)}

    rail = rail_columns(columns, rail_groups)
    first, _ = terminal_xy(columns, rail[0], "a")
    last, _ = terminal_xy(columns, rail[-1], "a")
    middle = (first + last) / 2.0
    span = last - first + 4.0
    sw, st = STRIPE
    for side, tag in ((-1, "bottom"), (1, "top")):
        shapes[f"stripe-red-{tag}"] = box(middle, side * (RAIL_INNER - 1.9), height + st / 2.0, span, sw, st)
        shapes[f"stripe-blue-{tag}"] = box(middle, side * (RAIL_OUTER + 1.9), height + st / 2.0, span, sw, st)
        for end in (-1, 1):
            x = middle + end * (span / 2.0 + 1.6)
            shapes[f"print-plus-{tag}{end:+d}"] = _print(x, side * RAIL_INNER, height, "+", PRINT_SIZE * 1.5)
            shapes[f"print-minus-{tag}{end:+d}"] = _print(x, side * RAIL_OUTER, height, "-", PRINT_SIZE * 1.5)
    letter_x = (columns + 1) / 2.0 * PITCH + 0.4
    for r in ROWS:
        for end in (-1, 1):
            _, y = terminal_xy(columns, 1, r)
            shapes[f"print-{r}{end:+d}"] = _print(end * letter_x, y, height, r, PRINT_SIZE)
    numbers = [1, *range(5, columns + 1, 5)]
    for c in numbers:
        x, _ = terminal_xy(columns, c, "a")
        for side, tag in ((-1, "a"), (1, "j")):
            _, y = terminal_xy(columns, c, tag)
            shapes[f"print-col{c}-{tag}"] = _print(x, y + side * 2.3, height, str(c), PRINT_SIZE * 0.9)
    return shapes


def shape(columns, rail_groups, size, label):
    """The painted breadboard: white body, red and blue stripes, grey print."""
    length, width, height = size
    colors = {"body": _WHITE, "stripe-red": _RED, "stripe-blue": _BLUE}
    children = []
    for name, solid in breadboard(columns, rail_groups, length, width, height).items():
        color = next((c for prefix, c in colors.items() if name.startswith(prefix)), _PRINT)
        children.append(paint(solid, name, color))
    return bd.Compound(children=children, label=label)
