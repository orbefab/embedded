# Copyright (c) 2026 Orbefab. SPDX-License-Identifier: MIT OR Apache-2.0
"""3 mm yellow LED. The body is the shared T-1 outline; this file sets the tint."""

import sys
from pathlib import Path

from cad import glb
from cad.tht import LED_3MM, round_led

PART = "led-3mm-yellow"
TINT = "#f2c12e"

if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(__file__).name} <output-folder>")
    shape, leads = round_led(PART, LED_3MM, TINT)
    anchors = [glb.Anchor(lead, f"lead-{lead}", (x, 0.0, -below)) for lead, x, below in leads]
    glb.write(shape, PART, anchors, Path(sys.argv[1]))
