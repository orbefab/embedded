# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""Printed markings on a part. The letters are the bundled Liberation Sans outlines."""

from io import BytesIO
from pathlib import Path

import build123d as bd
from OCP.Message import Message, Message_Gravity, Message_PrinterToReport, Message_Report

PRINT_T = 0.02
_FONTS = Path(__file__).resolve().parents[1] / "fonts"
_SANS = {
    True: _FONTS / "LiberationSans-Bold.ttf",
    False: _FONTS / "LiberationSans-Regular.ttf",
}
_report: Message_Report | None = None


def _font_report() -> Message_Report:
    global _report
    if _report is None:
        _report = Message_Report()
        printer = Message_PrinterToReport()
        printer.SetReport(_report)
        Message.DefaultMessenger_s().AddPrinter(printer)
        _report.UpdateActiveInMessenger()
    _report.Clear()
    return _report


def _refuse_font_fallback(report: Message_Report) -> None:
    # OCCT substitutes another face and only warns, which would change the outlines.
    buf = BytesIO()
    report.Dump(buf, Message_Gravity.Message_Warning)
    lines = [line for line in buf.getvalue().decode().splitlines() if "unable to find font" in line]
    if lines:
        raise RuntimeError("\n".join(lines))


def text(x, y, z, s, size, rotation=0.0, bold=True, thickness=PRINT_T):
    """Text centred on (x, y), standing `thickness` up from z."""
    path = _SANS[bold]
    if not path.is_file():
        raise FileNotFoundError(f"bundled font missing: {path}")
    report = _font_report()
    with bd.BuildSketch() as sketch:
        bd.Text(
            s,
            font_size=size,
            font_path=path,
            font_style=bd.FontStyle.BOLD if bold else bd.FontStyle.REGULAR,
            rotation=rotation,
        )
    _refuse_font_fallback(report)
    return bd.Pos(x, y, z) * bd.extrude(sketch.sketch, amount=thickness)
