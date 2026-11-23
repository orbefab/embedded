//! The LM358's values and the bands the `op-amp-*` circuits must meet on them.
//! The nine electrical values are read from `parts/lm358-dip8/part.jsonc`, both bodies. TI
//! SLOS068AB p10 section 5.7, the LM358 column, not the LM358A. The table header is VS = 5 V
//! and TA = 25 °C unless a row says otherwise. A typical is 25 °C (note 2).
//!
//! The part's `input_headroom` is 1.5 V, the VCM maximum of (V+) − 1.5 V at 25 °C. The sheet
//! does not copy it onto the circuits. Those netlists set it to 0, which rates no common-mode
//! ceiling, so these bands do not warn. The law's bias gate stays 0.8 V.
//! None of these bands turns on that difference. 4.95 V is above the common-mode range, so the
//! bias there is printed and not held to a band.
//!
//! A width around a typical is the acceptance where section 5.7 prints no minimum and no
//! maximum. It is fixed with the row, at the same fraction `op_amp.rs` uses for a typical
//! that has no limit.
//!
//! The circuits are the existing `op-amp-*` netlists. The sheet writes the part's nine electrical
//! values over each op-amp and leaves `input_headroom` and every other part as the file has it.
//! `op-amp-swing` in the file
//! returns its loads to 2.5 V. Section 5.7 returns RL to V−, so for that one circuit each
//! load's far end moves to `gnd`.

use super::op_amp_sheet::{self, LoadBand, Sheet, VCC, Values};

/// Ampere. IB maximum at 25 °C, −250 nA into the pin.
const IB_MAX: f64 = 250e-9;
/// Volt. VOH is at least VS − 1.5 V on the 5 V positive-rail row. Both swing loads are
/// at least 2 kohm, returned to V−.
const VOH_MIN: f64 = 3.5;
/// Volt. VOL is at most 20 mV on the 5 V negative-rail row, over 0 °C to 70 °C. Both
/// swing loads are at most 10 kohm, returned to V−.
const VOL_MAX: f64 = 0.020;
/// Ampere. ISC maximum is ±60 mA at VS = 10 V, VO = VS / 2. The short circuit runs at 5 V.
const ISC_MAX: f64 = 0.060;
/// V/µs. Section 5.7's SR typical. The band is `SLEW_WIDTH` either side of it.
const SLEW_TYPICAL: f64 = 0.3;
/// Share of the 0.3 V/µs typical. Section 5.7 gives no slew minimum or maximum.
const SLEW_WIDTH: f64 = 0.20;
/// Share of the infinite-DC-gain closed-loop value. Section 5.7 gives no gain-vs-frequency
/// limit. 2% at 10 kHz and 10% at 200 kHz.
const GAIN_WIDTH_10K: f64 = 0.02;
const GAIN_WIDTH_200K: f64 = 0.10;

fn near(got: f64, want: f64, tol: f64) -> bool {
    (got - want).abs() <= tol
}

