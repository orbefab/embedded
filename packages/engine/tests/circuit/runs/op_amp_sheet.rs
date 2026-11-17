//! The seven `op-amp-*` circuits, once, for whichever part a [`Sheet`] names.
//! The LMV358 sheet keeps the netlists as committed. The LM358 sheet writes its nine electrical
//! values over each op-amp, leaves `input_headroom` as the netlist has it, and returns the swing
//! loads to `gnd`.

use std::f64::consts::PI;

use orbe_engine::netlist::Netlist;
use orbe_engine::run_folder::Results;

use super::common::examples;

/// Volt. Both sheets run at 5 V.
pub const VCC: f64 = 5.0;
/// The engine's node gmin, siemens.
const GMIN: f64 = 1e-12;
/// The law's supply split, ampere.
const SPLIT_A: f64 = 1e-6;
const MIN_STEP_PS: f64 = 100.0;
const MAX_TRANSITION_STEPS: usize = 2000;
const HYSTERESIS_LAW_V: f64 = 1e-3;

/// The nine electrical `op_amp` fields the sheet overwrites, in SI. `input_headroom` stays
/// the netlist's. The examples set it to 0, which rates no common-mode ceiling.
#[derive(Clone, Copy)]
pub struct Values {
    pub gain_bandwidth: f64,
    pub slew_rate: f64,
    pub open_loop_gain: f64,
    pub input_bias_current: f64,
    pub swing_high: f64,
    pub swing_low: f64,
    pub output_resistance: f64,
    pub short_circuit_current: f64,
    pub supply_current: f64,
}

/// One swing load and the sheet limits it has to meet.
pub struct LoadBand {
    pub ohm: f64,
    pub voh_min: f64,
    pub vol_max: f64,
    pub high: &'static str,
    pub low: &'static str,
}

/// What differs between the two parts. The circuits below do not.
pub struct Sheet {
    pub values: Values,
    /// Write `values` over every op-amp in the netlist.
    pub write_values: bool,
    /// Move each `op-amp-swing` load whose `pins.b` is `vref` onto `gnd`.
    pub swing_to_ground: bool,
    /// The voltage those loads return to. `0` after the move, `2.5` in the file.
    pub swing_return: f64,
    pub loads: &'static [LoadBand],
    /// Datasheet slew, V/us. The band is `slew_width` either side of this, and it does not
    /// follow a value overwritten in the part file.
    pub slew_typical: f64,
    /// The slew band is this share either side of `slew_typical`.
    pub slew_width: f64,
    pub gain_width_10k: f64,
    pub gain_width_200k: f64,
    /// Inclusive. The check also requires the current to be above 0.
    pub isc_min: f64,
    pub isc_max: f64,
    /// Ampere. The bias-current band, the sheet's maximum.
    pub ib_max: f64,
    /// The 10% and 90% levels follow the no-load high stop, rather than the supply.
    pub edges_of_high_stop: bool,
}

fn field_value(values: &Values, field: &str) -> f64 {
    match field {
        "gain_bandwidth" => values.gain_bandwidth,
        "slew_rate" => values.slew_rate,
        "open_loop_gain" => values.open_loop_gain,
        "input_bias_current" => values.input_bias_current,
        "swing_high" => values.swing_high,
        "swing_low" => values.swing_low,
        "output_resistance" => values.output_resistance,
        "short_circuit_current" => values.short_circuit_current,
        "supply_current" => values.supply_current,
        _ => panic!("unknown op-amp field {field}"),
    }
}

fn netlist(sheet: &Sheet, name: &str) -> Netlist {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/circuits")
        .join(name);
    if !sheet.write_values && !sheet.swing_to_ground {
        return examples::parse(&dir);
    }
    let path = dir.join("netlist.json");
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for part in doc["parts"].as_array_mut().unwrap() {
        if sheet.write_values && part["kind"] == "op_amp" {
            for field in [
                "gain_bandwidth",
                "slew_rate",
                "open_loop_gain",
                "input_bias_current",
                "swing_high",
                "swing_low",
                "output_resistance",
                "short_circuit_current",
                "supply_current",
            ] {
                part[field]["value"] = serde_json::json!(field_value(&sheet.values, field));
            }
        }
    }
    if sheet.swing_to_ground && name == "op-amp-swing" {
        let mut moved = Vec::new();
        for part in doc["parts"].as_array_mut().unwrap() {
            if part["kind"] == "resistor" && part["pins"]["b"] == "vref" {
                part["pins"]["b"] = serde_json::json!("gnd");
                moved.push(part["name"].as_str().unwrap_or("?").to_string());
            }
        }
        assert!(
            moved.len() == 4,
            "{name} netlist: expected 4 resistors with pins.b vref, moved {} ({})",
            moved.len(),
            moved.join(", ")
        );
    }
    let bytes = serde_json::to_vec(&doc).unwrap();
    orbe_engine::netlist::parse_with(&bytes, &examples::blobs(&dir))
        .unwrap_or_else(|errors| panic!("{name}: {errors:?}"))
}

