//! The `op-amp-*` examples run the LMV358's values, TI SLOS263Y p7 section 6.8 at VCC 5 V.
//! Each check holds the run to a band from that table and to the law in
//! `docs/engine/netlist.md`, solved by hand for the circuit.

use std::f64::consts::PI;

use orbe_engine::diagnostic::Severity;
use orbe_engine::netlist::Netlist;
use orbe_engine::run_folder::Results;

use super::common::examples;

const VCC: f64 = 5.0;
const GBW: f64 = 1e6;
const SLEW: f64 = 1e6;
const A0: f64 = 1e5;
const IB: f64 = 1.5e-8;
const SWING_HIGH: f64 = 0.0025;
const SWING_LOW: f64 = 0.051;
const RO: f64 = 30.0;
const ISC: f64 = 0.04;
const ICC: f64 = 1.05e-4;
/// The law's current-limit knee and slope, `d = 1e-3 * Isc` and `Rlim = 500 V / Isc`.
const LIMIT_D: f64 = 1e-3 * ISC;
const LIMIT_R: f64 = 500.0 / ISC;
/// The engine's node gmin, siemens.
const GMIN: f64 = 1e-12;
/// The law's supply split, ampere.
const SPLIT_A: f64 = 1e-6;

fn netlist(name: &str) -> Netlist {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/circuits")
        .join(name);
    examples::parse(&dir)
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
        results.diagnostics().is_empty(),
        "{name}: {:?}",
        results.diagnostics()
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
    let trace = results.trace().unwrap();
    trace.time().iter().map(|t| t.as_ps() as f64).collect()
}

fn sp(x: f64) -> f64 {
    if x > 0.0 {
        x + libm::log1p(libm::exp(-x))
    } else {
        libm::log1p(libm::exp(x))
    }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + libm::exp(-x))
}

/// The share of an output current `i` the law draws from `vcc`.
fn from_vcc(i: f64) -> f64 {
    i * sigmoid(i / SPLIT_A)
}

/// The output at the high or the low swing into `ohm` to 2.5 V, from `vo = t - Ro * i`.
fn saturated(t: f64, ohm: f64) -> f64 {
    (t + RO * 2.5 / ohm) / (1.0 + RO / ohm)
}

