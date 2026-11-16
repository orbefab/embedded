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
//!
//! The circuits are the existing `op-amp-*` netlists. `netlist` writes these nine values
//! over each op-amp and leaves every other part as the file has it.

use std::f64::consts::PI;

use orbe_engine::diagnostic::Severity;
use orbe_engine::netlist::Netlist;
use orbe_engine::run_folder::Results;

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
/// Volt below `vcc` with no load. SLOS068AB p23 figure 5-46, section 5.12, which applies
/// to the LM358. The stroke center is 1.212 V below Vcc at 0.1 mA and 1.286 V below Vcc
/// at 1 mA. The line through them falls 82.2 ohm and starts 1.204 V below Vcc, rounded to
/// 1.20 V. The x-axis is printed "Output Sink Current". The title is Output Sourcing
/// Characteristics and the axis is the drop from Vcc, so the current is source current.
/// The figure does not print a supply. Section 5.7's 5 V positive-rail row is a maximum
/// of 1.5 V from V+ and has no typical. That maximum is the VOH band.
const SWING_HIGH: f64 = 1.20;
/// Volt above `gnd` with no load. The negative-rail typical is 5 mV from V− at VS = 5 V
/// and RL at most 10 kohm. One load, so the intercept is that typical. Figure 5-47 is the
/// sinking characteristic, including a 5 V curve. Its slope is not `output_resistance`.
const SWING_LOW: f64 = 0.005;
/// Ohm. The slope of the `swing_high` line, 74 mV over 0.9 mA, 82.2 ohm, rounded to 80.
const RO: f64 = 80.0;
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
    assert!(near(SWING_HIGH, 1.20, 0.0));
    assert!(near(SWING_LOW, 0.005, 0.0));
    assert!(near(RO, 80.0, 0.0));
    assert!(near(ISC, 0.04, 0.0));
    assert!(near(ICC, 350e-6, 0.0));
    assert!(near(IB_MAX, 250e-9, 0.0));
    assert!(near(VOH_MIN, 3.5, 0.0));
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
    assert!(near(slew, 0.2976190476190476, 1e-15));
    assert!((slew - 0.3).abs() <= SLEW_WIDTH * 0.3);

    for ohm in [10e3, 2e3] {
        let (voh, vol) = (saturated(VCC - SWING_HIGH, ohm), saturated(SWING_LOW, ohm));
        assert!(voh >= VOH_MIN, "{ohm}: {voh}");
        assert!(vol <= VOL_MAX, "{ohm}: {vol}");
    }
    let sourcing = limited(VCC - SWING_HIGH, 0.0);
    let sinking = -limited(SWING_LOW, VCC);
    assert!(sourcing > 0.0 && sourcing <= ISC_MAX, "{sourcing}");
    assert!(sinking > 0.0 && sinking <= ISC_MAX, "{sinking}");

    assert!(near(hysteresis_ramp(SWING_LOW), 2.7495, 1e-12));
    assert!(near(hysteresis_ramp(VCC - SWING_HIGH), 2.37, 1e-12));
    assert!(near(transition_low(), 0.38, 1e-12));
    assert!(near(transition_high(), 3.42, 1e-12));

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

/// The engine's node gmin, siemens, and the law's supply split, ampere.
const GMIN: f64 = 1e-12;
const SPLIT_A: f64 = 1e-6;

const FIELDS: &[(&str, f64)] = &[
    ("gain_bandwidth", GBW),
    ("slew_rate", SLEW),
    ("open_loop_gain", A0),
    ("input_bias_current", IB),
    ("swing_high", SWING_HIGH),
    ("swing_low", SWING_LOW),
    ("output_resistance", RO),
    ("short_circuit_current", ISC),
    ("supply_current", ICC),
];

fn netlist(name: &str) -> Netlist {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/circuits")
        .join(name)
        .join("netlist.json");
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for part in doc["parts"].as_array_mut().unwrap() {
        if part["kind"] != "op_amp" {
            continue;
        }
        for &(field, value) in FIELDS {
            part[field]["value"] = serde_json::json!(value);
        }
    }
    orbe_engine::netlist::parse(&serde_json::to_vec(&doc).unwrap())
        .unwrap_or_else(|errors| panic!("{name}: {errors:?}"))
}

fn errors(results: &Results) -> Vec<String> {
    results
        .diagnostics()
        .iter()
        .filter(|d| d.severity() == Severity::Error)
        .map(|d| d.code().to_string())
        .collect()
}

fn operating_point(name: &str) -> impl Fn(&str) -> f64 {
    let results = orbe_engine::simulate(&netlist(name));
    assert!(
        errors(&results).is_empty(),
        "{name}: {:?}",
        errors(&results)
    );
    let op = results.observations().op().unwrap().values.clone();
    move |reference: &str| op[&reference.parse().unwrap()].get()
}

