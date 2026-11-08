use std::fs;
use std::path::PathBuf;

use orbe_engine_core::name::Ref;
use orbe_engine_core::run_folder::Results;

const IS: f64 = 6.711e-10;
const N: f64 = 4.427;
const RS: f64 = 2.115;

fn simulate(bytes: &[u8]) -> Results {
    let netlist = orbe_engine_core::netlist::parse(bytes).unwrap();
    orbe_circuit::simulate(&netlist)
}

fn example(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/circuits")
        .join(name)
        .join("netlist.json");
    fs::read(path).unwrap()
}

fn value(results: &Results, reference: &str) -> f64 {
    let op = results.observations().op().expect("operating point");
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

fn led_voltage(i: f64) -> f64 {
    let vt = 1.380649e-23 * 298.15 / 1.602176634e-19;
    N * vt * libm::log1p(i / IS) + RS * i
}

#[test]
fn each_led_example_sits_on_the_device_curve_and_obeys_kcl() {
    for (name, volts, ohms) in [("led-5v", 5.0, 220.0), ("led-9v", 9.0, 10.0)] {
        let results = simulate(&example(name));
        let i = value(&results, "#led1.i");
        let v = value(&results, "#led1.v");
        let resistor = (volts - v) / ohms;
        assert!(
            (i - resistor).abs() <= 1e-6 * i,
            "{name}: led {i} A, resistor {resistor} A"
        );
        let curve = led_voltage(i);
        assert!(
            (v - curve).abs() <= 1e-6 * v,
            "{name}: {v} V at {i} A, curve {curve} V"
        );
        assert_eq!(value(&results, "#net.vled"), v);
    }
}

#[test]
fn the_5v_example_draws_about_14_ma() {
    let results = simulate(&example("led-5v"));
    let i = value(&results, "#led1.i");
    assert!((0.0135..0.0145).contains(&i), "{i}");
}

#[test]
fn a_reversed_led_blocks() {
    let results = simulate(
        br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin", "vled"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V", "source": "user" }
        },
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "vled" },
          "resistance": { "value": 220.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        },
        {
          "name": "led1",
          "kind": "led",
          "pins": { "anode": "gnd", "cathode": "vled" },
          "saturation_current": { "value": 6.711e-10, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 4.427, "unit": "1", "source": "user" },
          "series_resistance": { "value": 2.115, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null }
        }
      ]
    }"#,
    );
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let i = value(&results, "#led1.i");
    let expected = reverse_current(-5.0, IS, N, RS);
    assert!((i - expected).abs() <= 1e-18, "{i} vs {expected}");
    assert!((value(&results, "#net.vled") - 5.0).abs() < 1e-6);
}

/// Under 1 V of reverse bias, `GMIN * vj` is under the ngspice comparison's 1 pA floor. The
/// tolerance is 1e-15 A, a five-hundredth of `GMIN * 0.5 V` and well inside Newton's 1e-12 A.
#[test]
fn half_a_volt_across_a_reversed_diode_still_takes_gmin() {
    let results = simulate(biased_diode(-0.5, None, "1e-14", "1").as_bytes());
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let i = value(&results, "#d1.i");
    let expected = reverse_current(-0.5, 1e-14, 1.0, 1.0);
    assert!((i - expected).abs() <= 1e-15, "{i} vs {expected}");
}

/// Almost nothing reaches the junction behind 1e20 ohm, so the 2 pA leaves the anode through
/// node gmin and the anode sits at -2 V.
#[test]
fn a_reversed_diode_behind_1e20_ohm_solves() {
    let results = simulate(
        br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "a"],
      "parts": [
        {
          "name": "i1",
          "kind": "current_source",
          "pins": { "p": "a", "n": "gnd" },
          "current": { "value": -2e-12, "unit": "A", "source": "user" }
        },
        {
          "name": "d1",
          "kind": "diode",
          "pins": { "anode": "a", "cathode": "gnd" },
          "saturation_current": { "value": 1e-14, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 1.0, "unit": "1", "source": "user" },
          "series_resistance": { "value": 1e20, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null }
        }
      ]
    }"#,
    );
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let anode = value(&results, "#net.a");
    assert!((anode + 2.0).abs() < 1e-6, "{anode}");
}

