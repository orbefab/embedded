use crate::common;

use common::simulate;
use orbe_engine::name::Ref;

fn value(results: &orbe_engine::run_folder::Results, reference: &str) -> f64 {
    let op = results.observations().op().unwrap();
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

#[test]
fn a_potentiometer_divider_matches_the_closed_form() {
    let results = simulate(
        r#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin", "mid"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V" }
        },
        {
          "name": "rv1",
          "kind": "potentiometer",
          "pins": { "a": "vin", "wiper": "mid", "b": "gnd" },
          "resistance": { "value": 10000.0, "unit": "ohm" },
          "position": { "value": 0.3, "unit": "1" }
        }
      ]
    }"#,
    );
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let expect = 5.0 * (1.0 - 0.3);
    let mid = value(&results, "#net.mid");
    assert!(
        (mid - expect).abs() <= 1e-8 * expect.abs(),
        "mid {mid} vs {expect}"
    );
    let keys: Vec<_> = results
        .observations()
        .op()
        .unwrap()
        .values
        .keys()
        .map(|reference| reference.to_string())
        .collect();
    assert!(keys.iter().any(|key| key == "#net.mid"));
    assert!(keys.iter().all(|key| !key.starts_with("#rv1")));
}

#[test]
fn both_legs_reach_the_source() {
    let results = simulate(
        r#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin", "mid", "far"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V" }
        },
        {
          "name": "rv1",
          "kind": "potentiometer",
          "pins": { "a": "vin", "wiper": "mid", "b": "far" },
          "resistance": { "value": 10000.0, "unit": "ohm" },
          "position": { "value": 0.3, "unit": "1" }
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
    for net in ["#net.mid", "#net.far"] {
        let volts = value(&results, net);
        assert!((volts - 5.0).abs() <= 1e-6 * 5.0, "{net} {volts}");
    }
}
