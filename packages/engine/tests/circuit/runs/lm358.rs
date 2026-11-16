//! The LM358's values and the bands the `op-amp-*` circuits must meet on them.
//! TI SLOS068AB p10 section 5.7, the LM358 column, not the LM358A. The table header is
//! VS = 5 V and TA = 25 °C unless a row says otherwise. A typical is 25 °C (note 2).
//!
//! The common-mode range at 25 °C is (V−) to (V+) − 1.5 V. The law's input headroom stays
//! 0.8 V. None of these bands turns on that difference.
//!
//! A width around a typical is the acceptance where section 5.7 prints no minimum and no
//! maximum. It is fixed with the row, at the same fraction `op_amp.rs` uses for a typical
//! that has no limit.

use std::f64::consts::PI;

/// Volt. The table header.
const VCC: f64 = 5.0;
/// Hz. GBW typical, header conditions, no extra row condition.
const GBW: f64 = 700_000.0;
/// V/s. SR typical, G = +1.
const SLEW: f64 = 300_000.0;
/// 1. AOL 100 V/mV typical at VS = 15 V, VO = 1 V to 11 V, RL at least 2 kohm.
/// Section 5.7 has no 5 V open-loop gain.
const A0: f64 = 100_000.0;
/// Ampere, leaving each input. IB typical is −20 nA into the pin at VO = 1.4 V.
const IB: f64 = 20e-9;
/// Volt below `vcc` with no load. The 5 V positive-rail row is a maximum of 1.5 V from
/// V+ at RL at least 2 kohm and 25 °C, and that row has no typical. The 30 V typical of
/// 2 V is a different supply.
const SWING_HIGH: f64 = 1.5;
/// Volt above `gnd` with no load. The negative-rail typical is 5 mV from V− at VS = 5 V
/// and RL at most 10 kohm. One load, so the intercept is that typical.
const SWING_LOW: f64 = 0.005;
/// Ohm. Section 5.7 has no second 5 V load point, so the swing line has no slope.
const RO: f64 = 0.0;
/// Ampere. ISC ±40 mA typical at VS = 10 V, VO = VS / 2. Section 5.7 has no 5 V row.
const ISC: f64 = 0.04;
/// Ampere. IQ 350 µA typical per amplifier at VO = 2.5 V and IO = 0 A. The row is already
/// one amplifier.
const ICC: f64 = 350e-6;

/// Ampere. IB maximum at 25 °C, −250 nA into the pin.
const IB_MAX: f64 = 250e-9;
/// Volt. VOH is at least VS − 1.5 V on the 5 V positive-rail row. Both swing loads are
/// at least 2 kohm. The circuits return those loads to 2.5 V.
const VOH_MIN: f64 = 3.5;
/// Volt. VOL is at most 20 mV on the 5 V negative-rail row, over 0 °C to 70 °C. Both
/// swing loads are at most 10 kohm.
const VOL_MAX: f64 = 0.020;
/// Ampere. ISC maximum is ±60 mA at VS = 10 V, VO = VS / 2. The short circuit runs at 5 V.
const ISC_MAX: f64 = 0.060;
/// Share of the 0.3 V/µs typical. Section 5.7 gives no slew minimum or maximum.
const SLEW_WIDTH: f64 = 0.20;
/// Share of the infinite-DC-gain closed-loop value. Section 5.7 gives no gain-vs-frequency
/// limit. 2% at 10 kHz and 10% at 200 kHz.
const GAIN_WIDTH_10K: f64 = 0.02;
const GAIN_WIDTH_200K: f64 = 0.10;