#[test]
fn two_leds_in_series_from_9v_are_not_floating() {
    let results = simulate(
        br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "mid", "vin", "vled"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 9.0, "unit": "V", "source": "user" }
        },
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "vled" },
          "resistance": { "value": 330.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        },
        {
          "name": "led1",
          "kind": "led",
          "pins": { "anode": "vled", "cathode": "mid" },
          "saturation_current": { "value": 6.711e-10, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 4.427, "unit": "1", "source": "user" },
          "series_resistance": { "value": 2.115, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null }
        },
        {
          "name": "led2",
          "kind": "led",
          "pins": { "anode": "mid", "cathode": "gnd" },
          "saturation_current": { "value": 6.711e-10, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 4.427, "unit": "1", "source": "user" },
          "series_resistance": { "value": 2.115, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null }
        }
      ]
    }"#,
    );
    assert!(
        results
            .diagnostics()
            .iter()
            .all(|d| d.code() != "net.floating"),
        "{:?}",
        results.diagnostics()
    );
    assert_eq!(value(&results, "#net.vin"), 9.0);
    let vled = value(&results, "#net.vled");
    let mid = value(&results, "#net.mid");
    let i1 = value(&results, "#led1.i");
    let i2 = value(&results, "#led2.i");
    // Node gmin on `mid` is the only current that misses the lower LED.
    let gmin = 1e-12;
    assert!(
        (i1 - i2 - gmin * mid).abs() <= 1e-12,
        "led1 {i1} led2 {i2} mid {mid}"
    );
    assert!(
        ((9.0 - vled) / 330.0 - i1 - gmin * vled).abs() <= 1e-12,
        "vled {vled} led1 {i1}"
    );
    // Each LED is the Shockley law plus GMIN across the junction, and each drops about 2 V.
    assert_on_shockley(i1, value(&results, "#led1.v"), IS, N, RS);
    assert_on_shockley(i2, value(&results, "#led2.v"), IS, N, RS);
    assert!((1.5..2.5).contains(&mid), "{mid}");
    assert!((3.5..4.5).contains(&vled), "{vled}");
}

#[test]
fn a_junction_just_inside_the_domain_still_solves() {
    let results = simulate(diode_probe("1e-308", "90").as_bytes());
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let anode = value(&results, "#net.b");
    assert!((anode - 1.0).abs() < 1e-6, "{anode}");
    let i = value(&results, "#d1.i");
    assert!(i.is_finite() && i.abs() < 1e-9, "{i}");
}

/// 5 V diodes with `n` = 1 and `Rs` = 1. `None` is the diode straight across the source.
/// The operating point is the uncapped Shockley law plus `GMIN` across the junction.
#[test]
fn a_tiny_saturation_current_solves_to_the_pre_cap_point() {
    let cases = [
        ("1e-37", Some(100.0)),
        ("1e-40", Some(100.0)),
        ("1e-40", Some(100_000.0)),
        ("1e-50", None),
        ("1e-100", None),
    ];
    for (is, ohms) in cases {
        let results = simulate(biased_diode(5.0, ohms, is, "1").as_bytes());
        assert!(
            results.diagnostics().is_empty(),
            "{is} @ {ohms:?}: {:?}",
            results.diagnostics()
        );
        let i = value(&results, "#d1.i");
        let v = value(&results, "#d1.v");
        let is_value: f64 = is.parse().unwrap();
        let (expect_i, expect_v) = shockley_operating_point(5.0, ohms, is_value, 1.0);
        let i_tol = 1e-6 * expect_i.abs() + 1e-12;
        let v_tol = 1e-6 * expect_v.abs() + 1e-9;
        assert!(
            (i - expect_i).abs() <= i_tol,
            "{is} @ {ohms:?}: {i} vs {expect_i}"
        );
        assert!(
            (v - expect_v).abs() <= v_tol,
            "{is} @ {ohms:?}: {v} vs {expect_v}"
        );
    }
}