/// The output current that holds `vo` at a source of `held` volts, the integrator saturated at
/// swing `t`, by bisection on `t - Ro * i - Rlim * d * sp((i - Isc) / d) + Rlim * d * sp((-i -
/// Isc) / d) = held`, which falls with `i`.
fn limited(t: f64, held: f64) -> f64 {
    let law = |i: f64| {
        t - RO * i - LIMIT_R * LIMIT_D * sp((i - ISC) / LIMIT_D)
            + LIMIT_R * LIMIT_D * sp((-i - ISC) / LIMIT_D)
    };
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

/// Circuit 1. A follower whose `in_p` sits behind 1 Mohm from 2.5 V. The bias current leaves `in_p`,
/// so `in_p` sits 15 mV above 2.5 V. The datasheet bounds it by 250 nA. The node gmin of
/// 1e-12 S takes `V(in_p)` times that from it, so 1 Mohm carries the rest. The supply carries
/// one channel's supply current, both bias currents, the gmin on `vcc`, and the share of the
/// output current the law draws from `vcc`. `in_n` is the output net, so its bias current
/// feeds the 10 kohm load too, and the output sources the rest of the load's current. Newton
/// settles that sum to `RELTOL` of it plus `ABSTOL`.
#[test]
fn the_bias_current_leaves_each_input() {
    let read = operating_point("op-amp-bias");
    let bias = -read("#rin.i");
    eprintln!(
        "op-amp-bias: I(1 Mohm) {bias:e} A, V(in_p) - 2.5 V {:e} V",
        read("#net.inp") - 2.5
    );
    assert!(bias.abs() <= 250e-9, "{bias}");
    let want = IB - GMIN * read("#net.inp");
    assert!((bias - want).abs() <= 1e-16, "{bias} want {want}");
    assert!((read("#net.inp") - 2.5 - want * 1e6).abs() <= 1e-9);
    let out = read("#net.out");
    assert!((out - read("#net.inp")).abs() <= 1e-4, "follower {out}");
    let sourced = (out - 2.5) / 10e3 - IB - GMIN * out;
    let supply = -read("#vs.i");
    let want = ICC + 2.0 * IB + GMIN * VCC + from_vcc(sourced);
    eprintln!("op-amp-bias: supply {supply} A, law {want} A");
    let newton = 1e-6 * supply + 1e-12;
    assert!(
        (supply - want).abs() <= newton,
        "supply {supply} want {want}"
    );
}

/// Circuit 2. Overdriven open loop into 10 kohm and 2 kohm to 2.5 V. The datasheet's bands are VOH at
/// least 4.900 V and VOL at most 0.180 V into 10 kohm, and 4.700 V and 0.300 V into 2 kohm.
#[test]
fn the_output_swings_to_its_rails() {
    let read = operating_point("op-amp-swing");
    for (ohm, voh_min, vol_max, high, low) in [
        (10e3, 4.900, 0.180, "#net.h10k", "#net.l10k"),
        (2e3, 4.700, 0.300, "#net.h2k", "#net.l2k"),
    ] {
        let (voh, vol) = (read(high), read(low));
        let (want_high, want_low) = (saturated(VCC - SWING_HIGH, ohm), saturated(SWING_LOW, ohm));
        eprintln!(
            "op-amp-swing {ohm} ohm: VOH {voh} V (band >= {voh_min}, law {want_high}), VOL {vol} V (band <= {vol_max}, law {want_low})"
        );
        assert!(voh >= voh_min && vol <= vol_max, "{ohm}: {voh} {vol}");
        assert!(
            (voh - want_high).abs() <= 1e-5,
            "{ohm}: VOH {voh} want {want_high}"
        );
        assert!(
            (vol - want_low).abs() <= 1e-5,
            "{ohm}: VOL {vol} want {want_low}"
        );
    }
}

/// The 20 to 80 percent slope of the edge from `from` to `to` after `after`, V/us.
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

/// Circuit 3. A follower stepped from 0.5 V to 4.0 V at 10 us and back at 30 us. The datasheet's slew
/// rate is 1 V/us, and the band is 0.8 to 1.2 V/us. The integrator slews at `slew_rate`, and
/// the output follows it through `Ro` into 10 kohm, so it moves at `SR / (1 + Ro / 10 kohm)`.
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
    eprintln!("op-amp-slew: {lo} V to {hi} V, rise {rise} V/us, fall {fall} V/us, law {want}");
    for edge in [rise, fall] {
        assert!((0.8..=1.2).contains(&edge), "{edge}");
        assert!((edge - want).abs() <= 2e-3, "{edge} want {want}");
    }
}

/// The amplitude of `y` at `f` Hz over the rows from `start` ps, by a one-bin DFT. The last row
/// closes the run and is left out.
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

/// Circuit 4. Non-inverting gain 10, 10 kohm and 90 kohm, driven by a 64-level staircase of a 10 mV
/// sine on 0.25 V for 12 periods. The gain is the ratio of the output's and the input's
/// amplitudes at `f` over the last 6 periods, so the staircase's droop cancels. One pole with
/// DC gain `A0` and unity-gain frequency `GBW` gives `A0 / sqrt((1 + A0 / 10)^2 + (f A0 /
/// GBW)^2)`, 9.9494 at 10 kHz and 4.4721 at 200 kHz. The bands, fixed from the datasheet's
/// 1 MHz typical before any run, are 2% at 10 kHz and 10% at 200 kHz around the single pole's
/// value with an infinite DC gain, 9.950 and 4.472.
#[test]
fn the_closed_loop_gain_falls_with_one_pole() {
    for (name, f, band, tol) in [
        ("op-amp-gain-10k", 10e3, 9.950, 0.02),
        ("op-amp-gain-200k", 200e3, 4.472, 0.10),
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
        let (dc, pole) = (1.0 + A0 / 10.0, f * A0 / GBW);
        let want = A0 / (dc * dc + pole * pole).sqrt();
        eprintln!("{name}: |G| {gain}, band {band} +- {tol}, law {want}");
        assert!((gain - band).abs() <= tol * band, "{name}: {gain}");
        assert!(
            (gain - want).abs() <= 5e-3 * want,
            "{name}: {gain} want {want}"
        );
    }
}

