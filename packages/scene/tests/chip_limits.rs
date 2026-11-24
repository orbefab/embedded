//! The catalog's regulators and op-amps warn on a supply over their sheet's absolute maximum,
//! and the 2N2222A's gain falls at high current as its sheet's typical curve does.

use std::fs;
use std::path::Path;

use orbe_model::Time;

use orbe_engine::Session;
use orbe_engine::blob::Blobs;
use orbe_engine::name::Ref;
use orbe_scene::Files;

mod common;

const MS: u64 = 1_000_000_000;

const SUPPLY: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "params": { "voltage": { "unit": "V", "required": true, "sets": "/voltage" } },
  "model": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } }
}
"#;

const BASE: &str = r#"{
  "format": 1,
  "pins": { "out": [1], "back": [2] },
  "params": { "current": { "unit": "A", "required": true, "sets": "/current" } },
  "model": { "kind": "current_source", "pins": { "p": "out", "n": "back" } }
}
"#;

const PULSE: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "model": {
    "kind": "voltage_source",
    "pins": { "p": "pos", "n": "neg" },
    "voltage": {
      "kind": "pulse",
      "initial": { "value": 0.0, "unit": "V", "source": "user" },
      "pulsed": { "value": 10.0, "unit": "V", "source": "user" },
      "delay_ps": 1000000000,
      "width_ps": 100000000,
      "period_ps": 1000000000
    }
  }
}
"#;

const SWITCH: &str = r#"{
  "format": 1,
  "pins": { "a": [1], "b": [2] },
  "model": {
    "kind": "switch",
    "pins": { "a": "a", "b": "b" },
    "on_resistance": { "value": 0.1, "unit": "ohm", "source": "user" },
    "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
    "initial": "open",
    "events": [{ "t_ps": 10000000000, "state": "closed" }],
    "bounce": null
  }
}
"#;

/// A session on `parts` and `wires`, with the local supply, base, pulse and switch parts on hand.
fn open(dir: &Path, parts: &str, wires: &str) -> Session {
    for (name, text) in [
        ("supply", SUPPLY),
        ("base", BASE),
        ("pulse", PULSE),
        ("switch", SWITCH),
    ] {
        fs::create_dir_all(dir.join(name)).unwrap();
        fs::write(dir.join(name).join("part.jsonc"), text).unwrap();
    }
    let scene = format!(
        r#"{{ "format": 1, "parts": {{ {parts} }}, "wires": [{wires}], "ground": "v1.neg" }}"#
    );
    fs::write(dir.join("scene.jsonc"), scene).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc"))
        .unwrap_or_else(|found| panic!("{found:#?}"));
    Session::open_json(netlist.json.as_bytes(), &Blobs::new()).unwrap()
}

/// The `rating.voltage` messages after 1 ms of `id` on a supply of `volts`, with `extra` wires.
fn over_voltage(id: &str, vcc: &str, gnd: &str, extra: &str, volts: f64) -> Vec<String> {
    let project = common::project("nano-button");
    let parts = format!(
        r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "{volts}" }}, "u1": {{ "type": "{id}" }}"#
    );
    let wires = format!(r#"["v1.pos", "u1.{vcc}"], ["v1.neg", "u1.{gnd}"]{extra}"#);
    let mut session = open(project.path(), &parts, &wires);
    session.advance_to(Time::from_ps(MS));
    session
        .diagnostics()
        .iter()
        .filter(|d| d.code() == "rating.voltage")
        .map(|d| serde_json::to_value(d).unwrap()["message"].to_string())
        .collect()
}

#[test]
fn a_supply_over_the_sheet_s_maximum_warns_and_one_inside_it_does_not() {
    // Each op-amp amplifier follows ground.
    let followers = r#", ["u1.OUT1", "u1.IN1_N"], ["u1.IN1_P", "v1.neg"], ["u1.OUT2", "u1.IN2_N"], ["u1.IN2_P", "v1.neg"]"#;
    for (id, vcc, gnd, extra, inside, over) in [
        ("lm7805-to220", "IN", "GND", "", 12.0, 40.0),
        ("ams1117-sot223", "IN", "GND", "", 12.0, 16.0),
        ("sgm2212-sot223", "IN", "GND", "", 12.0, 23.0),
        ("lp2985-sot23-5", "IN", "GND", "", 12.0, 17.0),
        ("lm358-dip8", "VCC", "GND", followers, 30.0, 36.0),
        ("lmv358-msop8", "VCC", "GND", followers, 5.0, 6.0),
    ] {
        assert_eq!(
            over_voltage(id, vcc, gnd, extra, inside),
            Vec::<String>::new(),
            "{id} at {inside} V"
        );
        let found = over_voltage(id, vcc, gnd, extra, over);
        assert_eq!(found.len(), 1, "{id} at {over} V: {found:?}");
    }
}

