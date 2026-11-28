use std::fs;

use crate::runs::common;
use common::repo;
use orbe_engine::timestamp::Timestamp;

/// `packages/engine/tests/circuits/npn-led` as a 10 ns transient, with the transistor's current rating at 10 mA. Its
/// collector carries about 13 mA and its base about 4 mA.
fn rated_switch() -> Vec<u8> {
    let mut netlist: serde_json::Value = serde_json::from_slice(
        &fs::read(repo("packages/engine/tests/circuits/npn-led/netlist.json")).unwrap(),
    )
    .unwrap();
    netlist["analysis"] = serde_json::json!({
        "transient": { "stop_ps": 10000, "max_step_ps": 1000, "grid_ps": 1000 }
    });
    let q1 = netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "q1")
        .unwrap();
    q1["ratings"]["current"]["value"] = 0.01.into();
    serde_json::to_vec(&netlist).unwrap()
}

#[test]
fn a_transistor_writes_ib_ic_and_its_pin_power_and_its_current_rating_bounds_ic() {
    let done = orbe_engine::run(&rated_switch(), Timestamp::from_unix_seconds(0));
    let index: serde_json::Value =
        serde_json::from_slice(done.folder.file("traces/index.json").unwrap()).unwrap();
    let columns: Vec<(&str, &str)> = index["analog"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| {
            (
                column["ref"].as_str().unwrap(),
                column["unit"].as_str().unwrap(),
            )
        })
        .filter(|(reference, _)| reference.starts_with("#q1."))
        .collect();
    assert_eq!(columns, [("#q1.ib", "A"), ("#q1.ic", "A"), ("#q1.p", "W")]);

    let trace = done.results.trace().unwrap();
    let column = |key: &str| {
        let k = trace
            .columns()
            .iter()
            .position(|reference| reference.to_string() == key)
            .unwrap_or_else(|| panic!("no column {key}"));
        trace.column(k)
    };
    let (c, b) = (column("#net.c"), column("#net.b"));
    let (ib, ic, p) = (column("#q1.ib"), column("#q1.ic"), column("#q1.p"));
    assert!(ic.iter().all(|&ic| ic > 0.013), "{ic:?}");
    assert!(ib.iter().all(|&ib| ib < 0.005), "{ib:?}");
    for row in 0..ic.len() {
        assert_eq!(p[row], c[row] * ic[row] + b[row] * ib[row], "row {row}");
    }

    let diagnostics = serde_json::to_value(done.results.diagnostics()).unwrap();
    let found: Vec<(&str, &str)> = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["code"].as_str().unwrap(), d["refs"][0].as_str().unwrap()))
        .collect();
    assert_eq!(found, [("rating.current", "#q1.ic")]);
    let peak = diagnostics[0]["data"]["peak"].as_f64().unwrap();
    assert_eq!(peak, ic.iter().copied().fold(0.0, f64::max));
}

