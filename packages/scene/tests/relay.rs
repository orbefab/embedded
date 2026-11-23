//! The Songle coil and the 1-channel module.
//!
//! The contact closes above the must-operate voltage and opens below must-release.
//! A voltage between the two leaves it. The module's input current is the base
//! resistor, and the coil current comes from VCC.

mod common;

use std::fs;
use std::os::unix::fs::symlink;

use orbe_engine::diagnostic::Severity;
use orbe_engine::logic::Level;
use orbe_scene::Files;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    symlink(common::repo().join("parts"), dir.path().join("parts")).unwrap();
    dir
}

fn run(mut json: serde_json::Value, stop_ps: u64) -> orbe_engine::run_folder::Results {
    json["analysis"]["transient"]["stop_ps"] = serde_json::json!(stop_ps);
    json["analysis"]["transient"]["max_step_ps"] = serde_json::json!(1_000_000_000u64);
    json["analysis"]["transient"]["grid_ps"] = serde_json::json!(1_000_000_000u64);
    let bytes = serde_json::to_vec(&json).unwrap();
    let parsed = orbe_engine::netlist::parse(&bytes).expect("the netlist parses");
    orbe_engine::simulate(&parsed)
}

fn column(results: &orbe_engine::run_folder::Results, name: &str) -> Vec<f64> {
    let trace = results.trace().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|column| column.to_string() == name)
        .unwrap_or_else(|| panic!("no column {name}; have {:?}", trace.columns()));
    trace.column(index).to_vec()
}

fn at(results: &orbe_engine::run_folder::Results, name: &str, t_ps: u64) -> f64 {
    let trace = results.trace().unwrap();
    let values = column(results, name);
    let row = trace
        .time()
        .iter()
        .position(|time| time.as_ps() >= t_ps)
        .unwrap_or_else(|| panic!("{name} has no row at {t_ps}"));
    values[row]
}

fn contact_edges(results: &orbe_engine::run_folder::Results, part: &str) -> Vec<(u64, Level)> {
    let trace = results.trace().unwrap();
    let logic = trace.logic().expect("the relay has a contact");
    let want = format!("#{part}.logic.contact");
    let index = logic
        .signals()
        .iter()
        .position(|signal| signal.to_string() == want)
        .unwrap_or_else(|| panic!("no {want}"));
    logic
        .changes()
        .iter()
        .filter(|change| change.signal == index)
        .map(|change| (change.at.as_ps(), change.level))
        .collect()
}

fn errors(results: &orbe_engine::run_folder::Results) -> Vec<String> {
    results
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.severity() == Severity::Error)
        .map(|diagnostic| diagnostic.code().to_string())
        .collect()
}

const COIL: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "model": {
    "kind": "voltage_source",
    "pins": { "p": "pos", "n": "neg" },
    "voltage": {
      "kind": "steps",
      "initial": { "value": 5, "unit": "V", "source": "user" },
      "events": [
        { "t_ps": 50000000000, "level": { "value": 2, "unit": "V", "source": "user" } },
        { "t_ps": 100000000000, "level": { "value": 0.3, "unit": "V", "source": "user" } }
      ]
    }
  }
}"#;

const FIVE: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "model": {
    "kind": "voltage_source",
    "pins": { "p": "pos", "n": "neg" },
    "voltage": { "value": 5, "unit": "V", "source": "user" }
  }
}"#;

