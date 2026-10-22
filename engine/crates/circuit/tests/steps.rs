#[allow(dead_code)]
mod common;

use std::path::PathBuf;

use common::{column, simulate, simulate_steps, span, trace};
use sim_core::netlist::{self, Analysis, Model};
use sim_core::run_folder::{RunFolder, render};
use sim_core::time::Time;
use sim_core::timestamp::Timestamp;

fn level(value: f64, unit: &str) -> String {
    format!(r#"{{ "value": {value:?}, "unit": "{unit}", "source": "user" }}"#)
}

fn divider(voltage: &str, stop_ps: u64) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": 1000, "grid_ps": 1000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vin", "mid"],
      "parts": [
        {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vin", "n": "gnd" }}, "voltage": {voltage} }},
        {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "vin", "b": "mid" }},
           "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }},
        {{ "name": "r2", "kind": "resistor", "pins": {{ "a": "mid", "b": "gnd" }},
           "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }}
      ]
    }}"#
    )
}

fn rows(json: &str) -> (Vec<u64>, Vec<Vec<f64>>) {
    let results = simulate(json);
    let traced = trace(&results);
    let time = traced.time().iter().map(|time| time.as_ps()).collect();
    let values = (0..traced.columns().len())
        .map(|index| traced.column(index).to_vec())
        .collect();
    (time, values)
}

#[test]
fn one_steps_event_matches_a_step_and_no_events_match_the_value() {
    let step = divider(
        &format!(
            r#"{{ "kind": "step", "initial": {}, "final": {}, "at_ps": 5000 }}"#,
            level(0.0, "V"),
            level(5.0, "V")
        ),
        10_000,
    );
    let steps = divider(
        &format!(
            r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": 5000, "level": {} }}] }}"#,
            level(0.0, "V"),
            level(5.0, "V")
        ),
        10_000,
    );
    assert_eq!(rows(&step), rows(&steps));

    let plain = divider(&level(5.0, "V"), 10_000);
    let empty = divider(
        &format!(
            r#"{{ "kind": "steps", "initial": {}, "events": [] }}"#,
            level(5.0, "V")
        ),
        10_000,
    );
    assert_eq!(rows(&plain), rows(&empty));
}

#[test]
fn a_pulse_rewritten_as_steps_writes_the_same_run_folder() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/v-pulse-rc/netlist.json");
    let pulse = std::fs::read(&path).unwrap();
    let steps = pulse_as_steps(&pulse);
    assert_eq!(jumps(&pulse), jumps(&steps));
    same_run(&pulse, &steps);
}

fn pulse_as_steps(pulse: &[u8]) -> Vec<u8> {
    let mut doc: serde_json::Value = serde_json::from_slice(pulse).unwrap();
    let voltage = doc["parts"][0]["voltage"].clone();
    assert_eq!(voltage["kind"], "pulse");
    let initial = voltage["initial"].clone();
    let pulsed = voltage["pulsed"].clone();
    let event =
        |t_ps: u64, level: serde_json::Value| serde_json::json!({ "t_ps": t_ps, "level": level });
    doc["parts"][0]["voltage"] = serde_json::json!({
        "kind": "steps",
        "initial": initial,
        "events": [
            event(200_000_000, pulsed.clone()),
            event(400_000_000, initial.clone()),
            event(800_000_000, pulsed),
            event(1_000_000_000, initial),
        ]
    });
    serde_json::to_vec(&doc).unwrap()
}

fn jumps(input: &[u8]) -> Vec<u64> {
    let netlist = netlist::parse(input).unwrap();
    let source = netlist
        .parts()
        .find_map(|(_, part)| match &part.model {
            Model::VoltageSource(source) => Some(source),
            _ => None,
        })
        .expect("a voltage source");
    let Analysis::Transient(transient) = netlist.analysis() else {
        panic!("not a transient");
    };
    let wave = &source.voltage;
    std::iter::successors(wave.next_change_after(Time::ZERO), |at| {
        wave.next_change_after(*at)
    })
    .take_while(|at| *at < transient.stop)
    .map(Time::as_ps)
    .collect()
}

