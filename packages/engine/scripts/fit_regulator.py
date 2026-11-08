# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fit a regulator row.

    uv run packages/engine/scripts/fit_regulator.py
    uv run packages/engine/scripts/fit_regulator.py lp2985

With no argument the script fits the LM1117-3.3 row the regulator examples
use. The table is TI LM1117, SNOS412Q, revised January 2023. VOUT, line
regulation and load regulation are p5. Dropout, current limit and quiescent
current are p6. Note 3 on p5 measures line and load regulation at constant
junction room temperature, and the fit uses that table. Note 4 on p6 defines
dropout as the input-to-output differential where VOUT has fallen 100 mV
from its value at VIN = VOUT + 1.5 V. The AMS1117 defines dropout as a 1%
drop, 33 mV at 3.3 V. This script does not fit that sheet.

`lp2985` fits the legacy LP2985-33 column of TI SLVS522S, revised May 2025.
The floor constants are the module constants below. They are not refit.
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


def lm1117():
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


def sigmoid(x):
    if x >= 0.0:
        return 1.0 / (1.0 + math.exp(-x))
    e = math.exp(x)
    return e / (1.0 + e)


def law_row(vout, k, ro, rdo, vdo, ilim, i, vi):
    """The branch row and its partials. The floor constants stay as named above."""
    d = LIMIT_FRACTION * ilim
    rlim = LIMIT_HEADROOM * vout / ilim
    a = vout + k * (vi - vout) - ro * i
    raw = vi - vdo - rdo * i
    floor = -FLOOR_OHM * i
    blend = sigmoid((raw - floor) / KNEE_V)
    b = floor + KNEE_V * sp((raw - floor) / KNEE_V)
    share = sigmoid((a - b) / KNEE_V)
    vo_min = a - KNEE_V * sp((a - b) / KNEE_V)
    db_di = -FLOOR_OHM + blend * (FLOOR_OHM - rdo)
    db_dvi = blend
    over = (i - ilim) / d
    back = -i / BLOCK_A
    volts = vo_min - rlim * d * sp(over) + BLOCK_OHM * BLOCK_A * sp(back)
    dvo_di = -ro * (1.0 - share) + db_di * share - rlim * sigmoid(over) - BLOCK_OHM * sigmoid(back)
    dvo_dvi = k * (1.0 - share) + db_dvi * share
    return volts, dvo_di, dvo_dvi