/// The collector current and the gain with `ib` into the base and 10 V on the collector.
fn gain(ib: f64) -> (f64, f64) {
    let project = common::project("nano-button");
    let parts = format!(
        r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "10" }},
        "i1": {{ "type": "./base/part.jsonc", "current": "{ib}" }},
        "q1": {{ "type": "npn-2n2222-to92" }}"#
    );
    let wires = r#"["v1.pos", "q1.collector"], ["i1.out", "q1.base"], ["v1.neg", "q1.emitter"], ["v1.neg", "i1.back"]"#;
    let mut session = open(project.path(), &parts, wires);
    session.advance_to(Time::from_ps(MS));
    let probe = session.probe();
    let read = |r: &str| probe.get(&r.parse::<Ref>().unwrap()).unwrap().get();
    let (ic, ib) = (read("#q1.ic"), read("#q1.ib"));
    (ic, ic / ib)
}

/// onsemi P2N2222A/D Figure 3 at VCE = 10 V: typical hFE about 245 at 150 mA and 165 at
/// 500 mA. The model's gain falls by that ratio between the two, and stays over the sheet's
/// 40 minimum at 500 mA.
#[test]
fn the_2n2222a_gain_falls_from_150_ma_to_500_ma_as_its_sheet_s_curve_does() {
    let (ic_150, beta_150) = gain(0.15 / 192.0);
    let (ic_500, beta_500) = gain(0.5 / 132.0);
    assert!((ic_150 / 0.15 - 1.0).abs() < 0.05, "{ic_150} A");
    assert!((ic_500 / 0.5 - 1.0).abs() < 0.05, "{ic_500} A");
    let ratio = beta_500 / beta_150;
    assert!(
        (ratio / (165.0 / 245.0) - 1.0).abs() < 0.05,
        "beta {beta_150} at {ic_150} A, {beta_500} at {ic_500} A"
    );
    assert!(beta_500 > 40.0, "{beta_500}");
}

#[test]
fn ams1117_warns_regulator_current_limit_into_a_short_and_stays_quiet_under_limit() {
    let run_load = |ohms: &str| {
        let project = common::project("nano-button");
        let parts = format!(
            r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "9" }},
            "u1": {{ "type": "ams1117-sot223" }},
            "rl": {{ "type": "resistor-axial", "resistance": "{ohms}" }}"#
        );
        let wires =
            r#"["v1.pos", "u1.IN"], ["v1.neg", "u1.GND"], ["u1.OUT", "rl.a"], ["rl.b", "v1.neg"]"#;
        let mut session = open(project.path(), &parts, wires);
        session.advance_to(Time::from_ps(5 * MS));
        session
            .diagnostics()
            .into_iter()
            .filter(|d| d.code() == "regulator.current_limit")
            .map(|d| serde_json::to_value(&d).unwrap())
            .collect::<Vec<_>>()
    };

    // 4.75 ohm at 5.0 V draws ~1.05 A, just under AMS1117's 1.1 A current_limit.
    let under = run_load("4.75");
    assert!(under.is_empty(), "under limit warned: {under:?}");

    // 0.5 ohm shorts the AMS1117 into its 1.1 A limit and pulls OUT down to ~0.55 V.
    let shorted = run_load("0.5");
    assert_eq!(shorted.len(), 1, "{shorted:?}");
    let message = shorted[0]["message"].as_str().unwrap();
    assert!(
        message.contains("#u1.pin.out regulator output can give 1.1 A at 5 V"),
        "{message}"
    );
    assert!(message.contains("dropped to"), "{message}");
    assert!(shorted[0]["data"]["voltage"].as_f64().unwrap() < 1.0);
}

fn limit_warnings(session: &Session) -> Vec<serde_json::Value> {
    session
        .diagnostics()
        .into_iter()
        .filter(|d| d.code() == "regulator.current_limit")
        .map(|d| serde_json::to_value(&d).unwrap())
        .collect()
}

