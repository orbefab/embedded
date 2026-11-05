#[allow(dead_code)]
pub mod examples;

use sim_core::name::Ref;
use sim_core::run_folder::Results;
use sim_core::trace::Trace;

pub fn simulate(json: &str) -> Results {
    let netlist = sim_core::netlist::parse(json.as_bytes()).unwrap();
    sim_circuit::simulate(&netlist)
}

/// Like `simulate`, with a row at every accepted step.
pub fn simulate_steps(json: &str) -> Results {
    let netlist = sim_core::netlist::parse(json.as_bytes()).unwrap();
    sim_circuit::simulate_steps(&netlist)
}

pub fn trace(results: &Results) -> &Trace {
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    results.trace().unwrap()
}

pub fn column<'t>(trace: &'t Trace, reference: &str) -> &'t [f64] {
    let reference: Ref = reference.parse().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|r| *r == reference)
        .unwrap();
    trace.column(index)
}

pub fn span(values: &[f64]) -> f64 {
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    max - min
}

pub fn transient(stop_ps: u64, max_step_ps: u64, nets: &str, parts: &[&str]) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": {max_step_ps}, "grid_ps": {max_step_ps} }} }},
      "ground": "gnd",
      "nets": [{nets}],
      "parts": [{}]
    }}"#,
        parts.join(",")
    )
}

pub fn source(volts: f64) -> String {
    source_on("v1", "vcc", volts)
}

pub fn source_on(name: &str, net: &str, volts: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "voltage_source", "pins": {{ "p": "{net}", "n": "gnd" }},
          "voltage": {{ "value": {volts:?}, "unit": "V", "source": "user" }} }}"#
    )
}

pub fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "resistor", "pins": {{ "a": "{a}", "b": "{b}" }},
          "resistance": {{ "value": {ohm:?}, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }}"#
    )
}

pub fn held_capacitor(name: &str, a: &str, b: &str, farad: f64, initial: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "capacitor", "pins": {{ "a": "{a}", "b": "{b}" }},
          "capacitance": {{ "value": {farad:?}, "unit": "F", "source": "user" }},
          "initial": {{ "value": {initial:?}, "unit": "V", "source": "user" }} }}"#
    )
}