fn column(results: &Results, reference: &str) -> Vec<f64> {
    let trace = results.trace().unwrap();
    let reference: orbe_engine::name::Ref = reference.parse().unwrap();
    let k = (trace.columns().iter())
        .position(|r| *r == reference)
        .unwrap_or_else(|| panic!("no column {reference}"));
    trace.column(k).to_vec()
}

fn times(results: &Results) -> Vec<f64> {
    results
        .trace()
        .unwrap()
        .time()
        .iter()
        .map(|t| t.as_ps() as f64)
        .collect()
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + libm::exp(-x))
}

fn from_vcc(i: f64) -> f64 {
    i * sigmoid(i / SPLIT_A)
}

fn slope(t: &[f64], y: &[f64], from: f64, to: f64, after: f64) -> f64 {
    let cross = |level: f64| {
        let k = (1..t.len())
            .find(|&k| {
                t[k] > after && (y[k - 1] - level) * (y[k] - level) <= 0.0 && y[k] != y[k - 1]
            })
            .expect("crosses");
        let f = (level - y[k - 1]) / (y[k] - y[k - 1]);
        t[k - 1] + f * (t[k] - t[k - 1])
    };
    let (a, b) = (from + 0.2 * (to - from), from + 0.8 * (to - from));
    (b - a) / ((cross(b) - cross(a)) * 1e-12) / 1e6
}

fn amplitude(t: &[f64], y: &[f64], f: f64, start: f64) -> f64 {
    let (mut re, mut im, mut n) = (0.0, 0.0, 0.0);
    for (&tk, &yk) in t.iter().zip(y).take(t.len() - 1) {
        if tk < start {
            continue;
        }
        let phase = 2.0 * PI * f * tk * 1e-12;
        re += yk * libm::cos(phase);
        im += yk * libm::sin(phase);
        n += 1.0;
    }
    2.0 * (re * re + im * im).sqrt() / n
}

struct Steps {
    min_step_ps: f64,
    transitions: Vec<(f64, usize)>,
}

fn steps(results: &Results) -> Steps {
    let (t, y) = (times(results), column(results, "#net.out"));
    let (low, high_level) = (transition_low(), transition_high());
    let min_step_ps = t
        .windows(2)
        .map(|w| w[1] - w[0])
        .fold(f64::INFINITY, f64::min);
    let (mut transitions, mut start, mut high) = (Vec::new(), None, y[0] > VCC / 2.0);
    for k in 0..y.len() {
        let now = match y[k] {
            v if v >= high_level => Some(true),
            v if v <= low => Some(false),
            _ => None,
        };
        match now {
            None if start.is_none() => start = Some(k),
            None => {}
            Some(now) => {
                if let Some(s) = start.take()
                    && now != high
                {
                    transitions.push((t[k], k - s));
                }
                high = now;
            }
        }
    }
    Steps {
        min_step_ps,
        transitions,
    }
}

fn crossing(t: &[f64], y: &[f64], level: f64, after: f64) -> f64 {
    let k = (1..t.len())
        .find(|&k| t[k] > after && (y[k - 1] - level) * (y[k] - level) <= 0.0 && y[k] != y[k - 1])
        .expect("crosses");
    t[k - 1] + (level - y[k - 1]) / (y[k] - y[k - 1]) * (t[k] - t[k - 1])
}

fn crossings(t: &[f64], y: &[f64], level: f64) -> Vec<f64> {
    (1..t.len())
        .filter(|&k| (y[k - 1] - level) * (y[k] - level) < 0.0)
        .map(|k| t[k - 1] + (level - y[k - 1]) / (y[k] - y[k - 1]) * (t[k] - t[k - 1]))
        .collect()
}

/// Circuit 1. The sheet band is the 250 nA IB maximum. 2.5 V is inside the common-mode
/// range. 4.95 V is above (V+) − 1.5 V, so that point has no sheet band.
#[test]
fn the_bias_current_leaves_each_input() {
    let read = operating_point("op-amp-bias");
    let bias = -read("#rin.i");
    let want = IB - GMIN * read("#net.inp");
    eprintln!(
        "op-amp-bias: I(1 Mohm) {bias:e} A, band <= {IB_MAX:e}, margin {:e}, law {want:e}",
        IB_MAX - bias.abs()
    );
    assert!(bias.abs() <= IB_MAX, "{bias}");
    assert!((bias - want).abs() <= BIAS_LAW_A, "{bias} want {want}");
    assert!((read("#net.inp") - 2.5 - want * 1e6).abs() <= INPUT_LAW_V);
    assert!(bias >= BIAS_ON_SHARE * IB, "{bias}");
    let out = read("#net.out");
    assert!(
        (out - read("#net.inp")).abs() <= FOLLOWER_LAW_V,
        "follower {out}"
    );
    let sourced = (out - 2.5) / 10e3 - IB - GMIN * out;
    let supply = -read("#vs.i");
    let want = ICC + 2.0 * IB + GMIN * VCC + from_vcc(sourced);
    let newton = 1e-6 * supply + 1e-12;
    eprintln!(
        "op-amp-bias: supply {supply} A, law {want} A, at 4.95 V {:e} A",
        -read("#rinh.i")
    );
    assert!(
        (supply - want).abs() <= newton,
        "supply {supply} want {want}"
    );
}