/// 5 V closes the contact, 2 V leaves it closed, and 0.3 V opens it. None of those is an error.
#[test]
fn the_contact_follows_the_coil_voltage_with_hysteresis() {
    let dir = project();
    fs::write(dir.path().join("coil.jsonc"), COIL).unwrap();
    fs::write(dir.path().join("five.jsonc"), FIVE).unwrap();
    let scene = r#"{
      "format": 1,
      "parts": {
        "coil": { "type": "./coil.jsonc" },
        "load": { "type": "./five.jsonc" },
        "k": { "type": "relay-srd-05vdc-sl-c" },
        "rno": { "type": "resistor-axial", "resistance": "1k" }
      },
      "ground": "coil.neg",
      "wires": [
        ["coil.pos", "k.coil_a"],
        ["coil.neg", "k.coil_b"],
        ["load.neg", "coil.neg"],
        ["load.pos", "k.COM"],
        ["k.NO", "rno.a"],
        ["rno.b", "coil.neg"]
      ]
    }"#;
    let path = dir.path().join("scene.jsonc");
    fs::write(&path, scene).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &path).expect("the relay scene checks");
    let json = serde_json::from_str(&netlist.json).unwrap();
    let results = run(json, 150_000_000_000);
    assert!(errors(&results).is_empty(), "{:?}", results.diagnostics());
    let edges = contact_edges(&results, "k");
    // 5 V is on at t = 0, so the operating point closes the contact and writes no edge.
    assert!(
        !edges.iter().any(|(at, _)| *at < 100_000_000_000),
        "the contact moved at 5 V or at 2 V: {edges:?}"
    );
    assert!(
        edges
            .iter()
            .any(|(at, level)| *at > 100_000_000_000 && *level == Level::Low),
        "0.3 V did not open the contact: {edges:?}"
    );
    let closed = at(&results, "#rno.i", 40_000_000_000);
    let held = at(&results, "#rno.i", 80_000_000_000);
    let open = at(&results, "#rno.i", 130_000_000_000);
    assert!(closed > 0.004, "closed load {closed}");
    assert!(held > 0.004, "held load {held}");
    assert!(open.abs() < 1e-4, "open load {open}");
}

/// IN draws the base current. The coil current comes from VCC, and VCC at 0 V leaves the contact open.
#[test]
fn the_module_input_draws_the_driver_and_an_unpowered_coil_stays_open() {
    let dir = project();
    fs::write(dir.path().join("five.jsonc"), FIVE).unwrap();
    let vcc = r#"{
      "format": 1,
      "pins": { "pos": [1], "neg": [2] },
      "model": {
        "kind": "voltage_source",
        "pins": { "p": "pos", "n": "neg" },
        "voltage": {
          "kind": "steps",
          "initial": { "value": 0, "unit": "V", "source": "user" },
          "events": [
            { "t_ps": 20000000000, "level": { "value": 5, "unit": "V", "source": "user" } }
          ]
        }
      }
    }"#;
    fs::write(dir.path().join("vcc.jsonc"), vcc).unwrap();
    let scene = r#"{
      "format": 1,
      "parts": {
        "vcc": { "type": "./vcc.jsonc" },
        "sig": { "type": "./five.jsonc" },
        "mod": { "type": "relay-module-1ch" },
        "rno": { "type": "resistor-axial", "resistance": "1k" }
      },
      "ground": "vcc.neg",
      "wires": [
        ["vcc.neg", "sig.neg"],
        ["vcc.neg", "mod.GND"],
        ["vcc.pos", "mod.VCC"],
        ["sig.pos", "mod.IN"],
        ["vcc.pos", "mod.COM"],
        ["mod.NO", "rno.a"],
        ["rno.b", "vcc.neg"]
      ]
    }"#;
    let path = dir.path().join("scene.jsonc");
    fs::write(&path, scene).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &path).expect("the module scene checks");
    let json = serde_json::from_str(&netlist.json).unwrap();
    let results = run(json, 50_000_000_000);
    assert!(errors(&results).is_empty(), "{:?}", results.diagnostics());
    let edges = contact_edges(&results, "mod__k");
    assert!(
        edges.iter().all(|(at, _)| *at >= 20_000_000_000),
        "the contact moved while VCC was off: {edges:?}"
    );
    assert!(
        edges
            .iter()
            .any(|(at, level)| *at >= 20_000_000_000 && *level == Level::High),
        "5 V on VCC did not close the contact: {edges:?}"
    );
    let coil_off = at(&results, "#mod__k.i", 10_000_000_000).abs();
    let coil_on = at(&results, "#mod__k.i", 40_000_000_000).abs();
    let base = at(&results, "#mod__rb.i", 40_000_000_000).abs();
    assert!(coil_off < 0.001, "unpowered coil {coil_off}");
    assert!(coil_on > 0.05, "coil current {coil_on}");
    assert!(base < 0.01, "input current {base} is the coil");
    assert!(base > 0.001, "the driver draws nothing: {base}");
    let load = at(&results, "#rno.i", 40_000_000_000);
    assert!(load > 0.004, "the contact did not switch the load: {load}");
}