def lp2985():
    """Legacy chip, standard grade, TJ = 25 C. PDF page numbers of SLVS522S."""
    vout = 3.3
    # p5, legacy, 0.007 %/V typical from VO(NOM)+1 V to 16 V.
    line_percent_per_volt = 0.007
    line_lo = vout + 1.0
    line_hi = 16.0
    line_span = line_hi - line_lo
    k = (line_percent_per_volt / 100.0) * vout
    # p5, standard grade. The 1 mA to 150 mA window is the fitted slope.
    # The 1 mA to 50 mA window and the page 25 load-step figures are printed
    # beside it and are not the field.
    tol_1ma = 0.015
    tol_50 = 0.025
    tol_150 = 0.030
    i_lo = 0.001
    i_50 = 0.050
    i_hi = 0.150
    load_span = i_hi - i_lo
    load_limit_v = (tol_150 - tol_1ma) * vout
    ro = load_limit_v / load_span
    ro_50 = (tol_50 - tol_1ma) * vout / (i_50 - i_lo)
    figure_7_3_ohm = 0.26
    figure_7_5_ohm = 0.29
    # p6, legacy typical dropout, the line through 7 mV at 1 mA and 280 mV at 150 mA.
    drop = [(0.0, 0.001), (0.001, 0.007), (0.010, 0.040), (0.050, 0.120), (0.150, 0.280)]
    fit_lo, fit_hi = drop[1], drop[4]
    rdo = (fit_hi[1] - fit_lo[1]) / (fit_hi[0] - fit_lo[0])
    vdo = fit_lo[1] - rdo * fit_lo[0]
    # p7, legacy IO(PK) typical. The short-circuit row is 400 mA and is not this field.
    ilim = 0.350
    # p6, legacy IGND typical, 65 µA at 0 mA and 850 µA at 150 mA.
    iq = 65e-6
    ig_150 = 850e-6
    ratio = (ig_150 - iq) / i_hi
    # p4, 5.3, IOUT max 150 mA. p4, 5.3, recommended TJ 125 C.
    # p5, 5.4, legacy DBV RθJA 205.4 C/W. Ambient is the table's 25 C.
    rating_current = 0.150
    rth = 205.4
    tj = 125.0
    ta = 25.0
    power = (tj - ta) / rth
    power_absolute = (150.0 - ta) / rth
    note1_vin = line_lo
    ground = [
        (0.0, 65e-6),
        (0.001, 75e-6),
        (0.010, 120e-6),
        (0.050, 350e-6),
        (0.150, 850e-6),
    ]

    def volts(vi, i):
        return law_row(vout, k, ro, rdo, vdo, ilim, i, vi)[0]

    def amps_ground(vi, i):
        return (iq + ratio * i) * smooth(vi / vdo)

    def solve_current(vi, target):
        i = ilim
        for _ in range(40):
            got = volts(vi, i) - target
            step = 1e-8
            slope = (volts(vi, i + step) - volts(vi, i - step)) / (2.0 * step)
            i -= got / slope
        return i

    def open_circuit(vi):
        i = GMIN * vout
        for _ in range(20):
            got = volts(vi, i)
            i = GMIN * got
        return i, volts(vi, i)

    def dropout(i):
        reference = volts(note1_vin, i)
        target = reference - 0.1
        lo, hi = target, note1_vin
        for _ in range(80):
            mid = 0.5 * (lo + hi)
            if volts(mid, i) < target:
                lo = mid
            else:
                hi = mid
        vin = 0.5 * (lo + hi)
        return vin - volts(vin, i)

    print("part lp2985-33-legacy-standard")
    print("doc TI SLVS522S revised May 2025")
    print(f"output_voltage {vout!r}")
    print(f"line_percent_per_volt {line_percent_per_volt!r}")
    print(f"line_span {line_span!r}")
    print(f"line_regulation {k!r}")
    print(f"load_limit_widen {load_limit_v!r}")
    print(f"load_span {load_span!r}")
    print(f"output_resistance {ro!r}")
    print(f"tol_50ma {tol_50!r}")
    print(f"output_resistance_1ma_to_50ma {ro_50!r}")
    print(f"figure_7_3_load_step_ohm {figure_7_3_ohm!r}")
    print(f"figure_7_5_load_step_ohm {figure_7_5_ohm!r}")
    print("output_resistance_basis 1ma_to_150ma_window")
    print("uvlo_rising_typ_V 2.2")
    print("uvlo_rising_max_V 2.4")
    print("recommended_vin_min_V 2.2")
    print("reverse_current_section 7.1.4")
    print("reverse_current_page 21")
    print(f"dropout_voltage {vdo!r}")
    print(f"dropout_resistance {rdo!r}")
    print(f"current_limit {ilim!r}")
    print(f"quiescent_current {iq!r}")
    print(f"ground_current_ratio {ratio!r}")
    print(f"ratings.current {rating_current!r}")
    print(f"ratings.power {power!r}")
    print(f"rth_dbv_legacy {rth!r}")
    print(f"tj_recommended_C {tj!r}")
    print(f"ambient_C {ta!r}")
    print(f"tj_absolute_C {150.0!r}")
    print(f"line_vin_lo {line_lo!r}")
    print(f"line_vin_hi {line_hi!r}")
    print(f"tol_1ma {tol_1ma!r}")
    print(f"tol_150ma {tol_150!r}")
    print(f"orderable_code {33!r}")
    print("cite_output_voltage page 30 table 8-1")
    print("cite_line page 5")
    print("cite_load page 5")
    print("cite_dropout page 6")
    print("cite_peak page 7")
    print("cite_ground page 6")
    print("cite_current_rating page 4 section 5.3")
    print("cite_rth page 5 section 5.4")
    print("cite_note1 page 7 note 1")
    print("cite_ripple page 7")
    print("cite_noise page 7")
    print("cite_shutdown page 6")
    print("cite_onoff page 7")
    print("cite_cout page 4 section 5.3")
    print("cite_absolute_vout page 4 section 5.1")
    print(f"ripple_Hz {1000!r}")
    print(f"absolute_junction_150C {power_absolute!r}")
    print(f"floor_ohm {FLOOR_OHM!r}")
    print(f"knee_v {KNEE_V!r}")
    print(f"limit_fraction {LIMIT_FRACTION!r}")
    print(f"limit_headroom {LIMIT_HEADROOM!r}")
    print(f"block_ohm {BLOCK_OHM!r}")
    print(f"block_a {BLOCK_A!r}")
    print(f"gmin {GMIN!r}")
    print(f"note1_vin {note1_vin!r}")
    print("load_regulation_typical absent")
    print("short_circuit_table_typ 0.4")
    print("ripple_rejection_1kHz_dB 45")
    print("output_noise_uVrms 30")
    print("shutdown_ground_uA 0.01")
    print("on_off_high_typ_V 1.4")
    print("legacy_cout_min_uF 2.2")
    print("schematic_c3_uF 1")
    print("absolute_vout_above_vin_V 0.3")

    v_1ma = volts(note1_vin, i_lo)
    print(
        f"vout_1mA_4.3V model {v_1ma!r} nominal {vout!r} "
        f"window_min {vout * (1.0 - tol_1ma)!r} window_max {vout * (1.0 + tol_1ma)!r}"
    )

    def unloaded(vi):
        _, got = open_circuit(vi)
        return got

    line = unloaded(line_hi) - unloaded(line_lo)
    line_table = (line_percent_per_volt / 100.0) * vout * line_span
    print(f"line_4.3_to_16_unloaded model {line!r} table_typ {line_table!r}")

    load = volts(note1_vin, i_lo) - volts(note1_vin, i_hi)
    print(f"load_1mA_to_150mA_at_4.3V model {load!r} limit {load_limit_v!r}")

    for amps, table in drop:
        got = dropout(amps)
        print(f"dropout_{amps:.3f}A model {got!r} table {table!r} delta {got - table!r}")

    limited = solve_current(5.0, 0.0)
    headroom = LIMIT_HEADROOM * vout / ilim * LIMIT_FRACTION * ilim
    print(f"short_at_5V model {limited!r} peak_typ {ilim!r} short_typ 0.4")
    print(f"limit_headroom_volts {headroom!r}")

    for amps, table in ground:
        got = amps_ground(5.0, amps)
        print(f"ground_{amps:.3f}A model {got!r} table {table!r} delta {got - table!r}")

    i_open, v_open = open_circuit(5.0)
    print(f"open_5V i {i_open!r} vo {v_open!r} ig {amps_ground(5.0, i_open)!r}")
    i_off, v_off = open_circuit(0.0)
    print(f"open_0V i {i_off!r} vo {v_off!r} ig {amps_ground(0.0, i_off)!r}")
    print(f"rlim {LIMIT_HEADROOM * vout / ilim!r}")
    print(f"limit_width {LIMIT_FRACTION * ilim!r}")
    return 0


def main():
    which = sys.argv[1] if len(sys.argv) > 1 else "lm1117"
    if which == "lm1117":
        return lm1117()
    if which == "lp2985":
        return lp2985()
    print(f"unknown part {which}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