/// `packages/engine/tests/circuits/nmos-led` as a 10 ns transient, with the MOSFET's current rating at 10 mA.
/// The drain carries the LED current, above that rating, and the gate current is 0 at DC.
#[test]
fn an_nmos_writes_id_and_its_pin_power_and_its_current_rating_bounds_id() {
    let mut netlist: serde_json::Value = serde_json::from_slice(
        &fs::read(repo("packages/engine/tests/circuits/nmos-led/netlist.json")).unwrap(),
    )
    .unwrap();
    netlist["analysis"] = serde_json::json!({
        "transient": { "stop_ps": 10000, "max_step_ps": 1000, "grid_ps": 1000 }
    });
    let m1 = netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "m1")
        .unwrap();
    m1["ratings"]["current"]["value"] = 0.01.into();
    let done = orbe_engine::run(
        &serde_json::to_vec(&netlist).unwrap(),
        Timestamp::from_unix_seconds(0),
    );
    let index: serde_json::Value =
        serde_json::from_slice(done.folder.file("traces/index.json").unwrap()).unwrap();
    let columns: Vec<(&str, &str)> = index["analog"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| {
            (
                column["ref"].as_str().unwrap(),
                column["unit"].as_str().unwrap(),
            )
        })
        .filter(|(reference, _)| reference.starts_with("#m1."))
        .collect();
    assert_eq!(columns, [("#m1.id", "A"), ("#m1.p", "W")]);

    let trace = done.results.trace().unwrap();
    let column = |key: &str| {
        let k = trace
            .columns()
            .iter()
            .position(|reference| reference.to_string() == key)
            .unwrap_or_else(|| panic!("no column {key}"));
        trace.column(k)
    };
    let (d, id, p) = (column("#net.d"), column("#m1.id"), column("#m1.p"));
    assert!(id.iter().all(|&id| id > 0.01), "{id:?}");
    for row in 0..id.len() {
        assert!(
            (p[row] - d[row] * id[row]).abs() <= 1e-15,
            "row {row}: {} vs {}",
            p[row],
            d[row] * id[row]
        );
    }

    let diagnostics = serde_json::to_value(done.results.diagnostics()).unwrap();
    let found: Vec<(&str, &str)> = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["code"].as_str().unwrap(), d["refs"][0].as_str().unwrap()))
        .collect();
    assert_eq!(found, [("rating.current", "#m1.id")]);
    let peak = diagnostics[0]["data"]["peak"].as_f64().unwrap();
    assert_eq!(peak, id.iter().copied().fold(0.0, f64::max));
}

/// An off MOSFET whose gate is tied to its source so the channel stays cut off, with `v_rev`
/// driving the reverse-biased pin (`source` for `nmos`, `drain` for `pmos`) while the other pin
/// sits on ground.
fn body_diode_netlist(
    kind: &str,
    vf: f64,
    analysis: serde_json::Value,
    v_rev: serde_json::Value,
) -> Vec<u8> {
    let (drain, gate, source) = if kind == "nmos" {
        ("gnd", "rev", "rev")
    } else {
        ("rev", "gnd", "gnd")
    };
    let doc = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": analysis,
        "ground": "gnd",
        "nets": ["gnd", "rev"],
        "parts": [
            {
                "name": "v1",
                "kind": "voltage_source",
                "pins": { "p": "rev", "n": "gnd" },
                "voltage": v_rev
            },
            {
                "name": "m1",
                "kind": kind,
                "pins": { "drain": drain, "gate": gate, "source": source },
                "transconductance_parameter": { "value": 0.3236, "unit": "A/V^2" },
                "threshold_voltage": { "value": 1.3, "unit": "V" },
                "channel_length_modulation": { "value": 0.0, "unit": "1/V" },
                "gate_source_capacitance": { "value": 2.1e-11, "unit": "F" },
                "gate_drain_capacitance": { "value": 6e-12, "unit": "F" },
                "drain_source_capacitance": { "value": 7e-12, "unit": "F" },
                "body_diode_forward_voltage": { "value": vf, "unit": "V" },
                "ratings": { "current": null, "power": null }
            }
        ]
    });
    serde_json::to_vec(&doc).unwrap()
}

