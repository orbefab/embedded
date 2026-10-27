# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fit the LM1117-3.3 row the regulator examples use.

    uv run scripts/fit_regulator.py

The table is TI LM1117, SNOS412Q, revised January 2023. VOUT, line
regulation and load regulation are p5. Dropout, current limit and quiescent
current are p6. Note 3 on p5 measures line and load regulation at constant
junction room temperature, and the fit uses that table. Note 4 on p6 defines
dropout as the input-to-output differential where VOUT has fallen 100 mV
from its value at VIN = VOUT + 1.5 V. The AMS1117 defines dropout as a 1%
drop, 33 mV at 3.3 V. This script does not fit that sheet.
"""

import math
import sys

VOUT = 3.3
LINE_MV = 0.001
LINE_SPAN = 15.0 - 4.75
K = LINE_MV / LINE_SPAN
LOAD_MV = 0.001
LOAD_SPAN = 0.8
RO = LOAD_MV / LOAD_SPAN
DROP = [(0.1, 1.10), (0.5, 1.15), (0.8, 1.20)]
RDO = 0.1 / 0.7
VDO = 1.10 - RDO * 0.1
ILIM = 1.2
IQ = 5e-3
RTH = 61.6
POWER = (125.0 - 25.0) / RTH
POWER_ABSOLUTE = (150.0 - 25.0) / RTH
KNEE_V = 1e-3
FLOOR_OHM = 0.25
LIMIT_FRACTION = 1e-3
LIMIT_HEADROOM = 100.0
BLOCK_OHM = 1e9
BLOCK_A = 1e-12
GMIN = 1e-12
GROUND_RATIO = 0.0
RATING_CURRENT = 0.8
NOTE4_VIN = VOUT + 1.5


def sp(x):
    if x > 0.0:
        return x + math.log1p(math.exp(-x))
    return math.log1p(math.exp(x))


def smin(a, b):
    return a - KNEE_V * sp((a - b) / KNEE_V)


def smooth(x):
    if x <= 0.0:
        return 0.0
    if x >= 1.0:
        return 1.0
    return x * x * (3.0 - 2.0 * x)


def vo(vi, i):
    vset = VOUT + K * (vi - VOUT)
    a = vset - RO * i
    raw = vi - VDO - RDO * i
    floor = -FLOOR_OHM * i
    b = floor + KNEE_V * sp((raw - floor) / KNEE_V)
    d = LIMIT_FRACTION * ILIM
    rlim = LIMIT_HEADROOM * VOUT / ILIM
    return (
        smin(a, b)
        - rlim * d * sp((i - ILIM) / d)
        + BLOCK_OHM * BLOCK_A * sp(-i / BLOCK_A)
    )


def ig(vi, i):
    return (IQ + GROUND_RATIO * i) * smooth(vi / VDO)


def solve_current(vi, target):
    i = ILIM
    for _ in range(40):
        got = vo(vi, i) - target
        step = 1e-8
        slope = (vo(vi, i + step) - vo(vi, i - step)) / (2.0 * step)
        i -= got / slope
    return i


def open_circuit(vi):
    """The pass current that gmin on the output carries."""
    i = GMIN * VOUT
    for _ in range(20):
        got = vo(vi, i)
        i = GMIN * got
    return i, vo(vi, i)


def dropout(i):
    """Note 4. The reference input is VOUT + 1.5 V."""
    reference = vo(NOTE4_VIN, i)
    target = reference - 0.1
    lo, hi = target, NOTE4_VIN
    for _ in range(60):
        mid = 0.5 * (lo + hi)
        if vo(mid, i) < target:
            lo = mid
        else:
            hi = mid
    vin = 0.5 * (lo + hi)
    return vin - vo(vin, i)


def main():
    print(f"output_voltage {VOUT!r}")
    print(f"line_regulation {K!r}")
    print(f"output_resistance {RO!r}")
    print(f"dropout_voltage {VDO!r}")
    print(f"dropout_resistance {RDO!r}")
    print(f"current_limit {ILIM!r}")
    print(f"quiescent_current {IQ!r}")
    print(f"floor_ohm {FLOOR_OHM!r}")
    print(f"knee_v {KNEE_V!r}")
    print(f"limit_fraction {LIMIT_FRACTION!r}")
    print(f"limit_headroom {LIMIT_HEADROOM!r}")
    print(f"block_ohm {BLOCK_OHM!r}")
    print(f"block_a {BLOCK_A!r}")
    print(f"gmin {GMIN!r}")
    print(f"ground_current_ratio {GROUND_RATIO!r}")
    print(f"ratings.current {RATING_CURRENT!r}")
    print(f"ratings.power {POWER!r}")
    print(f"rth_sot223 {RTH!r}")
    print(f"line_span {LINE_SPAN!r}")
    print(f"dropout_vin_for_note4 {NOTE4_VIN!r}")
    print(f"absolute_junction_150C {POWER_ABSOLUTE!r}")

    v10 = vo(5.0, 0.01)
    print(f"vout_10mA_5V model {v10!r} table_typ 3.3 table_min 3.267 table_max 3.333")

    def unloaded(vi):
        _, volts = open_circuit(vi)
        return volts

    line = unloaded(15.0) - unloaded(4.75)
    print(f"line_4.75_to_15_at_0mA model {line!r} table_typ 0.001")

    load = unloaded(4.75) - vo(4.75, 0.8)
    print(f"load_0_to_800mA_at_4.75V model {load!r} table_typ 0.001")
    load_mid = vo(4.75, 0.1) - vo(4.75, 0.8)
    print(f"load_100_to_800mA model {load_mid!r}")

    for amps, table in DROP:
        got = dropout(amps)
        print(f"dropout_{amps:.3f}A model {got!r} table {table:.2f} delta {got - table!r}")

    limited = solve_current(5.0, 0.0)
    print(f"current_limit_at_5V_differential model {limited!r} table_min 0.8 table_typ 1.2 table_max 1.5")

    print(f"ground_current_5V model {ig(5.0, 0.0)!r} table_typ 0.005 table_max 0.01")
    print(f"ground_current_800mA model {ig(4.75, 0.8)!r}")

    i_open, v_open = open_circuit(5.0)
    print(f"open_5V i {i_open!r} vo {v_open!r}")

    model_step = 0.5
    figure_step = 0.4
    figure_dip = 0.140
    dip = RO * model_step
    print(f"static_dip_0.5A_on_10uF {dip!r}")
    print(f"figure_7_8_dip_reading {figure_dip!r}")
    print(f"figure_7_8_step_reading {figure_step!r}")
    print(f"dip_ratio {figure_dip / dip!r}")
    print(f"dip_ratio_per_ampere {(figure_dip / figure_step) / (dip / model_step)!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