fn require_clean(name: &str, results: &Results) {
    assert!(
        results.diagnostics().is_empty(),
        "{name}: {:?}",
        results.diagnostics()
    );
}

fn operating_point(sheet: &Sheet, name: &str) -> impl Fn(&str) -> f64 {
    let results = orbe_engine::simulate(&netlist(sheet, name));
    require_clean(name, &results);
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

fn softplus(x: f64) -> f64 {
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

/// How far past a rail `u` sits while the slew current runs into the clamp.
pub fn clamp_past_rail(slew: f64) -> f64 {
    0.1 + 0.01 * libm::log(libm::exp(slew / 1e5) - 1.0)
}

/// Closed-loop gain of 10 with one pole and an infinite DC gain.
pub fn infinite_gain(gbw: f64, f: f64) -> f64 {
    let x = 10.0 * f / gbw;
    10.0 / (1.0 + x * x).sqrt()
}

/// Closed-loop gain of 10 with DC gain `a0` and unity-gain frequency `gbw`.
pub fn finite_gain(gbw: f64, a0: f64, f: f64) -> f64 {
    let (dc, pole) = (1.0 + a0 / 10.0, f * a0 / gbw);
    a0 / (dc * dc + pole * pole).sqrt()
}

/// How long the integrator takes to move `volts` at `slew`, in ps.
pub fn slew_delay_ps(volts: f64, slew: f64) -> f64 {
    volts / slew * 1e12
}

/// `volts = pi * gbw * rate * t^2`, in ps. `rate` is V/s.
pub fn ramp_delay_ps(volts: f64, rate: f64, gbw: f64) -> f64 {
    (volts / (PI * gbw * rate)).sqrt() * 1e12
}

/// The output into `ohm` returned to `ret`, with the integrator held at `t`.
pub fn saturated(values: &Values, t: f64, ohm: f64, ret: f64) -> f64 {
    (t + values.output_resistance * ret / ohm) / (1.0 + values.output_resistance / ohm)
}

/// The output current that holds `vo` at `held`, integrator saturated at swing `t`.
pub fn limited(values: &Values, t: f64, held: f64) -> f64 {
    let d = 1e-3 * values.short_circuit_current;
    let rl = 500.0 / values.short_circuit_current;
    let law = |i: f64| {
        t - values.output_resistance * i - rl * d * softplus((i - values.short_circuit_current) / d)
            + rl * d * softplus((-i - values.short_circuit_current) / d)
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

/// The ramp voltage at which `in_p` is 2.5 V, `2.75 - 0.1 * V(out)`.
pub fn hysteresis_ramp(vout: f64) -> f64 {
    2.75 - 0.1 * vout
}

fn transition_levels(sheet: &Sheet) -> (f64, f64) {
    let span = if sheet.edges_of_high_stop {
        VCC - sheet.values.swing_high
    } else {
        VCC
    };
    (0.1 * span, 0.9 * span)
}

/// Circuit 1. A follower whose `in_p` sits behind 1 Mohm from 2.5 V.
pub fn bias(sheet: &Sheet) {
    let values = &sheet.values;
    let read = operating_point(sheet, "op-amp-bias");
    let bias = -read("#rin.i");
    let want = values.input_bias_current - GMIN * read("#net.inp");
    let margin = sheet.ib_max - bias.abs();
    eprintln!(
        "op-amp-bias: I(1 Mohm) {bias:e} A, band <= {:e}, margin {margin:e}, law {want:e}",
        sheet.ib_max
    );
    assert!(bias.abs() <= sheet.ib_max, "{bias}");
    assert!((bias - want).abs() <= 1e-16, "{bias} want {want}");
    assert!((read("#net.inp") - 2.5 - want * 1e6).abs() <= 1e-9);
    assert!(bias >= 0.99 * values.input_bias_current, "{bias}");
    let out = read("#net.out");
    assert!((out - read("#net.inp")).abs() <= 1e-4, "follower {out}");
    let sourced = (out - 2.5) / 10e3 - values.input_bias_current - GMIN * out;
    let supply = -read("#vs.i");
    let want =
        values.supply_current + 2.0 * values.input_bias_current + GMIN * VCC + from_vcc(sourced);
    eprintln!(
        "op-amp-bias: supply {supply} A, law {want} A, at 4.95 V {:e} A",
        -read("#rinh.i")
    );
    let newton = 1e-6 * supply + 1e-12;
    assert!(
        (supply - want).abs() <= newton,
        "supply {supply} want {want}"
    );
}

/// The second channel of circuit 1, where 4.95 V is inside the 0.8 V headroom.
pub fn bias_off(sheet: &Sheet) {
    let values = &sheet.values;
    let read = operating_point(sheet, "op-amp-bias");
    let on = -read("#rin.i");
    let off = -read("#rinh.i") + GMIN * read("#net.inh");
    eprintln!("op-amp-bias: bias at 2.5 V {on:e} A, at 4.95 V {off:e} A");
    assert!(on >= 0.99 * values.input_bias_current, "{on}");
    assert!(off.abs() <= 1e-3 * values.input_bias_current, "{off}");
}

/// Circuit 2. Overdriven open loop into 10 kohm and 2 kohm.
pub fn swing(sheet: &Sheet) {
    let read = operating_point(sheet, "op-amp-swing");
    for load in sheet.loads {
        let (voh, vol) = (read(load.high), read(load.low));
        let (want_high, want_low) = (
            saturated(
                &sheet.values,
                VCC - sheet.values.swing_high,
                load.ohm,
                sheet.swing_return,
            ),
            saturated(
                &sheet.values,
                sheet.values.swing_low,
                load.ohm,
                sheet.swing_return,
            ),
        );
        eprintln!(
            "op-amp-swing {} ohm: VOH {voh} V (band >= {}, margin {:e}, law {want_high}), VOL {vol} V (band <= {}, margin {:e}, law {want_low})",
            load.ohm,
            load.voh_min,
            voh - load.voh_min,
            load.vol_max,
            load.vol_max - vol
        );
        assert!(
            voh >= load.voh_min && vol <= load.vol_max,
            "{}: {voh} {vol}",
            load.ohm
        );
        assert!(
            (voh - want_high).abs() <= 1e-5,
            "{}: VOH {voh} want {want_high}",
            load.ohm
        );
        assert!(
            (vol - want_low).abs() <= 1e-5,
            "{}: VOL {vol} want {want_low}",
            load.ohm
        );
    }
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

/// Circuit 3. A follower stepped from 0.5 V to 4.0 V and back.
pub fn slew(sheet: &Sheet) {
    let name = "op-amp-slew";
    let results = orbe_engine::simulate(&netlist(sheet, name));
    require_clean(name, &results);
    let (t, y) = (times(&results), column(&results, "#net.out"));
    let lo = y[0];
    let hi = y[t.iter().position(|&x| x >= 29e6).unwrap()];
    let rise = slope(&t, &y, lo, hi, 10e6);
    let fall = -slope(&t, &y, hi, lo, 30e6);
    let (band_lo, band_hi) = (
        sheet.slew_typical * (1.0 - sheet.slew_width),
        sheet.slew_typical * (1.0 + sheet.slew_width),
    );
    let want = sheet.values.slew_rate / (1.0 + sheet.values.output_resistance / 10e3) / 1e6;
    eprintln!(
        "op-amp-slew: {lo} V to {hi} V, rise {rise} V/us, fall {fall} V/us, band {band_lo} to {band_hi}, law {want}"
    );
    for edge in [rise, fall] {
        assert!((band_lo..=band_hi).contains(&edge), "{edge}");
        assert!((edge - want).abs() <= 2e-3, "{edge} want {want}");
    }
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

/// Circuit 4. Non-inverting gain 10 at 10 kHz and at 200 kHz.
pub fn gain(sheet: &Sheet) {
    let values = &sheet.values;
    for (name, f, width) in [
        ("op-amp-gain-10k", 10e3, sheet.gain_width_10k),
        ("op-amp-gain-200k", 200e3, sheet.gain_width_200k),
    ] {
        let results = orbe_engine::simulate(&netlist(sheet, name));
        require_clean(name, &results);
        let t = times(&results);
        let stop = *t.last().unwrap();
        let start = stop - 6.0 / f * 1e12;
        let gain = amplitude(&t, &column(&results, "#net.out"), f, start)
            / amplitude(&t, &column(&results, "#net.inp"), f, start);
        let band = infinite_gain(values.gain_bandwidth, f);
        let want = finite_gain(values.gain_bandwidth, values.open_loop_gain, f);
        eprintln!(
            "{name}: |G| {gain}, band {band} +- {}, margin {:e}, law {want}",
            width * band,
            width * band - (gain - band).abs()
        );
        assert!((gain - band).abs() <= width * band, "{name}: {gain}");
        assert!(
            (gain - want).abs() <= 5e-3 * want,
            "{name}: {gain} want {want}"
        );
    }
}

/// Circuit 5. The output held at 0 V and at 5 V.
pub fn short(sheet: &Sheet) {
    let values = &sheet.values;
    let read = operating_point(sheet, "op-amp-short");
    let sourcing = read("#v0.i");
    let sinking = -read("#v5.i");
    let (want_source, want_sink) = (
        limited(values, VCC - values.swing_high, 0.0),
        -limited(values, values.swing_low, VCC),
    );
    eprintln!(
        "op-amp-short: sourcing {sourcing} A (band {} to {}, margin {:e}, law {want_source}), sinking {sinking} A (margin {:e}, law {want_sink})",
        sheet.isc_min,
        sheet.isc_max,
        sheet.isc_max - sourcing,
        sheet.isc_max - sinking
    );
    for (amps, want) in [(sourcing, want_source), (sinking, want_sink)] {
        assert!(
            amps > 0.0 && (sheet.isc_min..=sheet.isc_max).contains(&amps),
            "{amps}"
        );
        assert!((amps - want).abs() <= 1e-7, "{amps} want {want}");
    }
    let supply = -read("#vs.i");
    let want = 2.0 * values.supply_current
        + 4.0 * values.input_bias_current
        + from_vcc(sourcing)
        + from_vcc(-sinking);
    assert!((supply - want).abs() <= 1e-9, "supply {supply} want {want}");
}

struct Steps {
    landings: usize,
    min_step_ps: f64,
    transitions: Vec<(f64, usize)>,
}

fn steps(sheet: &Sheet, results: &Results) -> Steps {
    let (t, y) = (times(results), column(results, "#net.out"));
    let (low, high_level) = transition_levels(sheet);
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
        landings: t.len() - 1,
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

/// Circuits 6, 7 and the hysteresis comparator. The three share one window.
pub fn comparators(sheet: &Sheet) {
    let values = &sheet.values;
    let edges: Vec<f64> = (0..10)
        .flat_map(|n| [10e6 + n as f64 * 100e6, 60e6 + n as f64 * 100e6])
        .collect();
    let past = clamp_past_rail(values.slew_rate);
    let (near_v, far_v) = (2.5, 2.5 + past);
    let ramp = (
        ramp_delay_ps(near_v, 500.0, values.gain_bandwidth),
        ramp_delay_ps(far_v, 500.0, values.gain_bandwidth),
    );
    let slew = (
        slew_delay_ps(near_v, values.slew_rate),
        slew_delay_ps(far_v, values.slew_rate),
    );
    let fed = (slew.0, ramp_delay_ps(far_v, 400.0, values.gain_bandwidth));
    let (up_want, down_want) = (
        hysteresis_ramp(values.swing_low),
        hysteresis_ramp(VCC - values.swing_high),
    );
    for (name, input, (early, late)) in [
        ("op-amp-ramp", "#net.ramp", ramp),
        ("op-amp-hysteresis", "#net.inp", fed),
        ("op-amp-square", "", slew),
    ] {
        let results = orbe_engine::simulate_steps(&netlist(sheet, name));
        let s = steps(sheet, &results);
        let worst = s.transitions.iter().map(|&(_, n)| n).max().unwrap();
        eprintln!(
            "{name}: min step {} ps, {} landings, {} transitions, worst {} steps, window {early} to {late} ps",
            s.min_step_ps,
            s.landings,
            s.transitions.len(),
            worst
        );
        require_clean(name, &results);
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
