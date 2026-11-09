//! The IRF520 switching a load from a Nano. Pin D9 drives the gate directly, through 100 ohm,
//! or through 100 ohm with 10k from the gate to GND. The drain sinks a red LED behind 220 ohm,
//! or 100 ohm, 10 ohm or 1 ohm, from 5V. The 1 ohm load wants 5 A, more than a 5 V gate lets
//! through, so the transistor holds the current at its own limit. `gate_hold.hex` sets D9 high
//! in setup and keeps it there, while D7 rises at 100 ms and D8 toggles every 200 ms.

mod common;

use std::fs;

use orbe_circuit::RunStats;
use orbe_engine_core::diagnostic::Severity;
use orbe_engine_core::run_folder::Results;
use orbe_scene::Files;

const STOP_PS: u64 = 300_000_000_000;

/// Accepted steps over the 300 ms, 1.5 times the most any shape takes. The shapes take 496 to
/// 750.
const MOST_STEPS: u64 = 750 * 3 / 2;

/// The part's values, from `parts/nmos-irf520-to220/part.jsonc`.
const KP: f64 = 1.259;
const VTO: f64 = 3.18;
const LAMBDA: f64 = 0.0014;

#[derive(Clone, Copy, Debug)]
enum Drive {
    Direct,
    Series,
    PulledDown,
}

#[derive(Clone, Copy, Debug)]
enum Load {
    Led,
    Ohms(&'static str),
}

const DRIVES: [Drive; 3] = [Drive::Direct, Drive::Series, Drive::PulledDown];
const LOADS: [Load; 4] = [
    Load::Led,
    Load::Ohms("100"),
    Load::Ohms("10"),
    Load::Ohms("1"),
];

fn scene(drive: Drive, load: Load) -> String {
    let mut parts = vec![
        r#""nano": { "type": "nano-board", "firmware": "build/gate_hold.hex" }"#.to_string(),
        r#""m1": { "type": "nmos-irf520-to220" }"#.to_string(),
    ];
    let mut wires = vec![r#"["m1.source", "nano.GND"]"#.to_string()];
    match drive {
        Drive::Direct => wires.push(r#"["nano.D9", "m1.gate"]"#.into()),
        Drive::Series | Drive::PulledDown => {
            parts.push(r#""rg": { "type": "resistor-axial", "resistance": "100" }"#.into());
            wires.push(r#"["nano.D9", "rg.a"]"#.into());
            wires.push(r#"["rg.b", "m1.gate"]"#.into());
        }
    }
    if let Drive::PulledDown = drive {
        parts.push(r#""rpd": { "type": "resistor-axial", "resistance": "10k" }"#.into());
        wires.push(r#"["m1.gate", "rpd.a"]"#.into());
        wires.push(r#"["rpd.b", "nano.GND"]"#.into());
    }
    match load {
        Load::Led => {
            parts.push(r#""rl": { "type": "resistor-axial", "resistance": "220" }"#.into());
            parts.push(r#""led1": { "type": "led-3mm", "color": "red" }"#.into());
            wires.push(r#"["nano.5V", "rl.a"]"#.into());
            wires.push(r#"["rl.b", "led1.anode"]"#.into());
            wires.push(r#"["led1.cathode", "m1.drain"]"#.into());
        }
        Load::Ohms(ohms) => {
            parts.push(format!(
                r#""rl": {{ "type": "resistor-axial", "resistance": "{ohms}" }}"#
            ));
            wires.push(r#"["nano.5V", "rl.a"]"#.into());
            wires.push(r#"["rl.b", "m1.drain"]"#.into());
        }
    }
    format!(
        r#"{{ "format": 1, "parts": {{ {} }}, "wires": [{}], "nets": {{ "GND": "nano.GND", "gate": "m1.gate", "drain": "m1.drain" }} }}"#,
        parts.join(", "),
        wires.join(", ")
    )
}

/// The shape's netlist, cut to `STOP_PS`, run to its end.
fn run(drive: Drive, load: Load) -> (Results, RunStats) {
    let project = common::project("nano-button");
    fs::copy(
        common::repo().join("packages/engine/tests/firmware/sketch/gate_hold.hex"),
        project.path().join("build/gate_hold.hex"),
    )
    .unwrap();
    let path = project.path().join("mosfet.jsonc");
    fs::write(&path, scene(drive, load)).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &path).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&netlist.json).unwrap();
    json["analysis"]["transient"]["stop_ps"] = STOP_PS.into();
    let netlist = orbe_engine_core::netlist::parse(json.to_string().as_bytes()).unwrap();
    orbe_circuit::simulate_stats(&netlist)
}

/// The value of `key` on the last row.
fn last(results: &Results, key: &str) -> f64 {
    let trace = results.trace().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|column| column.to_string() == key)
        .unwrap_or_else(|| panic!("no column {key}"));
    *trace.column(index).last().unwrap()
}

/// Level 1's drain current at `vgs` and `vds`, both positive.
fn square_law(vgs: f64, vds: f64) -> f64 {
    let vov = vgs - VTO;
    let k = 1.0 + LAMBDA * vds;
    if vds >= vov {
        KP / 2.0 * vov * vov * k
    } else {
        KP * vds * (vov - vds / 2.0) * k
    }
}

fn switches_on(drive: Drive, load: Load) {
    let label = format!("{drive:?} {load:?}");
    let (results, stats) = run(drive, load);
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{label}: {:?}",
        results.diagnostics()
    );
    assert!(
        stats.landings <= MOST_STEPS,
        "{label}: {} steps, bound {MOST_STEPS}",
        stats.landings
    );
    let (vgs, vds, id) = (
        last(&results, "#net.gate"),
        last(&results, "#net.drain"),
        last(&results, "#m1.id"),
    );
    let law = square_law(vgs, vds);
    assert!(
        (id - law).abs() <= 1e-6 * law,
        "{label}: id {id} A at vgs {vgs} V and vds {vds} V, where the square law gives {law} A"
    );
    let wanted = match load {
        Load::Led => (0.013, 0.015),
        Load::Ohms("100") => (0.049, 0.050),
        Load::Ohms("10") => (0.47, 0.48),
        Load::Ohms(_) => (1.7, 2.1),
    };
    assert!(
        (wanted.0..=wanted.1).contains(&id),
        "{label}: id {id} A, outside {wanted:?}"
    );
}

#[test]
fn every_gate_drive_switches_every_load_within_the_step_bound() {
    for drive in DRIVES {
        for load in LOADS {
            switches_on(drive, load);
        }
    }
}
