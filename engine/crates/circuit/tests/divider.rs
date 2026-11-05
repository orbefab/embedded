use std::fs;
use std::path::PathBuf;

use sim_core::diagnostic::Severity;
use sim_core::name::Ref;
use sim_core::run_folder;
use sim_core::timestamp::Timestamp;

fn read_example() -> Vec<u8> {
    fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/divider/netlist.json"))
        .unwrap()
}

fn simulate_bytes(bytes: &[u8]) -> sim_core::run_folder::Results {
    let netlist = sim_core::netlist::parse(bytes).unwrap();
    sim_circuit::simulate(&netlist)
}

fn value(results: &sim_core::run_folder::Results, reference: &str) -> f64 {
    let op = results.observations().op().unwrap();
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

#[test]
fn divider_matches_the_closed_form_and_the_sign_rule() {
    let results = simulate_bytes(&read_example());
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let expect = 10.0 / 3.0;
    let vout = value(&results, "#net.vout");
    assert!(
        (vout - expect).abs() <= 1e-8 * expect.abs(),
        "vout {vout} vs {expect}"
    );
    assert_eq!(value(&results, "#net.gnd"), 0.0);
    assert!((value(&results, "#net.vin") - 5.0).abs() <= 1e-8 * 5.0);

    assert!(value(&results, "#r1.i") > 0.0);
    assert!(value(&results, "#r2.i") > 0.0);
    assert!(value(&results, "#v1.i") < 0.0);
    assert!(value(&results, "#r1.p") > 0.0);
    assert!(value(&results, "#r2.p") > 0.0);
    assert!(value(&results, "#v1.p") < 0.0);
    let sum = value(&results, "#r1.p") + value(&results, "#r2.p") + value(&results, "#v1.p");
    assert!(
        sum.abs() < 1e-9,
        "power sum {sum} is the gmin residual, not a real source or load"
    );
}

#[test]
fn two_renders_of_the_divider_are_the_same_bytes() {
    let bytes = read_example();
    let netlist = sim_core::netlist::parse(&bytes).unwrap();
    let at = Timestamp::from_unix_seconds(1_609_459_200);
    let first = sim_circuit::simulate(&netlist);
    let second = sim_circuit::simulate(&netlist);
    let left = run_folder::render(&bytes, Some(&netlist), &first, at);
    let right = run_folder::render(&bytes, Some(&netlist), &second, at);
    assert_eq!(left, right);
}

const SHUFFLED_DIVIDER: &[u8] = br#"{
  "version": 0,
  "seed": 7,
  "analysis": { "op": {} },
  "ground": "gnd",
  "nets": ["vout", "gnd", "vin"],
  "parts": [
    {
      "name": "r2",
      "kind": "resistor",
      "pins": { "b": "gnd", "a": "vout" },
      "resistance": { "value": 2000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    },
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "n": "gnd", "p": "vin" },
      "voltage": { "value": 5.0, "unit": "V", "source": "user" }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "b": "vout", "a": "vin" },
      "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    }
  ]
}"#;

#[test]
fn reordering_parts_and_nets_keeps_observations_and_diagnostics() {
    let original = simulate_bytes(&read_example());
    let shuffled = simulate_bytes(SHUFFLED_DIVIDER);
    let at = Timestamp::from_unix_seconds(0);
    let netlist = sim_core::netlist::parse(&read_example()).unwrap();
    let left = run_folder::render(&read_example(), Some(&netlist), &original, at);
    let right_net = sim_core::netlist::parse(SHUFFLED_DIVIDER).unwrap();
    let right = run_folder::render(b"shuffled", Some(&right_net), &shuffled, at);
    assert_eq!(
        left.file("observations.json"),
        right.file("observations.json")
    );
    assert_eq!(
        left.file("diagnostics.json"),
        right.file("diagnostics.json")
    );
}

#[test]
fn a_resistor_onto_an_otherwise_untouched_net_is_not_floating() {
    let results = simulate_bytes(
        br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin", "lonely"],
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
          "pins": { "a": "vin", "b": "lonely" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
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
    assert!(results.observations().op().is_some());
}

#[test]
fn a_net_with_no_dc_path_to_ground_is_floating() {
    let results = simulate_bytes(
        br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["orphan", "gnd", "isle2", "vin", "isle"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V", "source": "user" }
        },
        {
          "name": "risle",
          "kind": "resistor",
          "pins": { "a": "isle", "b": "isle2" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        }
      ]
    }"#,
    );
    let floating: Vec<_> = results
        .diagnostics()
        .iter()
        .filter(|d| d.code() == "net.floating")
        .map(|d| d.refs()[0].to_string())
        .collect();
    assert_eq!(floating, ["#net.isle", "#net.isle2", "#net.orphan"]);
    assert!(
        results
            .diagnostics()
            .iter()
            .all(|d| d.severity() == Severity::Warning)
    );
    assert!(results.observations().op().is_some());
    assert!((value(&results, "#net.vin") - 5.0).abs() <= 1e-8 * 5.0);
}

#[test]
fn parallel_voltage_sources_are_singular_at_the_second_source() {
    let results = simulate_bytes(
        br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V", "source": "user" }
        },
        {
          "name": "v2",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 3.0, "unit": "V", "source": "user" }
        }
      ]
    }"#,
    );
    assert!(results.observations().op().is_none());
    let singular: Vec<_> = results
        .diagnostics()
        .iter()
        .filter(|d| d.code() == "solver.singular")
        .collect();
    assert_eq!(singular.len(), 1, "{:?}", results.diagnostics());
    assert_eq!(singular[0].refs(), vec!["#v2".parse().unwrap()]);
    assert_eq!(singular[0].severity(), Severity::Error);
}