/// `part.jsonc` allows a `//` comment and a trailing comma. `serde_json` accepts neither.
fn json_of_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if c == ',' {
            let mut look = chars.clone();
            let next = loop {
                match look.next() {
                    Some(w) if w.is_whitespace() => {}
                    other => break other,
                }
            };
            if matches!(next, Some('}' | ']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// `field` on both bodies of a part file. The bodies match apart from `pins`.
fn catalog_value(rel: &str, field: &str, unit: &str) -> f64 {
    let path = super::common::repo(rel);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let doc: serde_json::Value = serde_json::from_str(&json_of_jsonc(&text))
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let body = &doc["model"]["a"][field];
    assert_eq!(
        body,
        &doc["model"]["b"][field],
        "{}: model.b {field} differs from model.a",
        path.display()
    );
    assert_eq!(body["unit"].as_str().unwrap_or(""), unit, "{rel} {field}");
    body["value"]
        .as_f64()
        .unwrap_or_else(|| panic!("{rel} {field} has no number"))
}

/// Both bodies of `parts/lm358-dip8/part.jsonc`, which have to carry one set of values. Only
/// `a` carries `ratings.supply_voltage`, since both share the supply pins.
fn part_values() -> Values {
    let path = super::common::repo("parts/lm358-dip8/part.jsonc");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let doc: serde_json::Value = serde_json::from_str(&json_of_jsonc(&text))
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let model = &doc["model"];
    let (mut a, mut b) = (model["a"].clone(), model["b"].clone());
    for body in [&mut a, &mut b] {
        body.as_object_mut()
            .unwrap_or_else(|| panic!("{} model is not an object", path.display()))
            .remove("pins");
    }
    a["ratings"]
        .as_object_mut()
        .unwrap_or_else(|| panic!("{} model.a has no ratings", path.display()))
        .remove("supply_voltage")
        .unwrap_or_else(|| panic!("{} model.a rates no supply", path.display()));
    assert_eq!(
        a,
        b,
        "{}: model.b differs from model.a apart from pins",
        path.display()
    );
    let body = &model["a"];
    let field = |name: &str, unit: &str| -> f64 {
        let got = body[name]["unit"].as_str().unwrap_or("");
        assert_eq!(got, unit, "{name} unit {got}");
        body[name]["value"]
            .as_f64()
            .unwrap_or_else(|| panic!("{name} has no number"))
    };
    Values {
        gain_bandwidth: field("gain_bandwidth", "Hz"),
        slew_rate: field("slew_rate", "V/s"),
        open_loop_gain: field("open_loop_gain", "1"),
        input_bias_current: field("input_bias_current", "A"),
        swing_high: field("swing_high", "V"),
        swing_low: field("swing_low", "V"),
        output_resistance: field("output_resistance", "ohm"),
        short_circuit_current: field("short_circuit_current", "A"),
        supply_current: field("supply_current", "A"),
    }
}

fn lm358() -> Sheet {
    Sheet {
        values: part_values(),
        write_values: true,
        swing_to_ground: true,
        swing_return: 0.0,
        loads: &[
            LoadBand {
                ohm: 10e3,
                voh_min: VOH_MIN,
                vol_max: VOL_MAX,
                high: "#net.h10k",
                low: "#net.l10k",
            },
            LoadBand {
                ohm: 2e3,
                voh_min: VOH_MIN,
                vol_max: VOL_MAX,
                high: "#net.h2k",
                low: "#net.l2k",
            },
        ],
        slew_typical: SLEW_TYPICAL,
        slew_width: SLEW_WIDTH,
        gain_width_10k: GAIN_WIDTH_10K,
        gain_width_200k: GAIN_WIDTH_200K,
        isc_min: 0.0,
        isc_max: ISC_MAX,
        ib_max: IB_MAX,
        edges_of_high_stop: true,
    }
}

/// The part file carries section 5.7's typicals, and the hand-solved law meets the rows that
/// print a limit. The nine numbers are the file's, not a second copy. `input_headroom` is the
/// part's too, and the LMV358's is its own.
#[test]
fn the_pass_bands_are_the_sheet_limits() {
    let path = "parts/lm358-dip8/part.jsonc";
    let v = part_values();
    let sheet = [
        ("gain_bandwidth", v.gain_bandwidth, 700_000.0),
        ("slew_rate", v.slew_rate, 300_000.0),
        ("open_loop_gain", v.open_loop_gain, 100_000.0),
        ("input_bias_current", v.input_bias_current, 20e-9),
        ("swing_high", v.swing_high, 1.21),
        ("swing_low", v.swing_low, 0.005),
        ("output_resistance", v.output_resistance, 30.0),
        ("short_circuit_current", v.short_circuit_current, 0.04),
        ("supply_current", v.supply_current, 350e-6),
    ];
    for (name, got, want) in sheet {
        assert!(
            near(got, want, want.abs() * 1e-12 + 1e-15),
            "{path} {name} is {got}, section 5.7 typical {want}"
        );
    }
    assert_eq!(
        catalog_value("parts/lm358-dip8/part.jsonc", "input_headroom", "V"),
        1.5
    );
    assert_eq!(
        catalog_value("parts/lmv358-msop8/part.jsonc", "input_headroom", "V"),
        0.8
    );

    let past = op_amp_sheet::clamp_past_rail(v.slew_rate);
    assert!(near(past, 0.129489308190573, 1e-12), "{past}");
    let (early, late) = (2.5, 2.5 + past);
    assert!(near(
        op_amp_sheet::slew_delay_ps(early, v.slew_rate),
        8.333333333333334e6,
        1.0
    ));
    assert!(near(
        op_amp_sheet::slew_delay_ps(late, v.slew_rate),
        8.764964360635243e6,
        1.0
    ));
    assert!(near(
        op_amp_sheet::ramp_delay_ps(early, 500.0, v.gain_bandwidth),
        4.768272270088961e7,
        1.0
    ));
    assert!(near(
        op_amp_sheet::ramp_delay_ps(late, 500.0, v.gain_bandwidth),
        4.890201405176436e7,
        1.0
    ));
    assert!(near(
        op_amp_sheet::ramp_delay_ps(late, 400.0, v.gain_bandwidth),
        5.467411382819752e7,
        1.0
    ));

    let gain_10k = op_amp_sheet::infinite_gain(v.gain_bandwidth, 10e3);
    let gain_200k = op_amp_sheet::infinite_gain(v.gain_bandwidth, 200e3);
    assert!(near(gain_10k, 9.899494936611665, 1e-12), "{gain_10k}");
    assert!(near(gain_200k, 3.303504247281061, 1e-12), "{gain_200k}");
    assert!(near(
        op_amp_sheet::finite_gain(v.gain_bandwidth, v.open_loop_gain, 10e3),
        9.898524880203445,
        1e-12
    ));
    assert!(near(
        op_amp_sheet::finite_gain(v.gain_bandwidth, v.open_loop_gain, 200e3),
        3.303468194463312,
        1e-12
    ));
    assert!(
        (op_amp_sheet::finite_gain(v.gain_bandwidth, v.open_loop_gain, 10e3) - gain_10k).abs()
            <= GAIN_WIDTH_10K * gain_10k
    );
    assert!(
        (op_amp_sheet::finite_gain(v.gain_bandwidth, v.open_loop_gain, 200e3) - gain_200k).abs()
            <= GAIN_WIDTH_200K * gain_200k
    );

    let slew = v.slew_rate / (1.0 + v.output_resistance / 10e3) / 1e6;
    assert!(
        (slew - SLEW_TYPICAL).abs() <= SLEW_WIDTH * SLEW_TYPICAL,
        "{slew}"
    );

    for ohm in [10e3, 2e3] {
        let voh = op_amp_sheet::saturated(&v, VCC - v.swing_high, ohm, 0.0);
        let vol = op_amp_sheet::saturated(&v, v.swing_low, ohm, 0.0);
        assert!(voh >= VOH_MIN, "{ohm}: {voh}");
        assert!(vol <= VOL_MAX, "{ohm}: {vol}");
    }
    let sourcing = op_amp_sheet::limited(&v, VCC - v.swing_high, 0.0);
    let sinking = -op_amp_sheet::limited(&v, v.swing_low, VCC);
    assert!(sourcing > 0.0 && sourcing <= ISC_MAX, "{sourcing}");
    assert!(sinking > 0.0 && sinking <= ISC_MAX, "{sinking}");
}

/// Circuit 1. The sheet band is the 250 nA IB maximum. 2.5 V is inside the common-mode
/// range. 4.95 V is above (V+) − 1.5 V, so that point has no sheet band.
#[test]
fn the_bias_current_leaves_each_input() {
    op_amp_sheet::bias(&lm358());
}

/// Circuit 2. Both loads return to V− and meet the 5 V rows, so VOH is at least 3.5 V and
/// VOL is at most 20 mV.
#[test]
fn the_output_swings_to_its_rails() {
    op_amp_sheet::swing(&lm358());
}

/// Circuit 3. The sheet gives 0.3 V/µs and no limit, so the band is 20% either side. The
/// follower is asked for 4.0 V and the output stops about 1.21 V below 5 V. The slope is 20%
/// to 80% of the output it reaches.
#[test]
fn the_output_slews_at_the_slew_rate() {
    op_amp_sheet::slew(&lm358());
}

/// Circuit 4. The bands are 2% and 10% around the infinite-DC-gain gain. The law is the
/// finite `A0` value.
#[test]
fn the_closed_loop_gain_falls_with_one_pole() {
    op_amp_sheet::gain(&lm358());
}

/// Circuit 5. The sheet maximum is 60 mA at VS = 10 V. The circuit shorts at 5 V.
#[test]
fn the_output_current_is_limited_both_ways() {
    op_amp_sheet::short(&lm358());
}

/// Circuit 6. A transition is 10% to 90% of the no-load high stop. The delay windows are the
/// clamp overrun at this slew rate and this gain-bandwidth product.
#[test]
fn a_comparator_switches_in_few_steps_at_its_threshold() {
    op_amp_sheet::comparators(&lm358());
}
