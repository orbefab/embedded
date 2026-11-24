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

/// A session on `parts` and `wires`, with the local supply and base parts on hand.
fn open(dir: &Path, parts: &str, wires: &str) -> Session {
    for (name, text) in [("supply", SUPPLY), ("base", BASE)] {
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