#[test]
fn every_regulator_warns_at_105_percent_load_and_stays_quiet_at_90_percent() {
    let cases = [
        ("ams1117-sot223", "IN", "OUT", "GND", 9.0, 5.0, 1.1),
        ("lm7805-to220", "IN", "OUT", "GND", 9.0, 5.0, 2.4),
        ("sgm2212-sot223", "IN", "OUT", "GND", 5.0, 3.3, 1.1),
        ("lp2985-sot23-5", "IN", "OUT", "GND", 5.0, 3.3, 0.35),
        ("xc6206-sot23", "VIN", "VOUT", "VSS", 5.0, 3.3, 0.2),
        ("ch340g", "VCC", "V3", "GND", 5.0, 3.3, 0.03),
    ];
    for (id, vin, vout, gnd, supply, target, ilim) in cases {
        for (pct, expect_warn) in [(0.90, false), (1.05, true)] {
            let ohms = target / (pct * ilim);
            let project = common::project("nano-button");
            let parts = format!(
                r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "{supply}" }},
                "u1": {{ "type": "{id}" }},
                "rl": {{ "type": "resistor-axial", "resistance": "{ohms}" }}"#
            );
            let wires = format!(
                r#"["v1.pos", "u1.{vin}"], ["v1.neg", "u1.{gnd}"], ["u1.{vout}", "rl.a"], ["rl.b", "v1.neg"]"#
            );
            let mut session = open(project.path(), &parts, &wires);
            session.advance_to(Time::from_ps(10 * MS));
            let probe = session.probe();
            let v_out = probe.get(&"#rl.v".parse::<Ref>().unwrap()).unwrap().get();
            let found = limit_warnings(&session);
            if expect_warn {
                assert_eq!(
                    found.len(),
                    1,
                    "{id} at {:.0}% (R={ohms:.3}, vout={v_out:.4} V): {found:?}",
                    pct * 100.0
                );
                let reported_v = found[0]["data"]["voltage"].as_f64().unwrap();
                assert!(
                    (reported_v - v_out).abs() < 0.01,
                    "{id} reported {reported_v} V vs probe {v_out} V ({})",
                    found[0]["message"]
                );
            } else {
                assert!(
                    found.is_empty(),
                    "{id} at {:.0}% warned: {found:?}",
                    pct * 100.0
                );
            }
        }
    }
}