/// Circuit 2. Both loads meet the 5 V rows, so VOH is at least 3.5 V and VOL is at most 20 mV.
#[test]
fn the_output_swings_to_its_rails() {
    let read = operating_point("op-amp-swing");
    for (ohm, high, low) in [
        (10e3, "#net.h10k", "#net.l10k"),
        (2e3, "#net.h2k", "#net.l2k"),
    ] {
        let (voh, vol) = (read(high), read(low));
        let (want_high, want_low) = (saturated(VCC - SWING_HIGH, ohm), saturated(SWING_LOW, ohm));
        eprintln!(
            "op-amp-swing {ohm} ohm: VOH {voh} V (band >= {VOH_MIN}, margin {:e}), VOL {vol} V (band <= {VOL_MAX}, margin {:e}), law {want_high} {want_low}",
            voh - VOH_MIN,
            VOL_MAX - vol
        );
        assert!(voh >= VOH_MIN && vol <= VOL_MAX, "{ohm}: {voh} {vol}");
        assert!(
            (voh - want_high).abs() <= SWING_LAW_V,
            "{ohm}: VOH {voh} want {want_high}"
        );
        assert!(
            (vol - want_low).abs() <= SWING_LAW_V,
            "{ohm}: VOL {vol} want {want_low}"
        );
    }
}

/// Circuit 3. The sheet gives 0.3 V/µs and no limit, so the band is 20% either side. The
/// follower is asked for 4.0 V and the output stops 1.20 V below 5 V. The slope is 20% to 80%
/// of the output it reaches.
#[test]
fn the_output_slews_at_the_slew_rate() {
    let results = orbe_engine::simulate(&netlist("op-amp-slew"));
    assert!(errors(&results).is_empty(), "{:?}", errors(&results));
    let (t, y) = (times(&results), column(&results, "#net.out"));
    let lo = y[0];
    let hi = y[t.iter().position(|&x| x >= 29e6).unwrap()];
    let rise = slope(&t, &y, lo, hi, 10e6);
    let fall = -slope(&t, &y, hi, lo, 30e6);
    let want = SLEW / (1.0 + RO / 10e3) / 1e6;
    let (band_lo, band_hi) = (0.3 * (1.0 - SLEW_WIDTH), 0.3 * (1.0 + SLEW_WIDTH));
    eprintln!(
        "op-amp-slew: {lo} V to {hi} V, rise {rise} V/us, fall {fall} V/us, band {band_lo} to {band_hi}, law {want}"
    );
    for edge in [rise, fall] {
        assert!((band_lo..=band_hi).contains(&edge), "{edge}");
        assert!(
            (edge - want).abs() <= SLEW_LAW_V_PER_US,
            "{edge} want {want}"
        );
    }
}

/// Circuit 4. The bands are 2% and 10% around the infinite-DC-gain gain. The law is the
/// finite `A0` value.
#[test]
fn the_closed_loop_gain_falls_with_one_pole() {
    for (name, f, width) in [
        ("op-amp-gain-10k", 10e3, GAIN_WIDTH_10K),
        ("op-amp-gain-200k", 200e3, GAIN_WIDTH_200K),
    ] {
        let results = orbe_engine::simulate(&netlist(name));
        assert!(
            errors(&results).is_empty(),
            "{name}: {:?}",
            errors(&results)
        );
        let t = times(&results);
        let stop = *t.last().unwrap();
        let start = stop - 6.0 / f * 1e12;
        let gain = amplitude(&t, &column(&results, "#net.out"), f, start)
            / amplitude(&t, &column(&results, "#net.inp"), f, start);
        let band = infinite_gain(f);
        let want = finite_gain(f);
        eprintln!(
            "{name}: |G| {gain}, band {band} +- {}, margin {:e}, law {want}",
            width * band,
            width * band - (gain - band).abs()
        );
        assert!((gain - band).abs() <= width * band, "{name}: {gain}");
        assert!(
            (gain - want).abs() <= GAIN_LAW_REL * want,
            "{name}: {gain} want {want}"
        );
    }
}

