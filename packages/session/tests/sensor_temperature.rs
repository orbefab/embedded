//! The catalog's DHT22 and BMP280 in a live session past their operating temperature:
//! - A sensor on a 0 V or 0.05 V supply, or with its supply pins open, is not operating, so it
//!   does not warn `rating.temperature`.
//! - A powered sensor's `snapshot` counts its time outside up to the snapshot, not to the last
//!   accepted step, and so does a pin's `rating.voltage`.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_session::{Command, Files, Live};
use serde_json::Value;

const MS: u64 = 1_000_000_000;

const SUPPLY: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "params": { "voltage": { "unit": "V", "required": true, "sets": "/voltage" } },
  "model": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } }
}
"#;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The catalog's DHT22 and BMP280 part files, copied into the project's `parts/`.
fn parts() -> Vec<(PathBuf, Vec<u8>)> {
    ["dht22", "bmp280"]
        .iter()
        .map(|id| {
            let path = format!("{id}/part.jsonc");
            let bytes = fs::read(repo().join("parts").join(&path)).unwrap();
            (Path::new("proj/parts").join(path), bytes)
        })
        .collect()
}

/// A live session on a DHT22 `u1` and a BMP280 `u2` on one `volts` supply, in a world whose
/// temperature is `world`. `wired` false leaves both sensors' supply pins open.
fn open(volts: &str, wired: bool, world: &str) -> Live {
    let supply = if wired {
        r#"["v1.pos", "u1.VDD"], ["v1.neg", "u1.GND"],
           ["v1.pos", "u2.VDD"], ["v1.pos", "u2.VDDIO"], ["v1.neg", "u2.GND"],
           ["v1.pos", "u2.CSB"], ["v1.pos", "u2.SDI"], ["v1.pos", "u2.SCK"], ["v1.neg", "u2.SDO"]"#
    } else {
        r#"["v1.neg", "u1.DATA"], ["v1.neg", "u2.SDO"]"#
    };
    let scene = format!(
        r#"{{ "format": 1,
          "parts": {{ "v1": {{ "type": "./supply/part.jsonc", "voltage": "{volts}" }},
                     "u1": {{ "type": "dht22" }}, "u2": {{ "type": "bmp280" }} }},
          "wires": [{supply}], "ground": "v1.neg", "world": {{ "temperature": {world} }} }}"#
    );
    let mut files = parts();
    files.push((PathBuf::from("proj/scene.jsonc"), scene.into_bytes()));
    files.push((
        PathBuf::from("proj/supply/part.jsonc"),
        SUPPLY.as_bytes().to_vec(),
    ));
    let files = Files::memory(files);
    let mut live = Live::new("sensor-temperature", files);
    run(&mut live, "open");
    live
}

fn run(live: &mut Live, line: &str) -> Value {
    let command = Command::parse(line, Path::new("proj")).unwrap().unwrap();
    let reply = live.run(&command);
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply.json()
}

/// Each `code` warning on `snapshot`, as `(ref, over_ps)`.
fn snapshot(live: &mut Live, code: &str) -> Vec<(String, Value)> {
    let snap = run(live, "snapshot");
    snap["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == code)
        .map(|d| {
            (
                d["refs"][0].as_str().unwrap().to_string(),
                d["data"]["over_ps"].clone(),
            )
        })
        .collect()
}

fn both(over_ps: u64) -> Vec<(String, Value)> {
    vec![
        ("#u1__sensor".to_string(), over_ps.into()),
        ("#u2".to_string(), over_ps.into()),
    ]
}

#[test]
fn an_unpowered_sensor_past_its_range_does_not_warn() {
    for (volts, wired) in [("0", true), ("0.05", true), ("5", false)] {
        let mut live = open(volts, wired, "110");
        assert_eq!(snapshot(&mut live, "rating.temperature"), []);
        run(&mut live, "step 10ms");
        assert_eq!(
            snapshot(&mut live, "rating.temperature"),
            [],
            "{volts} V, wired {wired}"
        );
    }
}

#[test]
fn a_powered_sensor_counts_its_time_outside_up_to_the_snapshot() {
    let mut live = open("3.3", true, "110");
    run(&mut live, "step 10ms");
    assert_eq!(snapshot(&mut live, "rating.temperature"), both(10 * MS));
    run(&mut live, "step 1ms");
    assert_eq!(snapshot(&mut live, "rating.temperature"), both(11 * MS));

    let mut live = open("3.3", true, "110");
    run(&mut live, "step 30ms");
    assert_eq!(snapshot(&mut live, "rating.temperature"), both(30 * MS));
}

#[test]
fn a_world_that_leaves_mid_run_counts_from_the_change_to_the_snapshot() {
    let world = r#"{ "initial": 25, "steps": [{ "at": "10ms", "to": 110 }] }"#;
    let mut live = open("3.3", true, world);
    run(&mut live, "step 11ms");
    assert_eq!(snapshot(&mut live, "rating.temperature"), both(MS));
    run(&mut live, "step 1ms");
    assert_eq!(snapshot(&mut live, "rating.temperature"), both(2 * MS));
}

/// The BMP280's supply pins at 5 V are over their 4.25 V rating from the first step, 1 ps
/// after t = 0, so their time over runs from there to the snapshot.
#[test]
fn a_pin_over_its_voltage_rating_counts_its_time_up_to_the_snapshot() {
    let mut live = open("5", true, "25");
    run(&mut live, "step 10ms");
    let over = |ms: u64| {
        vec![
            ("#u2.pin.VDD".to_string(), Value::from(ms * MS - 1)),
            ("#u2.pin.VDDIO".to_string(), Value::from(ms * MS - 1)),
        ]
    };
    assert_eq!(snapshot(&mut live, "rating.voltage"), over(10));
    run(&mut live, "step 1ms");
    assert_eq!(snapshot(&mut live, "rating.voltage"), over(11));
}