/// Law acceptances, the same widths as `op_amp.rs`. They are not rows of section 5.7.
const BIAS_LAW_A: f64 = 1e-16;
const FOLLOWER_LAW_V: f64 = 1e-4;
const INPUT_LAW_V: f64 = 1e-9;
const BIAS_ON_SHARE: f64 = 0.99;
const SWING_LAW_V: f64 = 1e-5;
const SLEW_LAW_V_PER_US: f64 = 2e-3;
const GAIN_LAW_REL: f64 = 5e-3;
const SHORT_LAW_A: f64 = 1e-7;
const SHORT_SUPPLY_A: f64 = 1e-9;
const MIN_STEP_PS: f64 = 100.0;
const MAX_TRANSITION_STEPS: usize = 2000;
const HYSTERESIS_LAW_V: f64 = 1e-3;

/// The law's clamp, from `docs/engine/netlist.md`. `u` sits this far past a rail while the
/// slew current runs into the clamp.
fn clamp_past_rail() -> f64 {
    0.1 + 0.01 * libm::log(libm::exp(SLEW / 1e5) - 1.0)
}

/// Closed-loop gain of 10 with one pole and an infinite DC gain.
fn infinite_gain(f: f64) -> f64 {
    let x = 10.0 * f / GBW;
    10.0 / (1.0 + x * x).sqrt()
}

/// Closed-loop gain of 10 with DC gain `A0` and unity-gain frequency `GBW`.
fn finite_gain(f: f64) -> f64 {
    let (dc, pole) = (1.0 + A0 / 10.0, f * A0 / GBW);
    A0 / (dc * dc + pole * pole).sqrt()
}

/// How long the integrator takes to move `volts` at `SLEW`, in ps.
fn slew_delay_ps(volts: f64) -> f64 {
    volts / SLEW * 1e12
}

/// `volts = pi * GBW * rate * t^2`, in ps. `rate` is V/s.
fn ramp_delay_ps(volts: f64, rate: f64) -> f64 {
    (volts / (PI * GBW * rate)).sqrt() * 1e12
}

/// The ramp voltage at which `in_p` is 2.5 V, `2.75 - 0.1 * V(out)`.
fn hysteresis_ramp(vout: f64) -> f64 {
    2.75 - 0.1 * vout
}

/// 90% of the 5 V supply is 4.5 V. The 5 V row does not put the output there, so a
/// transition runs between these shares of the no-load high stop.
const EDGE_LOW_SHARE: f64 = 0.1;
const EDGE_HIGH_SHARE: f64 = 0.9;

fn transition_low() -> f64 {
    EDGE_LOW_SHARE * (VCC - SWING_HIGH)
}

fn transition_high() -> f64 {
    EDGE_HIGH_SHARE * (VCC - SWING_HIGH)
}

fn saturated(t: f64, ohm: f64) -> f64 {
    (t + RO * 2.5 / ohm) / (1.0 + RO / ohm)
}

fn softplus(x: f64) -> f64 {
    if x > 0.0 {
        x + libm::log1p(libm::exp(-x))
    } else {
        libm::log1p(libm::exp(x))
    }
}