/// Circuit 5. The output held at 0 V while driven high, and at 5 V while driven low. The datasheet's
/// short-circuit current is 40 mA typical each way, and the band is 36 to 44 mA. The law
/// holds it a little past `Isc`, where its slope meets the swing. The sourced current comes
/// from `vcc` and the sunk current goes to `gnd`.
#[test]
fn the_output_current_is_limited_both_ways() {
    let read = operating_point("op-amp-short");
    let sourcing = read("#v0.i");
    let sinking = -read("#v5.i");
    let (want_source, want_sink) = (limited(VCC - SWING_HIGH, 0.0), -limited(SWING_LOW, VCC));
    eprintln!(
        "op-amp-short: sourcing into 0 V {sourcing} A (law {want_source}), sinking from 5 V {sinking} A (law {want_sink})"
    );
    for (amps, want) in [(sourcing, want_source), (sinking, want_sink)] {
        assert!((0.036..=0.044).contains(&amps), "{amps}");
        assert!((amps - want).abs() <= 1e-7, "{amps} want {want}");
    }
    let supply = -read("#vs.i");
    let want = 2.0 * ICC + 4.0 * IB + from_vcc(sourcing) + from_vcc(-sinking);
    assert!((supply - want).abs() <= 1e-9, "supply {supply} want {want}");
}

struct Steps {
    landings: usize,
    min_step_ps: f64,
    /// Each output transition's end, from leaving 10% of the supply to reaching 90%, or back,
    /// with the accepted steps it took.
    transitions: Vec<(f64, usize)>,
}