fn same_run(left: &[u8], right: &[u8]) {
    let left = render_at(left);
    let right = render_at(right);
    let names: Vec<_> = left.files().map(|(name, _)| name.to_string()).collect();
    let other: Vec<_> = right.files().map(|(name, _)| name.to_string()).collect();
    assert_eq!(names, other);
    for name in names {
        if name == "netlist.json" {
            continue;
        }
        let a = left.file(&name).unwrap();
        let b = right.file(&name).unwrap();
        if name == "manifest.json" {
            assert_eq!(manifest_body(a), manifest_body(b), "{name}");
        } else {
            assert_eq!(a, b, "{name}");
        }
    }
}

fn render_at(input: &[u8]) -> RunFolder {
    let netlist = netlist::parse(input).unwrap();
    let results = sim_circuit::simulate(&netlist);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    render(
        input,
        Some(&netlist),
        &results,
        Timestamp::from_unix_seconds(1_700_000_000),
    )
}

fn manifest_body(bytes: &[u8]) -> serde_json::Value {
    let mut doc: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    doc["input"]["sha256"] = serde_json::Value::Null;
    doc["input"]["bytes"] = serde_json::Value::Null;
    doc
}

#[test]
fn a_wiper_move_holds_the_old_divider_and_the_capacitor_follows_the_new_one() {
    let position = format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": {EVENT_PS}, "level": {} }}] }}"#,
        level(0.25, "1"),
        level(0.75, "1")
    );
    let json = format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": 100000, "max_step_ps": 10000, "grid_ps": 10000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vin", "mid"],
      "parts": [
        {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vin", "n": "gnd" }},
           "voltage": {{ "value": 4.0, "unit": "V", "source": "user" }} }},
        {{ "name": "rv1", "kind": "potentiometer", "pins": {{ "a": "vin", "wiper": "mid", "b": "gnd" }},
           "resistance": {{ "value": 10000.0, "unit": "ohm", "source": "user" }},
           "position": {position} }},
        {{ "name": "c1", "kind": "capacitor", "pins": {{ "a": "mid", "b": "gnd" }},
           "capacitance": {{ "value": {FARAD:?}, "unit": "F", "source": "user" }},
           "initial": null }}
      ]
    }}"#
    );
    let results = simulate_steps(&json);
    let traced = trace(&results);
    let rows: Vec<u64> = traced.time().iter().map(|time| time.as_ps()).collect();
    let mid = column(traced, "#net.mid");
    let current = column(traced, "#c1.i");
    let at_event = rows.iter().position(|&time| time == EVENT_PS).unwrap();
    assert!(at_event > 0);
    assert!((mid[at_event] - mid[at_event - 1]).abs() <= 1e-8);
    assert!((mid[at_event] - 3.0).abs() <= 1e-8, "{}", mid[at_event]);
    assert!(
        rows.binary_search(&(EVENT_PS + 1_000)).is_ok(),
        "no row 1 ns after the move"
    );
    let (v_span, i_span) = (span(mid), span(current));
    let mut worst = (0.0f64, 0.0f64);
    for (row, &at) in rows.iter().enumerate() {
        let (volts, amps) = wiper_closed_form(at);
        worst.0 = worst.0.max((mid[row] - volts).abs() / v_span);
        worst.1 = worst.1.max((current[row] - amps).abs() / i_span);
    }
    // docs/engine.md promises under 1e-4 of the swing.
    assert!(
        worst.0 <= 1e-4 && worst.1 <= 1e-4,
        "voltage {:e} of {v_span:e}, current {:e} of {i_span:e}",
        worst.0,
        worst.1
    );
}

const EVENT_PS: u64 = 20_000;
const FARAD: f64 = 1e-12;

fn wiper_closed_form(t_ps: u64) -> (f64, f64) {
    let r_th = |position: f64| position * (1.0 - position) * 10_000.0;
    let volts = |position: f64| 4.0 * (1.0 - position);
    if t_ps <= EVENT_PS {
        return (volts(0.25), 0.0);
    }
    let (v_th, before) = (volts(0.75), volts(0.25));
    let tau = r_th(0.75) * FARAD;
    let decay = (-((t_ps - EVENT_PS) as f64 * 1e-12) / tau).exp();
    let v = v_th + (before - v_th) * decay;
    (v, (v_th - v) / r_th(0.75))
}