/// The output current that holds `vo` at `held`, integrator saturated at swing `t`.
fn limited(t: f64, held: f64) -> f64 {
    let d = 1e-3 * ISC;
    let rl = 500.0 / ISC;
    let law =
        |i: f64| t - RO * i - rl * d * softplus((i - ISC) / d) + rl * d * softplus((-i - ISC) / d);
    let (mut lo, mut hi) = (-1.0, 1.0);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if law(mid) > held {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

fn near(got: f64, want: f64, tol: f64) -> bool {
    (got - want).abs() <= tol
}

/// The sheet limits and the hand-solved law, with no circuit run.
#[test]
fn the_pass_bands_are_the_sheet_limits() {
    assert!(near(GBW, 700_000.0, 0.0));
    assert!(near(SLEW, 300_000.0, 0.0));
    assert!(near(A0, 100_000.0, 0.0));
    assert!(near(IB, 20e-9, 0.0));
    assert!(near(SWING_HIGH, 1.5, 0.0));
    assert!(near(SWING_LOW, 0.005, 0.0));
    assert!(near(RO, 0.0, 0.0));
    assert!(near(ISC, 0.04, 0.0));
    assert!(near(ICC, 350e-6, 0.0));
    assert!(near(IB_MAX, 250e-9, 0.0));
    assert!(near(VOH_MIN, VCC - SWING_HIGH, 0.0));
    assert!(near(VOL_MAX, 0.020, 0.0));
    assert!(near(ISC_MAX, 0.060, 0.0));

    let past = clamp_past_rail();
    assert!(near(past, 0.129489308190573, 1e-12), "{past}");
    let (early, late) = (2.5, 2.5 + past);
    assert!(near(slew_delay_ps(early), 8.333333333333334e6, 1.0));
    assert!(near(slew_delay_ps(late), 8.764964360635243e6, 1.0));
    assert!(near(ramp_delay_ps(early, 500.0), 4.768272270088961e7, 1.0));
    assert!(near(ramp_delay_ps(late, 500.0), 4.890201405176436e7, 1.0));
    assert!(near(ramp_delay_ps(late, 400.0), 5.467411382819752e7, 1.0));

    let gain_10k = infinite_gain(10e3);
    let gain_200k = infinite_gain(200e3);
    assert!(near(gain_10k, 9.899494936611665, 1e-12), "{gain_10k}");
    assert!(near(gain_200k, 3.303504247281061, 1e-12), "{gain_200k}");
    assert!(near(finite_gain(10e3), 9.898524880203445, 1e-12));
    assert!(near(finite_gain(200e3), 3.303468194463312, 1e-12));
    assert!((finite_gain(10e3) - gain_10k).abs() <= GAIN_WIDTH_10K * gain_10k);
    assert!((finite_gain(200e3) - gain_200k).abs() <= GAIN_WIDTH_200K * gain_200k);

    let slew = SLEW / (1.0 + RO / 10e3) / 1e6;
    assert!(near(slew, 0.3, 1e-15));
    assert!((slew - 0.3).abs() <= SLEW_WIDTH * 0.3);

    for ohm in [10e3, 2e3] {
        assert!(near(saturated(VCC - SWING_HIGH, ohm), VOH_MIN, 1e-15));
        assert!(saturated(SWING_LOW, ohm) <= VOL_MAX);
    }
    let sourcing = limited(VCC - SWING_HIGH, 0.0);
    let sinking = -limited(SWING_LOW, VCC);
    assert!(sourcing > 0.0 && sourcing <= ISC_MAX, "{sourcing}");
    assert!(sinking > 0.0 && sinking <= ISC_MAX, "{sinking}");

    assert!(near(hysteresis_ramp(SWING_LOW), 2.7495, 1e-12));
    assert!(near(hysteresis_ramp(VCC - SWING_HIGH), 2.4, 1e-12));
    assert!(near(transition_low(), 0.35, 1e-12));
    assert!(near(transition_high(), 3.15, 1e-12));

    assert!(near(BIAS_LAW_A, 1e-16, 0.0));
    assert!(near(FOLLOWER_LAW_V, 1e-4, 0.0));
    assert!(near(INPUT_LAW_V, 1e-9, 0.0));
    assert!(near(BIAS_ON_SHARE, 0.99, 0.0));
    assert!(near(SWING_LAW_V, 1e-5, 0.0));
    assert!(near(SLEW_LAW_V_PER_US, 2e-3, 0.0));
    assert!(near(GAIN_LAW_REL, 5e-3, 0.0));
    assert!(near(SHORT_LAW_A, 1e-7, 0.0));
    assert!(near(SHORT_SUPPLY_A, 1e-9, 0.0));
    assert!(near(MIN_STEP_PS, 100.0, 0.0));
    assert_eq!(MAX_TRANSITION_STEPS, 2000);
    assert!(near(HYSTERESIS_LAW_V, 1e-3, 0.0));
    assert!(near(EDGE_LOW_SHARE, 0.1, 0.0));
    assert!(near(EDGE_HIGH_SHARE, 0.9, 0.0));
}
