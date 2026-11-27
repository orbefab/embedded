//! The catalog's DHT22 and BMP280 keep reading in a world past their sheet's operating
//! temperature, and the run warns `rating.temperature` once, naming the part, the temperature
//! and the sheet's range. A world inside the range, its ends included, stays quiet. Both
//! sheets' minimum is -40 C, the world's own floor, so only the maximum can be passed here.

use std::fs;
use std::path::Path;

use orbe_model::Time;

use orbe_engine::Session;
use orbe_engine::blob::Blobs;
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

/// A session on `id` powered from `volts` on `vcc` and `gnd`, with `wires` more, in a world
/// whose temperature is `world`.
fn open(dir: &Path, id: &str, volts: f64, wires: &str, world: &str) -> Session {
    fs::create_dir_all(dir.join("supply")).unwrap();
    fs::write(dir.join("supply").join("part.jsonc"), SUPPLY).unwrap();
    let scene = format!(
        r#"{{ "format": 1,
          "parts": {{ "v1": {{ "type": "./supply/part.jsonc", "voltage": "{volts}" }}, "u1": {{ "type": "{id}" }} }},
          "wires": [{wires}], "ground": "v1.neg", "world": {{ "temperature": {world} }} }}"#
    );
    fs::write(dir.join("scene.jsonc"), scene).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc"))
        .unwrap_or_else(|found| panic!("{found:#?}"));
    Session::open_json(netlist.json.as_bytes(), &Blobs::new()).unwrap()
}

/// Each `rating.temperature` message after `ms` of the part in `world`, up to its time
/// outside, which runs through the last accepted step, and every warning.
fn run(id: &str, world: &str, ms: u64) -> (Vec<String>, Vec<serde_json::Value>) {
    let project = common::project("nano-button");
    let (volts, wires) = match id {
        "dht22" => (5.0, r#"["v1.pos", "u1.VDD"], ["v1.neg", "u1.GND"]"#),
        // CSB high selects I2C. SDI and SCK idle high, SDO low is address 0x76.
        "bmp280" => (
            3.3,
            r#"["v1.pos", "u1.VDD"], ["v1.pos", "u1.VDDIO"], ["v1.neg", "u1.GND"],
               ["v1.pos", "u1.CSB"], ["v1.pos", "u1.SDI"], ["v1.pos", "u1.SCK"], ["v1.neg", "u1.SDO"]"#,
        ),
        _ => unreachable!(),
    };
    let mut session = open(project.path(), id, volts, wires, world);
    session.advance_to(Time::from_ps(ms * MS));
    let all: Vec<serde_json::Value> = session
        .diagnostics()
        .iter()
        .map(|d| serde_json::to_value(d).unwrap())
        .collect();
    let messages = all
        .iter()
        .filter(|d| d["code"] == "rating.temperature")
        .map(|d| {
            let message = d["message"].as_str().unwrap();
            assert!(message.ends_with(" ps"), "{message}");
            message.split(" for ").next().unwrap().to_string()
        })
        .collect();
    (messages, all)
}

#[test]
fn a_dht22_past_80_c_warns_once_and_keeps_its_range() {
    let (found, _) = run("dht22", "110", 1);
    assert_eq!(
        found,
        ["#u1__sensor sits at 110 degC, outside its -40 degC to 80 degC operating temperature"]
    );
    for inside in ["80", "-40", "25"] {
        let (found, all) = run("dht22", inside, 1);
        assert!(found.is_empty(), "{inside} degC: {all:?}");
    }
}

#[test]
fn a_bmp280_past_85_c_warns_once_and_keeps_its_range() {
    let (found, _) = run("bmp280", "110", 1);
    assert_eq!(
        found,
        ["#u1 sits at 110 degC, outside its -40 degC to 85 degC operating temperature"]
    );
    for inside in ["85", "-40", "70"] {
        let (found, all) = run("bmp280", inside, 1);
        assert!(found.is_empty(), "{inside} degC: {all:?}");
    }
}

/// A world that steps out and back warns once, from the step out, for the time it spent out,
/// and names the farthest it went.
#[test]
fn a_world_that_leaves_and_returns_warns_once_with_its_time_outside() {
    let world = r#"{ "initial": 25, "steps": [{ "at": "1ms", "to": 90 }, { "at": "2ms", "to": 120 }, { "at": "3ms", "to": 25 }] }"#;
    let (_, all) = run("bmp280", world, 5);
    let found: Vec<_> = all
        .iter()
        .filter(|d| d["code"] == "rating.temperature")
        .collect();
    assert_eq!(found.len(), 1, "{all:?}");
    let d = found[0];
    assert_eq!(d["t_ps"], 1_000_000_000u64);
    assert_eq!(d["refs"], serde_json::json!(["#u1"]));
    assert_eq!(d["data"]["peak"], 120.0);
    assert_eq!(d["data"]["peak_ps"], 2_000_000_000u64);
    assert_eq!(d["data"]["low"], -40.0);
    assert_eq!(d["data"]["high"], 85.0);
    assert_eq!(d["data"]["over_ps"], 2_000_000_000u64);
}