/// `|I - Shockley(vd) - GMIN * vd|` inside Newton's current tolerance. `vd` is the junction
/// voltage, the terminal voltage minus the drop on `Rs`.
fn assert_on_shockley(i: f64, v: f64, is: f64, n: f64, rs: f64) {
    let vd = v - i * rs;
    let nvt = n * orbe_engine_core::netlist::Junction::VT;
    let predicted = is * (libm::exp(vd / nvt) - 1.0) + 1e-12 * vd;
    let tol = 1e-6 * i.abs().max(predicted.abs()) + 1e-12;
    assert!((i - predicted).abs() <= tol, "{i} vs {predicted}");
}

/// The current into a diode whose terminals sit `volts` apart, with `volts` below `-3 * n * VT`.
/// ngspice-42's reverse law `-Is * (1 + (3 * n * VT / (e * vj))^3)` plus `GMIN * vj`, with `vj`
/// taken as `volts`. The drop on `Rs` moves the current by under 1e-18 A.
fn reverse_current(volts: f64, is: f64, n: f64, rs: f64) -> f64 {
    let gmin = 1e-12;
    let arg = 3.0 * n * orbe_engine_core::netlist::Junction::VT / (volts * std::f64::consts::E);
    (-is * (1.0 + arg * arg * arg) + gmin * volts) / (1.0 + gmin * rs)
}

/// Terminal current and voltage of a cathode-at-ground diode. The root is bracketed on the
/// uncapped device law, so a clamped exponential cannot satisfy it.
fn shockley_operating_point(volts: f64, ohms: Option<f64>, is: f64, n: f64) -> (f64, f64) {
    let nvt = n * orbe_engine_core::netlist::Junction::VT;
    let gmin = 1e-12;
    let rs = 1.0;
    let residual = |vd: f64| {
        let device = is * (libm::exp(vd / nvt) - 1.0) + gmin * vd;
        let circuit = match ohms {
            None => (volts - vd) / rs,
            Some(r) => (volts - vd * (1.0 + r * gmin)) / (r + rs * (1.0 + r * gmin)),
        };
        device - circuit
    };
    let (mut lo, mut hi) = (-1.0, volts);
    assert!(residual(lo) < 0.0 && residual(hi) > 0.0);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if residual(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let vd = 0.5 * (lo + hi);
    match ohms {
        None => ((volts - vd) / rs, volts),
        Some(r) => {
            let i = (volts - vd * (1.0 + r * gmin)) / (r + rs * (1.0 + r * gmin));
            (i, vd + i * rs)
        }
    }
}

fn diode_probe(is: &str, n: &str) -> String {
    biased_diode(1.0, Some(1000.0), is, n)
}

fn biased_diode(volts: f64, ohms: Option<f64>, is: &str, n: &str) -> String {
    let (nets, resistor, anode) = match ohms {
        Some(ohms) => (
            r#""gnd", "a", "b""#,
            format!(
                r#"{{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "a", "b": "b" }},
          "resistance": {{ "value": {ohms}, "unit": "ohm", "source": "user" }},
          "ratings": {{ "power": null }}
        }},"#
            ),
            "b",
        ),
        None => (r#""gnd", "a""#, String::new(), "a"),
    };
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": [{nets}],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "a", "n": "gnd" }},
          "voltage": {{ "value": {volts}, "unit": "V", "source": "user" }}
        }},
        {resistor}
        {{
          "name": "d1",
          "kind": "diode",
          "pins": {{ "anode": "{anode}", "cathode": "gnd" }},
          "saturation_current": {{ "value": {is}, "unit": "A", "source": "user" }},
          "emission_coefficient": {{ "value": {n}, "unit": "1", "source": "user" }},
          "series_resistance": {{ "value": 1.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "current": null, "power": null }}
        }}
      ]
    }}"#
    )
}