/// Circuit 5. The sheet maximum is 60 mA at VS = 10 V. The circuit shorts at 5 V.
#[test]
fn the_output_current_is_limited_both_ways() {
    let read = operating_point("op-amp-short");
    let sourcing = read("#v0.i");
    let sinking = -read("#v5.i");
    let (want_source, want_sink) = (limited(VCC - SWING_HIGH, 0.0), -limited(SWING_LOW, VCC));
    eprintln!(
        "op-amp-short: sourcing {sourcing} A (band <= {ISC_MAX}, margin {:e}, law {want_source}), sinking {sinking} A (margin {:e}, law {want_sink})",
        ISC_MAX - sourcing,
        ISC_MAX - sinking
    );
    for (amps, want) in [(sourcing, want_source), (sinking, want_sink)] {
        assert!(amps > 0.0 && amps <= ISC_MAX, "{amps}");
        assert!((amps - want).abs() <= SHORT_LAW_A, "{amps} want {want}");
    }
    let supply = -read("#vs.i");
    let want = 2.0 * ICC + 4.0 * IB + from_vcc(sourcing) + from_vcc(-sinking);
    assert!(
        (supply - want).abs() <= SHORT_SUPPLY_A,
        "supply {supply} want {want}"
    );
}

/// Circuit 6. A transition is 10% to 90% of the no-load high stop. The delay windows are the
/// clamp overrun at this slew rate and this gain-bandwidth product.
#[test]
fn a_comparator_switches_in_few_steps_at_its_threshold() {
    let edges: Vec<f64> = (0..10)
        .flat_map(|n| [10e6 + n as f64 * 100e6, 60e6 + n as f64 * 100e6])
        .collect();
    let past = clamp_past_rail();
    let (near_v, far_v) = (2.5, 2.5 + past);
    let ramp = (ramp_delay_ps(near_v, 500.0), ramp_delay_ps(far_v, 500.0));
    let slew = (slew_delay_ps(near_v), slew_delay_ps(far_v));
    let fed = (slew.0, ramp_delay_ps(far_v, 400.0));
    let (up_want, down_want) = (
        hysteresis_ramp(SWING_LOW),
        hysteresis_ramp(VCC - SWING_HIGH),
    );
    for (name, input, (early, late)) in [
        ("op-amp-ramp", "#net.ramp", ramp),
        ("op-amp-hysteresis", "#net.inp", fed),
        ("op-amp-square", "", slew),
    ] {
        let results = orbe_engine::simulate_steps(&netlist(name));
        let s = steps(&results);
        let worst = s.transitions.iter().map(|&(_, n)| n).max().unwrap();
        eprintln!(
            "{name}: errors {:?}, min step {} ps, {} transitions, worst {} steps, window {early} to {late} ps",
            errors(&results),
            s.min_step_ps,
            s.transitions.len(),
            worst
        );
        assert!(
            errors(&results).is_empty(),
            "{name}: {:?}",
            errors(&results)
        );
        assert!(s.min_step_ps >= MIN_STEP_PS, "{name}: {}", s.min_step_ps);
        assert!(worst <= MAX_TRANSITION_STEPS, "{name}: {worst}");
        let t = times(&results);
        let thresholds = match input {
            "" => edges.clone(),
            input => crossings(&t, &column(&results, input), VCC / 2.0),
        };
        assert_eq!(s.transitions.len(), thresholds.len(), "{name}");
        let y = column(&results, "#net.out");
        for &threshold in &thresholds {
            let delay = crossing(&t, &y, VCC / 2.0, threshold) - threshold;
            eprintln!(
                "  {name}: crosses {delay} ps after {threshold} ps, margin {:e} to {:e}",
                delay - early,
                late - delay
            );
            assert!(
                (early..=late).contains(&delay),
                "{name}: {delay} after {threshold}"
            );
        }
        if name == "op-amp-hysteresis" {
            let ramp = column(&results, "#net.ramp");
            let at = |when: f64| {
                let k = t.partition_point(|&x| x < when);
                ramp[k - 1] + (when - t[k - 1]) / (t[k] - t[k - 1]) * (ramp[k] - ramp[k - 1])
            };
            let (up, down) = (at(thresholds[0]), at(thresholds[1]));
            eprintln!(
                "  {name}: ramp at the thresholds {up} V and {down} V, law {up_want} and {down_want}"
            );
            assert!(
                (up - up_want).abs() <= HYSTERESIS_LAW_V
                    && (down - down_want).abs() <= HYSTERESIS_LAW_V,
                "{up} {down}"
            );
        }
    }
}
