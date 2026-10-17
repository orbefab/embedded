use std::fs;
use std::path::PathBuf;

use sim_core::name::Ref;
use sim_core::run_folder::Results;

const IS: f64 = 6.711e-10;
const N: f64 = 4.427;
const RS: f64 = 2.115;

fn simulate(bytes: &[u8]) -> Results {
    let netlist = sim_core::netlist::parse(bytes).unwrap();
    sim_circuit::simulate(&netlist)
}

fn example(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .join("netlist.json");
    fs::read(path).unwrap()
}

fn value(results: &Results, reference: &str) -> f64 {
    let op = results.observations().op.as_ref().expect("operating point");
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

fn led_voltage(i: f64) -> f64 {
    let vt = 1.380649e-23 * 298.15 / 1.602176634e-19;
    N * vt * (i / IS).ln_1p() + RS * i
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
    // Reverse Shockley current is -Is. The internal node's gmin steals Is*gmin*Rs/(1+gmin*Rs).
    let gmin = 1e-12;
    let expected = -IS / (1.0 + gmin * RS);
    assert!((i - expected).abs() <= 1e-18, "{i} vs {expected}");
    assert!((value(&results, "#net.vled") - 5.0).abs() < 1e-6);
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
    assert_eq!(value(&results, "#net.vled"), 3.9205257931291944);
    assert_eq!(value(&results, "#net.mid"), 1.9602628965440472);
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
/// The current and the terminal voltage are the operating point from before the argument cap.
#[test]
fn a_tiny_saturation_current_solves_to_the_pre_cap_point() {
    let cases = [
        (
            "1e-37",
            Some(100.0),
            0.028735671797872175,
            2.126432820000095,
        ),
        (
            "1e-40",
            Some(100.0),
            0.02699436509495179,
            2.3005634902747105,
        ),
        (
            "1e-40",
            Some(100_000.0),
            2.9020204738383626e-05,
            2.097979316341305,
        ),
        ("1e-50", None, 2.023918337867733, 4.999999999999999),
        ("1e-100", None, 5.001332681331405e-12, 5.0),
    ];
    for (is, ohms, current, voltage) in cases {
        let results = simulate(biased_diode(5.0, ohms, is, "1").as_bytes());
        assert!(
            results.diagnostics().is_empty(),
            "{is} @ {ohms:?}: {:?}",
            results.diagnostics()
        );
        let i = value(&results, "#d1.i");
        let v = value(&results, "#d1.v");
        assert_eq!(i, current, "{is} @ {ohms:?}");
        assert_eq!(v, voltage, "{is} @ {ohms:?}");
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