#[test]
fn large_electrolytic_power_on_charge_stays_quiet_while_overload_warns() {
    for (id, cap) in [
        ("ams1117-sot223", "470u"),
        ("ams1117-sot223", "1000u"),
        ("lm7805-to220", "470u"),
        ("lm7805-to220", "1000u"),
    ] {
        let project = common::project("nano-button");
        let parts = format!(
            r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "9" }},
            "u1": {{ "type": "{id}" }},
            "c1": {{ "type": "capacitor-electrolytic-radial", "capacitance": "{cap}", "initial": "0" }},
            "rl": {{ "type": "resistor-axial", "resistance": "1000" }}"#
        );
        let wires = r#"["v1.pos", "u1.IN"], ["v1.neg", "u1.GND"], ["u1.OUT", "c1.pos"], ["c1.neg", "v1.neg"], ["u1.OUT", "rl.a"], ["rl.b", "v1.neg"]"#;
        let mut session = open(project.path(), &parts, wires);
        session.advance_to(Time::from_ps(15 * MS));
        let probe = session.probe();
        let v_out = probe.get(&"#rl.v".parse::<Ref>().unwrap()).unwrap().get();
        assert!(v_out > 4.95, "{id} {cap} did not reach 5 V: {v_out} V");
        let found = limit_warnings(&session);
        assert!(found.is_empty(), "{id} {cap} power-on warned: {found:?}");
    }

    // A 1000 uF capacitor with a 0.5 ohm short stops rising at ~0.55 V and must warn once.
    let project = common::project("nano-button");
    let parts = r#""v1": { "type": "./supply/part.jsonc", "voltage": "9" },
        "u1": { "type": "ams1117-sot223" },
        "c1": { "type": "capacitor-electrolytic-radial", "capacitance": "1000u", "initial": "0" },
        "rl": { "type": "resistor-axial", "resistance": "0.5" }"#;
    let wires = r#"["v1.pos", "u1.IN"], ["v1.neg", "u1.GND"], ["u1.OUT", "c1.pos"], ["c1.neg", "v1.neg"], ["u1.OUT", "rl.a"], ["rl.b", "v1.neg"]"#;
    let mut session = open(project.path(), parts, wires);
    session.advance_to(Time::from_ps(15 * MS));
    let found = limit_warnings(&session);
    assert_eq!(found.len(), 1, "1000uF + short did not warn: {found:?}");
    let reported_v = found[0]["data"]["voltage"].as_f64().unwrap();
    let probe_v = session
        .probe()
        .get(&"#rl.v".parse::<Ref>().unwrap())
        .unwrap()
        .get();
    assert!(
        (reported_v - probe_v).abs() < 0.02,
        "1000uF + short reported {reported_v} V vs probe {probe_v} V ({})",
        found[0]["message"]
    );

    // Mid-run short after a 1000 uF rail has charged: quiet until the switch closes at 10 ms,
    // then warns once with the settled shorted voltage.
    let project = common::project("nano-button");
    let parts = r#""v1": { "type": "./supply/part.jsonc", "voltage": "9" },
        "u1": { "type": "ams1117-sot223" },
        "c1": { "type": "capacitor-electrolytic-radial", "capacitance": "1000u", "initial": "0" },
        "rb": { "type": "resistor-axial", "resistance": "1000" },
        "sw": { "type": "./switch/part.jsonc" },
        "rl": { "type": "resistor-axial", "resistance": "0.5" }"#;
    let wires = r#"["v1.pos", "u1.IN"], ["v1.neg", "u1.GND"],
        ["u1.OUT", "c1.pos"], ["c1.neg", "v1.neg"],
        ["u1.OUT", "rb.a"], ["rb.b", "v1.neg"],
        ["u1.OUT", "sw.a"], ["sw.b", "rl.a"], ["rl.b", "v1.neg"]"#;
    let mut session = open(project.path(), parts, wires);
    session.advance_to(Time::from_ps(9 * MS));
    assert!(
        limit_warnings(&session).is_empty(),
        "warned before mid-run short: {:?}",
        limit_warnings(&session)
    );
    session.advance_to(Time::from_ps(15 * MS));
    let found = limit_warnings(&session);
    assert_eq!(found.len(), 1, "mid-run short did not warn: {found:?}");
    let reported_v = found[0]["data"]["voltage"].as_f64().unwrap();
    let probe_v = session
        .probe()
        .get(&"#rb.v".parse::<Ref>().unwrap())
        .unwrap()
        .get();
    assert!(
        (reported_v - probe_v).abs() < 0.02,
        "mid-run short reported {reported_v} V vs probe {probe_v} V ({})",
        found[0]["message"]
    );
}

#[test]
fn pwm_mosfet_overload_reports_settled_pulse_voltage_matching_probe() {
    let cases = [
        ("ams1117-sot223", "IN", "OUT", "GND", 9.0, 5.0, 1.1),
        ("lp2985-sot23-5", "IN", "OUT", "GND", 5.0, 3.3, 0.35),
        ("xc6206-sot23", "VIN", "VOUT", "VSS", 5.0, 3.3, 0.2),
    ];
    for (id, vin, vout, gnd, supply, target, ilim) in cases {
        let bleed = target / 0.01;
        let load = target / (1.5 * ilim);
        let project = common::project("nano-button");
        let parts = format!(
            r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "{supply}" }},
            "vg": {{ "type": "./pulse/part.jsonc" }},
            "u1": {{ "type": "{id}" }},
            "rb": {{ "type": "resistor-axial", "resistance": "{bleed}" }},
            "rl": {{ "type": "resistor-axial", "resistance": "{load}" }},
            "q1": {{ "type": "nmos-irf520-to220" }}"#
        );
        let wires = format!(
            r#"["v1.pos", "u1.{vin}"], ["v1.neg", "u1.{gnd}"],
            ["u1.{vout}", "rb.a"], ["rb.b", "v1.neg"],
            ["u1.{vout}", "rl.a"], ["rl.b", "q1.drain"], ["q1.source", "v1.neg"],
            ["vg.pos", "q1.gate"], ["vg.neg", "v1.neg"]"#
        );
        let mut session = open(project.path(), &parts, &wires);
        // Middle of the first 100 us pulse (starts at 1 ms).
        session.advance_to(Time::from_ps(1_050_000_000));
        let in_pulse_v = session
            .probe()
            .get(&"#rb.v".parse::<Ref>().unwrap())
            .unwrap()
            .get();
        // Run across several PWM cycles including turn-off edges.
        session.advance_to(Time::from_ps(5 * MS));
        let found = limit_warnings(&session);
        assert_eq!(found.len(), 1, "{id} PWM overload: {found:?}");
        let reported_v = found[0]["data"]["voltage"].as_f64().unwrap();
        assert!(
            (reported_v - in_pulse_v).abs() < 0.02,
            "{id} PWM reported {reported_v} V vs in-pulse {in_pulse_v} V ({})",
            found[0]["message"]
        );
    }
}

