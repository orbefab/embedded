//! A 74HC595 sources an LED through 220 ohm, and eight outputs on 22 ohm go past
//! the sheet's pin and supply currents.

use std::str::FromStr;

use orbe_engine::Session;
use orbe_engine::blob::Blobs;
use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::name::Ref;
use orbe_model::Time;

fn val(value: f64, unit: &str) -> serde_json::Value {
    serde_json::json!({ "value": value, "unit": unit })
}

fn stage(high: f64, low: f64) -> serde_json::Value {
    serde_json::json!({
        "high_drop": val(0.0, "V"),
        "high_resistance": val(high, "ohm"),
        "low_drop": val(0.0, "V"),
        "low_resistance": val(low, "ohm"),
    })
}

fn source(name: &str, net: &str, volts: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": "voltage_source",
        "pins": { "p": net, "n": "gnd" },
        "voltage": val(volts, "V"),
    })
}

fn steps(name: &str, net: &str, events: &[(u64, f64)]) -> serde_json::Value {
    let events: Vec<_> = events
        .iter()
        .map(|(t, volts)| serde_json::json!({ "t_ps": t, "level": val(*volts, "V") }))
        .collect();
    serde_json::json!({
        "name": name,
        "kind": "voltage_source",
        "pins": { "p": net, "n": "gnd" },
        "voltage": {
            "kind": "steps",
            "initial": val(0.0, "V"),
            "events": events,
        },
    })
}

fn register(
    loads: Vec<serde_json::Value>,
    clocks: serde_json::Value,
    latch: serde_json::Value,
) -> serde_json::Value {
    let mut nets = [
        "gnd", "vcc", "ser", "srclk", "rclk", "qa", "qb", "qc", "qd", "qe", "qf", "qg", "qh", "qhp",
    ]
    .map(str::to_string)
    .to_vec();
    let mut parts = vec![
        source("vcc", "vcc", 5.0),
        source("ser", "ser", 5.0),
        clocks,
        latch,
        serde_json::json!({
            "name": "u1",
            "kind": "hc595",
            "pins": {
                "QB": "qb", "QC": "qc", "QD": "qd", "QE": "qe", "QF": "qf", "QG": "qg", "QH": "qh",
                "GND": "gnd", "QHp": "qhp", "SRCLR": "vcc", "SRCLK": "srclk", "RCLK": "rclk",
                "OE": "gnd", "SER": "ser", "QA": "qa", "VCC": "vcc",
            },
            "output": stage(33.333333333333336, 28.333333333333332),
            "serial_output": stage(50.0, 42.5),
            "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
            "ratings": {
                "pin_current": val(0.035, "A"),
                "supply_current": val(0.07, "A"),
            },
        }),
    ];
    for load in loads {
        if let Some(net) = load.get("net").and_then(|net| net.as_str()) {
            nets.push(net.to_string());
        }
        parts.push(load["part"].clone());
    }
    serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": 50_000_000_000u64, "max_step_ps": 1_000_000_000u64, "grid_ps": 1_000_000_000u64 } },
        "ground": "gnd",
        "nets": nets,
        "parts": parts,
    })
}

fn led(name: &str, output: &str) -> serde_json::Value {
    let mid = format!("{name}_a");
    serde_json::json!({
        "net": mid,
        "part": {
            "name": name,
            "kind": "led",
            "pins": { "anode": mid, "cathode": "gnd" },
            "saturation_current": val(6.711e-10, "A"),
            "emission_coefficient": val(4.427, "1"),
            "series_resistance": val(2.115, "ohm"),
            "ratings": { "current": val(0.03, "A"), "power": val(0.075, "W") },
        },
        "series": {
            "name": format!("r_{name}"),
            "kind": "resistor",
            "pins": { "a": output, "b": mid },
            "resistance": val(220.0, "ohm"),
            "ratings": { "power": null },
        },
    })
}

fn open(netlist: serde_json::Value) -> Session {
    Session::open_json(netlist.to_string().as_bytes(), &Blobs::new())
        .unwrap_or_else(|err| panic!("{err:?}\n{netlist}"))
}

fn amps(session: &Session, reference: &str) -> f64 {
    session
        .probe()
        .get(&Ref::from_str(reference).unwrap())
        .unwrap_or_else(|| panic!("{reference} is not a column"))
        .get()
}

#[test]
fn one_latched_bit_lights_qa_and_leaves_qh_dark() {
    let qa = led("qa_led", "qa");
    let qh = led("qh_led", "qh");
    let netlist = register(
        vec![
            serde_json::json!({ "net": null, "part": qa["series"] }),
            qa,
            serde_json::json!({ "net": null, "part": qh["series"] }),
            qh,
        ],
        steps("srclk", "srclk", &[(1_000_000, 5.0), (20_000_000, 0.0)]),
        steps("rclk", "rclk", &[(200_000_000, 5.0)]),
    );
    let mut session = open(netlist);
    session.advance_to(Time::from_ps(1_000_000_000));
    let qa_i = amps(&session, "#qa_led.i");
    let qh_i = amps(&session, "#qh_led.i");
    assert!(
        (8.0e-3..16.0e-3).contains(&qa_i),
        "QA LED current is {qa_i}"
    );
    assert!(qh_i < 1.0e-4, "QH LED current is {qh_i}");
    assert!(
        session
            .diagnostics()
            .iter()
            .all(|d| !matches!(d, Diagnostic::RatingCurrent(_))),
        "{:?}",
        session.diagnostics()
    );
}

#[test]
fn eight_outputs_on_22_ohm_warn_on_the_pin_and_the_supply() {
    let mut loads = Vec::new();
    for (name, net) in [
        ("ra", "qa"),
        ("rb", "qb"),
        ("rc", "qc"),
        ("rd", "qd"),
        ("re", "qe"),
        ("rf", "qf"),
        ("rg", "qg"),
        ("rh", "qh"),
    ] {
        loads.push(serde_json::json!({
            "net": null,
            "part": {
                "name": name,
                "kind": "resistor",
                "pins": { "a": net, "b": "gnd" },
                "resistance": val(22.0, "ohm"),
                "ratings": { "power": null },
            },
        }));
    }
    let mut clocks = Vec::new();
    for n in 0..8 {
        let t = 1_000_000 + n * 100_000_000;
        clocks.push((t, 5.0));
        clocks.push((t + 20_000_000, 0.0));
    }
    let netlist = register(
        loads,
        steps("srclk", "srclk", &clocks),
        steps("rclk", "rclk", &[(900_000_000, 5.0)]),
    );
    let mut session = open(netlist);
    session.advance_to(Time::from_ps(2_000_000_000));
    let names: Vec<String> = session
        .diagnostics()
        .iter()
        .filter_map(|d| match d {
            Diagnostic::RatingCurrent(excursion) => Some(excursion.quantity.to_string()),
            _ => None,
        })
        .collect();
    assert!(
        names.iter().any(|name| name.contains("QA")),
        "pin warning missing in {names:?}"
    );
    assert!(
        names.iter().any(|name| name.contains("VCC")),
        "supply warning missing in {names:?}"
    );
}