#[test]
fn reverse_biased_off_nmos_and_pmos_warn_body_diode_with_part_and_time() {
    let tran = serde_json::json!({
        "transient": {
            "stop_ps": 5_000_000_000u64,
            "max_step_ps": 100_000_000u64,
            "grid_ps": 100_000_000u64
        }
    });
    let step_at: u64 = 1_000_000_000;
    let step_src = serde_json::json!({
        "kind": "step",
        "initial": { "value": 0.0, "unit": "V" },
        "final": { "value": 3.3, "unit": "V" },
        "at_ps": step_at
    });

    for (kind, vf, direction) in [
        ("nmos", 0.8, "conduct from source to drain"),
        ("pmos", 0.7, "conduct from drain to source"),
    ] {
        // Operating point warns at t = 0 with null over_ps.
        let op = orbe_engine::run(
            &body_diode_netlist(
                kind,
                vf,
                serde_json::json!({ "op": {} }),
                serde_json::json!({ "value": 3.3, "unit": "V" }),
            ),
            Timestamp::from_unix_seconds(0),
        );
        let op_diags = serde_json::to_value(op.results.diagnostics()).unwrap();
        let arr = op_diags.as_array().unwrap();
        assert_eq!(arr.len(), 1, "{kind} OP: {arr:?}");
        assert_eq!(arr[0]["code"], "mosfet.body_diode");
        assert_eq!(arr[0]["severity"], "warning");
        assert_eq!(arr[0]["refs"], serde_json::json!(["#m1"]));
        assert_eq!(arr[0]["t_ps"], 0);
        assert_eq!(arr[0]["data"]["channel"], kind);
        assert_eq!(arr[0]["data"]["forward_voltage"], vf);
        assert_eq!(arr[0]["data"]["over_ps"], serde_json::Value::Null);
        assert!((arr[0]["data"]["peak"].as_f64().unwrap() - 3.3).abs() < 1e-6);
        let msg = arr[0]["message"].as_str().unwrap();
        assert!(msg.contains("body diode is not simulated"), "{msg}");
        assert!(msg.contains(direction), "{msg}");

        // Transient: quiet until the step at 1 ms, then warns with t_ps at the edge.
        let run = orbe_engine::run(
            &body_diode_netlist(kind, vf, tran.clone(), step_src.clone()),
            Timestamp::from_unix_seconds(0),
        );
        let diags = serde_json::to_value(run.results.diagnostics()).unwrap();
        let arr = diags.as_array().unwrap();
        assert_eq!(arr.len(), 1, "{kind} transient: {arr:?}");
        assert_eq!(arr[0]["code"], "mosfet.body_diode");
        assert_eq!(arr[0]["refs"], serde_json::json!(["#m1"]));
        assert_eq!(arr[0]["t_ps"], step_at);
        assert_eq!(arr[0]["data"]["channel"], kind);
        assert_eq!(arr[0]["data"]["over_ps"], 4_000_000_000u64);
    }
}

#[test]
fn brief_reverse_spike_unpowered_and_below_knee_mosfets_stay_quiet() {
    let tran = serde_json::json!({
        "transient": {
            "stop_ps": 10_000_000u64,
            "max_step_ps": 500_000u64,
            "grid_ps": 500_000u64
        }
    });
    // 500 ns reverse pulse (under the 1 us hold time) stays quiet on both NMOS and PMOS.
    let brief_pulse = serde_json::json!({
        "kind": "pulse",
        "initial": { "value": 0.0, "unit": "V" },
        "pulsed": { "value": 5.0, "unit": "V" },
        "delay_ps": 1_000_000u64,
        "width_ps": 500_000u64,
        "period_ps": 20_000_000u64
    });
    for (kind, vf) in [("nmos", 0.8), ("pmos", 0.7)] {
        let spike = orbe_engine::run(
            &body_diode_netlist(kind, vf, tran.clone(), brief_pulse.clone()),
            Timestamp::from_unix_seconds(0),
        );
        assert_eq!(spike.results.diagnostics(), [], "{kind} brief spike");

        // Unpowered (0 V) and small reverse bias (0.4 V < Vf) stay quiet at OP and in transient.
        for volts in [0.0, 0.4, -5.0] {
            let op = orbe_engine::run(
                &body_diode_netlist(
                    kind,
                    vf,
                    serde_json::json!({ "op": {} }),
                    serde_json::json!({ "value": volts, "unit": "V" }),
                ),
                Timestamp::from_unix_seconds(0),
            );
            assert_eq!(op.results.diagnostics(), [], "{kind} OP at {volts} V");
            let run = orbe_engine::run(
                &body_diode_netlist(
                    kind,
                    vf,
                    tran.clone(),
                    serde_json::json!({ "value": volts, "unit": "V" }),
                ),
                Timestamp::from_unix_seconds(0),
            );
            assert_eq!(
                run.results.diagnostics(),
                [],
                "{kind} transient at {volts} V"
            );
        }
    }
}