#[test]
fn charging_large_capacitor_with_heavy_load_stays_quiet_while_overload_warns() {
    let quiet_cases = [
        (
            "ch340g",
            "VCC",
            "V3",
            "GND",
            5.0,
            "122.2",
            "4700u",
            Some(800 * MS),
            3_000 * MS,
            3.25,
        ),
        (
            "ch340g",
            "VCC",
            "V3",
            "GND",
            5.0,
            "110",
            "1000u",
            None,
            1_000 * MS,
            3.25,
        ),
        (
            "ch340g",
            "VCC",
            "V3",
            "GND",
            5.0,
            "110",
            "4700u",
            None,
            3_000 * MS,
            3.25,
        ),
        (
            "ams1117-sot223",
            "IN",
            "OUT",
            "GND",
            9.0,
            "4.545",
            "4700u",
            Some(100 * MS),
            300 * MS,
            4.95,
        ),
        (
            "sgm2212-sot223",
            "IN",
            "OUT",
            "GND",
            5.0,
            "3.000",
            "4700u",
            None,
            200 * MS,
            3.25,
        ),
        (
            "xc6206-sot23",
            "VIN",
            "VOUT",
            "VSS",
            5.0,
            "16.5",
            "4700u",
            None,
            500 * MS,
            3.15,
        ),
    ];
    let mut failures = Vec::new();
    for (id, vin, vout, gnd, supply, ohms, cap, mid_ps, stop_ps, min_v) in quiet_cases {
        let project = common::project("nano-button");
        let parts = format!(
            r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "{supply}" }},
            "u1": {{ "type": "{id}" }},
            "c1": {{ "type": "capacitor-electrolytic-radial", "capacitance": "{cap}", "initial": "0" }},
            "rl": {{ "type": "resistor-axial", "resistance": "{ohms}" }}"#
        );
        let wires = format!(
            r#"["v1.pos", "u1.{vin}"], ["v1.neg", "u1.{gnd}"],
            ["u1.{vout}", "c1.pos"], ["c1.neg", "v1.neg"],
            ["u1.{vout}", "rl.a"], ["rl.b", "v1.neg"]"#
        );
        let mut session = open(project.path(), &parts, &wires);
        if let Some(mid) = mid_ps {
            session.advance_to(Time::from_ps(mid));
            let mid_found = limit_warnings(&session);
            if !mid_found.is_empty() {
                failures.push(format!(
                    "{id} {ohms} ohm {cap} warned mid-charge at {mid} ps: {}",
                    mid_found[0]["message"]
                ));
            }
        }
        session.advance_to(Time::from_ps(stop_ps));
        let probe = session.probe();
        let v_out = probe.get(&"#rl.v".parse::<Ref>().unwrap()).unwrap().get();
        let i_out = probe
            .get(&"#rl.i".parse::<Ref>().unwrap())
            .unwrap()
            .get()
            .abs();
        let found = limit_warnings(&session);
        if v_out <= min_v {
            failures.push(format!(
                "{id} {ohms} ohm {cap} did not reach regulation: vout={v_out:.4} V i={i_out:.4} A"
            ));
        }
        if !found.is_empty() {
            failures.push(format!(
                "{id} {ohms} ohm {cap} (vout={v_out:.4} V i={i_out:.4} A) warned: {}",
                found[0]["message"]
            ));
        }
    }

    let overload_cases = [
        (
            "ams1117-sot223",
            "IN",
            "OUT",
            "GND",
            9.0,
            "4.329",
            "4700u",
            300 * MS,
        ),
        (
            "ch340g",
            "VCC",
            "V3",
            "GND",
            5.0,
            "104.76",
            "4700u",
            3_000 * MS,
        ),
    ];
    for (id, vin, vout, gnd, supply, ohms, cap, stop_ps) in overload_cases {
        let project = common::project("nano-button");
        let parts = format!(
            r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "{supply}" }},
            "u1": {{ "type": "{id}" }},
            "c1": {{ "type": "capacitor-electrolytic-radial", "capacitance": "{cap}", "initial": "0" }},
            "rl": {{ "type": "resistor-axial", "resistance": "{ohms}" }}"#
        );
        let wires = format!(
            r#"["v1.pos", "u1.{vin}"], ["v1.neg", "u1.{gnd}"],
            ["u1.{vout}", "c1.pos"], ["c1.neg", "v1.neg"],
            ["u1.{vout}", "rl.a"], ["rl.b", "v1.neg"]"#
        );
        let mut session = open(project.path(), &parts, &wires);
        session.advance_to(Time::from_ps(stop_ps));
        let probe_v = session
            .probe()
            .get(&"#rl.v".parse::<Ref>().unwrap())
            .unwrap()
            .get();
        let found = limit_warnings(&session);
        if found.len() != 1 {
            failures.push(format!(
                "{id} {ohms} ohm {cap} overload did not warn (vout={probe_v:.4} V): {found:?}"
            ));
            continue;
        }
        let reported_v = found[0]["data"]["voltage"].as_f64().unwrap();
        if (reported_v - probe_v).abs() >= 0.05 {
            failures.push(format!(
                "{id} {ohms} ohm {cap} reported {reported_v} V vs probe {probe_v} V ({})",
                found[0]["message"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn pwm_mosfet_under_limit_stays_quiet_and_105_percent_warns_with_pulse_voltage() {
    let cases = [
        ("ams1117-sot223", "IN", "OUT", "GND", 9.0, 5.0, 1.1),
        ("lm7805-to220", "IN", "OUT", "GND", 9.0, 5.0, 2.4),
        ("sgm2212-sot223", "IN", "OUT", "GND", 5.0, 3.3, 1.1),
        ("lp2985-sot23-5", "IN", "OUT", "GND", 5.0, 3.3, 0.35),
        ("xc6206-sot23", "VIN", "VOUT", "VSS", 5.0, 3.3, 0.2),
        ("ch340g", "VCC", "V3", "GND", 5.0, 3.3, 0.03),
    ];
    let mut failures = Vec::new();
    for (id, vin, vout, gnd, supply, target, ilim) in cases {
        let bleed = target / 0.005;
        let load_80 = target / (0.80 * ilim);
        let project = common::project("nano-button");
        let parts = format!(
            r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "{supply}" }},
            "vg": {{ "type": "./pulse/part.jsonc" }},
            "u1": {{ "type": "{id}" }},
            "rl": {{ "type": "resistor-axial", "resistance": "{load_80}" }},
            "q1": {{ "type": "nmos-irf520-to220" }}"#
        );
        let wires = format!(
            r#"["v1.pos", "u1.{vin}"], ["v1.neg", "u1.{gnd}"],
            ["u1.{vout}", "rl.a"], ["rl.b", "q1.drain"], ["q1.source", "v1.neg"],
            ["vg.pos", "q1.gate"], ["vg.neg", "v1.neg"]"#
        );
        let mut session = open(project.path(), &parts, &wires);
        session.advance_to(Time::from_ps(5 * MS));
        let found = limit_warnings(&session);
        if !found.is_empty() {
            failures.push(format!(
                "{id} 80% PWM (R={load_80:.3}) warned: {}",
                found[0]["message"]
            ));
        }

        // 105% PWM overload (subtracting IRF520's ~0.12 ohm Rds(on) at 10 V gate).
        let load_105 = target / (1.05 * ilim) - 0.12;
        let project = common::project("nano-button");
        let parts = format!(
            r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "{supply}" }},
            "vg": {{ "type": "./pulse/part.jsonc" }},
            "u1": {{ "type": "{id}" }},
            "rb": {{ "type": "resistor-axial", "resistance": "{bleed}" }},
            "rl": {{ "type": "resistor-axial", "resistance": "{load_105}" }},
            "q1": {{ "type": "nmos-irf520-to220" }}"#
        );
        let wires = format!(
            r#"["v1.pos", "u1.{vin}"], ["v1.neg", "u1.{gnd}"],
            ["u1.{vout}", "rb.a"], ["rb.b", "v1.neg"],
            ["u1.{vout}", "rl.a"], ["rl.b", "q1.drain"], ["q1.source", "v1.neg"],
            ["vg.pos", "q1.gate"], ["vg.neg", "v1.neg"]"#
        );
        let mut session = open(project.path(), &parts, &wires);
        session.advance_to(Time::from_ps(1_050_000_000));
        let in_pulse_v = session
            .probe()
            .get(&"#rb.v".parse::<Ref>().unwrap())
            .unwrap()
            .get();
        session.advance_to(Time::from_ps(5 * MS));
        let found = limit_warnings(&session);
        if found.len() != 1 {
            failures.push(format!("{id} 105% PWM overload: {found:?}"));
            continue;
        }
        let reported_v = found[0]["data"]["voltage"].as_f64().unwrap();
        if (reported_v - in_pulse_v).abs() >= 0.02 {
            failures.push(format!(
                "{id} 105% PWM reported {reported_v} V vs in-pulse {in_pulse_v} V ({})",
                found[0]["message"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn mid_charge_overload_step_reports_settled_voltage_not_rising_charge_dip() {
    let cases = [
        (
            "ams1117-sot223",
            "IN",
            "OUT",
            "GND",
            9.0,
            5.0,
            1.1,
            "1000u",
            2 * MS,
            40 * MS,
        ),
        (
            "lp2985-sot23-5",
            "IN",
            "OUT",
            "GND",
            5.0,
            3.3,
            0.35,
            "1000u",
            3_800_000_000,
            80 * MS,
        ),
        (
            "ch340g",
            "VCC",
            "V3",
            "GND",
            5.0,
            3.3,
            0.03,
            "100u",
            4_400_000_000,
            100 * MS,
        ),
    ];
    let mut failures = Vec::new();
    for (id, vin, vout, gnd, supply, target, ilim, cap, step_ps, stop_ps) in cases {
        let light = target / (0.20 * ilim);
        let overload = target / (1.10 * ilim);
        let project = common::project("nano-button");
        fs::create_dir_all(project.path().join("step_sw")).unwrap();
        fs::write(
            project.path().join("step_sw/part.jsonc"),
            format!(
                r#"{{
  "format": 1,
  "pins": {{ "a": [1], "b": [2] }},
  "model": {{
    "kind": "switch",
    "pins": {{ "a": "a", "b": "b" }},
    "on_resistance": {{ "value": 0.001, "unit": "ohm", "source": "user" }},
    "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
    "initial": "open",
    "events": [{{ "t_ps": {step_ps}, "state": "closed" }}],
    "bounce": null
  }}
}}
"#
            ),
        )
        .unwrap();
        let parts = format!(
            r#""v1": {{ "type": "./supply/part.jsonc", "voltage": "{supply}" }},
            "u1": {{ "type": "{id}" }},
            "c1": {{ "type": "capacitor-electrolytic-radial", "capacitance": "{cap}", "initial": "0" }},
            "rb": {{ "type": "resistor-axial", "resistance": "{light}" }},
            "sw": {{ "type": "./step_sw/part.jsonc" }},
            "rl": {{ "type": "resistor-axial", "resistance": "{overload}" }}"#
        );
        let wires = format!(
            r#"["v1.pos", "u1.{vin}"], ["v1.neg", "u1.{gnd}"],
            ["u1.{vout}", "c1.pos"], ["c1.neg", "v1.neg"],
            ["u1.{vout}", "rb.a"], ["rb.b", "v1.neg"],
            ["u1.{vout}", "sw.a"], ["sw.b", "rl.a"], ["rl.b", "v1.neg"]"#
        );
        let mut session = open(project.path(), &parts, &wires);
        session.advance_to(Time::from_ps(step_ps + 200_000_000));
        let early = limit_warnings(&session);
        if !early.is_empty() {
            failures.push(format!(
                "{id} {cap} warned 200 us after step while still charging: {} (voltage={})",
                early[0]["message"], early[0]["data"]["voltage"]
            ));
        }
        session.advance_to(Time::from_ps(stop_ps));
        let probe_v = session
            .probe()
            .get(&"#rb.v".parse::<Ref>().unwrap())
            .unwrap()
            .get();
        let found = limit_warnings(&session);
        if found.len() != 1 {
            failures.push(format!(
                "{id} {cap} expected 1 warning at settled overload ({probe_v:.4} V), got {found:?}"
            ));
            continue;
        }
        let reported_v = found[0]["data"]["voltage"].as_f64().unwrap();
        if (reported_v - probe_v).abs() >= 0.02 {
            failures.push(format!(
                "{id} {cap} reported {reported_v:.4} V vs settled {probe_v:.4} V ({})",
                found[0]["message"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
