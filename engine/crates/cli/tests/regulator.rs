//! The LM1117-3.3 rows of TI LM1117, SNOS412Q, solved on the example's part.
//! p5: VOUT 3.267 to 3.333 V at 10 mA and 5 V, line regulation 1 mV typical
//! from 4.75 V to 15 V at 0 mA, load regulation 1 mV typical from 0 to 800 mA
//! at 4.75 V. p6: dropout 1.10, 1.15 and 1.20 V at 100, 500 and 800 mA, by
//! note 4's 100 mV drop from the value at 4.8 V. Current limit 0.8 to 1.5 A
//! at a 5 V differential. Quiescent current 5 mA typical.

use serde_json::{Value, json};
use sim_core::netlist;

fn part() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/regulator-3v3/netlist.json");
    let doc: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    doc["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["kind"] == "linear_regulator")
        .unwrap()
        .clone()
}

fn solve(parts: Vec<Value>) -> (f64, f64, f64) {
    let doc = json!({
        "version": 0,
        "seed": 1,
        "analysis": { "op": {} },
        "ground": "gnd",
        "nets": ["gnd", "vin", "vout"],
        "parts": parts,
    });
    let netlist = netlist::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
    let results = sim_circuit::simulate(&netlist);
    let errors: Vec<_> = results
        .diagnostics()
        .iter()
        .filter(|d| d.severity() == sim_core::diagnostic::Severity::Error)
        .map(|d| d.code().to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let op = results.observations().op().unwrap();
    let read = |reference: &str| op.values[&reference.parse().unwrap()].get();
    (read("#net.vout"), read("#u1.iin"), read("#u1.iout"))
}

fn source(name: &str, net: &str, volts: f64) -> Value {
    json!({
        "name": name,
        "kind": "voltage_source",
        "pins": { "p": net, "n": "gnd" },
        "voltage": { "value": volts, "unit": "V", "source": "user" }
    })
}

fn load(amps: f64) -> Value {
    json!({
        "name": "i1",
        "kind": "current_source",
        "pins": { "p": "gnd", "n": "vout" },
        "current": { "value": amps, "unit": "A", "source": "user" }
    })
}

fn at(vin: f64, amps: f64) -> (f64, f64, f64) {
    solve(vec![source("v1", "vin", vin), load(amps), part()])
}

fn dropout(amps: f64) -> f64 {
    let (reference, _, _) = at(4.8, amps);
    let target = reference - 0.1;
    let mut lo = target;
    let mut hi = 4.8;
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        let (vout, _, _) = at(mid, amps);
        if vout < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let vin = 0.5 * (lo + hi);
    let (vout, _, _) = at(vin, amps);
    vin - vout
}

#[test]
fn the_lm1117_3v3_table_holds() {
    let (vout, _, _) = at(5.0, 0.01);
    assert!(
        (3.267..=3.333).contains(&vout),
        "VOUT {vout} outside 3.267 to 3.333"
    );
    assert!((vout - 3.300153).abs() < 1e-4, "VOUT {vout}");

    let (high, _, _) = at(15.0, 0.0);
    let (low, _, _) = at(4.75, 0.0);
    let line = high - low;
    assert!((line - 0.001).abs() <= 1e-4, "line {line}");

    let (idle, _, _) = at(4.75, 0.0);
    let (full, _, _) = at(4.75, 0.8);
    let load = idle - full;
    assert!((load - 0.001).abs() <= 1e-4, "load {load}");
    let (light, _, _) = at(4.75, 0.1);
    let mid = light - full;
    assert!((mid - 0.000875).abs() <= 1e-4, "load 100 to 800 mA {mid}");

    let points = [(0.1, 1.10), (0.5, 1.15), (0.8, 1.20)];
    for (amps, table) in points {
        let got = dropout(amps);
        assert!((got - table).abs() <= 0.010, "{amps} A dropout {got}");
    }

    let (_, _, iout) = solve(vec![
        source("v1", "vin", 5.0),
        source("v2", "vout", 0.0),
        part(),
    ]);
    let limited = iout.abs();
    assert!((0.8..1.5).contains(&limited), "current limit {limited}");
    assert!((limited - 1.2).abs() < 0.02, "current limit {limited}");

    let (_, iin, iout) = at(5.0, 0.0);
    let ground = iin + iout;
    assert!((ground - 0.005).abs() <= 1e-6, "ground current {ground}");
}