fn steps(results: &Results) -> Steps {
    let (t, y) = (times(results), column(results, "#net.out"));
    let min_step_ps = t
        .windows(2)
        .map(|w| w[1] - w[0])
        .fold(f64::INFINITY, f64::min);
    let (mut transitions, mut start, mut high) = (Vec::new(), None, y[0] > VCC / 2.0);
    for k in 0..y.len() {
        let now = match y[k] {
            v if v >= 0.9 * VCC => Some(true),
            v if v <= 0.1 * VCC => Some(false),
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
        landings: t.len() - 1,
        min_step_ps,
        transitions,
    }
}

/// When `y` first crosses `level` after `after` ps, interpolated, in ps.
fn crossing(t: &[f64], y: &[f64], level: f64, after: f64) -> f64 {
    let k = (1..t.len())
        .find(|&k| t[k] > after && (y[k - 1] - level) * (y[k] - level) <= 0.0 && y[k] != y[k - 1])
        .expect("crosses");
    t[k - 1] + (level - y[k - 1]) / (y[k] - y[k - 1]) * (t[k] - t[k - 1])
}

/// Each time `y` crosses `level`, interpolated, in ps.
fn crossings(t: &[f64], y: &[f64], level: f64) -> Vec<f64> {
    (1..t.len())
        .filter(|&k| (y[k - 1] - level) * (y[k] - level) < 0.0)
        .map(|k| t[k - 1] + (level - y[k - 1]) / (y[k] - y[k - 1]) * (t[k] - t[k - 1]))
        .collect()
}

/// How long an integrator at `GBW` takes to move `volts` while its input difference ramps from
/// 0 at `rate` V/s, in the transconductance's linear range: `volts = pi * GBW * rate * t^2`.
fn ramp_delay_ps(volts: f64, rate: f64) -> f64 {
    (volts / (PI * GBW * rate)).sqrt() * 1e12
}

/// Circuit 6. Open-loop comparators against 2.5 V, run on every accepted step. A 1 mA current into
/// 2 uF ramps the input at 500 V/s from 1 ms to 11 ms and back down to 21 ms. The square is
/// 0 V and 5 V at 10 kHz. The hysteresis circuit puts the ramp through 10 kohm into `in_p`,
/// with 100 kohm from the output, so `in_p` reaches 2.5 V when the ramp is
/// `2.75 - 0.1 * V(out)`: 2.7448 V going up from the low swing and 2.2502 V coming down from
/// the high one. Each run has no error, no accepted step shorter than 100 ps, and no
/// transition over 2000 steps.
///
/// Each output crosses 2.5 V after `in_p` does by what one pole allows. The integrator waits
/// past a rail, at most where the clamp carries the whole slew current, `OVERRUN` plus
/// `CLAMP_KNEE * ln(e^10 - 1)`, 0.2 V. So it moves 2.5 V to 2.7 V to reach the middle. On the
/// square that is a slew at 1 V/us, 2.5 us to 2.7 us after each edge. On the ramp the
/// difference grows at 500 V/s and stays in the linear range, so it takes `ramp_delay_ps`,
/// 39.9 us to 41.4 us. With hysteresis `in_p` ramps at 10/11 of a ramp that 10 kohm also loads, above
/// 400 V/s, and the output's rise feeds it, so the delay is under `ramp_delay_ps` at 400 V/s
/// and over the slew.
#[test]
fn a_comparator_switches_in_few_steps_at_its_threshold() {
    let edges: Vec<f64> = (0..10)
        .flat_map(|n| [10e6 + n as f64 * 100e6, 60e6 + n as f64 * 100e6])
        .collect();
    let ramp = (ramp_delay_ps(2.5, 500.0), ramp_delay_ps(2.7, 500.0));
    let slew = (2.5e6, 2.7e6);
    let fed = (slew.0, ramp_delay_ps(2.7, 400.0));
    for (name, input, (early, late)) in [
        ("op-amp-ramp", "#net.ramp", ramp),
        ("op-amp-hysteresis", "#net.inp", fed),
        ("op-amp-square", "", slew),
    ] {
        let results = orbe_engine::simulate_steps(&netlist(name));
        let s = steps(&results);
        let worst = s.transitions.iter().map(|&(_, n)| n).max().unwrap();
        eprintln!(
            "{name}: errors {:?}, landings {}, min step {} ps, {} transitions, worst {} steps",
            errors(&results),
            s.landings,
            s.min_step_ps,
            s.transitions.len(),
            worst
        );
        assert!(
            errors(&results).is_empty(),
            "{name}: {:?}",
            errors(&results)
        );
        assert!(s.min_step_ps >= 100.0, "{name}: {}", s.min_step_ps);
        assert!(worst <= 2000, "{name}: {worst}");
        let t = times(&results);
        let thresholds = match input {
            "" => edges.clone(),
            input => crossings(&t, &column(&results, input), VCC / 2.0),
        };
        assert_eq!(s.transitions.len(), thresholds.len(), "{name}");
        let y = column(&results, "#net.out");
        for &threshold in &thresholds {
            let delay = crossing(&t, &y, VCC / 2.0, threshold) - threshold;
            eprintln!("  {name}: crosses {delay} ps after {threshold} ps, law {early} to {late}");
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
            eprintln!("  {name}: ramp at the thresholds {up} V and {down} V");
            assert!(
                (up - 2.7448).abs() <= 1e-3 && (down - 2.2502).abs() <= 1e-3,
                "{up} {down}"
            );
        }
    }
}
