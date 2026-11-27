//! Bench-honesty checks for the TMP36 analog temperature sensor (`tmp36`) and `nano-tmp36`.

use std::path::{Path, PathBuf};

use orbe_session::{Command, Files, Live};
use serde_json::{Value, json};

const DC_SOURCE_PART: &str = r#"{
  "format": 4,
  "part": "test-dc-source",
  "version": "0.1.0",
  "title": "Test DC source",
  "license": "MIT OR Apache-2.0",
  "pins": { "pos": [1], "neg": [2] },
  "params": { "voltage": { "unit": "V", "required": true, "sets": "/voltage" } },
  "model": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } }
}"#;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn catalog() -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}/"), out);
            } else {
                out.push((format!("{prefix}{name}"), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    walk(&repo().join("parts"), "", &mut files);
    files
}

fn open_scene(scene: Value) -> Live {
    let block = json!({
        "format": 1,
        "block": "test",
        "version": "0.0.0",
        "title": "test",
        "board": "nano-board",
        "license": "MIT OR Apache-2.0"
    });
    let entries = vec![
        ("block.jsonc".to_string(), block.to_string().into_bytes()),
        (
            "source/part.jsonc".to_string(),
            DC_SOURCE_PART.as_bytes().to_vec(),
        ),
        ("scene.jsonc".to_string(), scene.to_string().into_bytes()),
    ];
    let files = Files::memory(entries).with_catalog(catalog());
    let mut live = Live::new("t", files);
    let reply = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(reply.ok(), "{}", reply.text());
    live
}

fn run(live: &mut Live, line: &str) -> orbe_session::Reply {
    let reply = live.run(&Command::parse(line, Path::new(".")).unwrap().unwrap());
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply
}

fn run_bench(supply_v: f64, temp_c: f64, load_ohm: Option<&str>) -> (f64, f64, Vec<Value>) {
    let mut parts = serde_json::Map::new();
    parts.insert(
        "vs".into(),
        json!({ "type": "./source", "voltage": format!("{supply_v}V") }),
    );
    parts.insert("tmp".into(), json!({ "type": "tmp36" }));
    let mut wires = vec![json!(["vs.pos", "tmp.VS"]), json!(["vs.neg", "tmp.GND"])];
    if let Some(ohm) = load_ohm {
        parts.insert(
            "rl".into(),
            json!({ "type": "resistor-axial", "resistance": ohm }),
        );
        wires.push(json!(["tmp.VOUT", "rl.a"]));
        wires.push(json!(["rl.b", "vs.neg"]));
    }
    let scene = json!({
        "format": 1,
        "parts": parts,
        "ground": "vs.neg",
        "world": { "temperature": temp_c },
        "wires": wires
    });
    let mut live = open_scene(scene);
    run(&mut live, "step 5ms");
    let vout = run(&mut live, "get @tmp.VOUT").json()["data"]["value"]["v"]
        .as_f64()
        .unwrap();
    let iin = run(&mut live, "get @tmp.iin").json()["data"]["value"]
        .as_f64()
        .unwrap();
    let snap = run(&mut live, "snapshot").json();
    let diags = snap["data"]["diagnostics"].as_array().unwrap().clone();
    (vout, iin, diags)
}

#[test]
fn tmp36_bench_transfer_unpowered_supply_and_temperature_warnings() {
    // Normal 5 V at 25 C: 750 mV, 50 uA quiescent draw, quiet.
    let (vout, iin, diags) = run_bench(5.0, 25.0, None);
    assert!((vout - 0.75).abs() < 1e-4, "vout={vout}");
    assert!((iin - 50e-6).abs() < 1e-7, "iin={iin}");
    assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");

    // 5 V at 40 C: 900 mV, quiet.
    let (vout, _, diags) = run_bench(5.0, 40.0, None);
    assert!((vout - 0.90).abs() < 1e-4, "vout={vout}");
    assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");

    // Unpowered (0 V) with 10k load: 0 V output, 0 A draw, quiet.
    let (vout, iin, diags) = run_bench(0.0, 25.0, Some("10k"));
    assert!(vout.abs() < 1e-6, "unpowered vout={vout}");
    assert!(iin.abs() < 1e-9, "unpowered iin={iin}");
    assert!(diags.is_empty(), "unpowered warned: {diags:?}");

    // Under-supply (1.2 V < 2.7 V min) at 100 C (target 1.5 V): clamps to VS and warns rating.voltage.
    let (vout, _, diags) = run_bench(1.2, 100.0, None);
    assert!((vout - 1.2).abs() < 5e-3, "clamped vout={vout}");
    assert!(
        diags.iter().any(|d| d["code"] == "rating.voltage"
            && d["message"]
                .as_str()
                .unwrap_or("")
                .contains("minimum operating supply")),
        "expected under-supply rating.voltage, got {diags:?}"
    );

    // Over-supply (8 V > 7 V abs max): keeps working and warns rating.voltage.
    let (vout, _, diags) = run_bench(8.0, 25.0, None);
    assert!((vout - 0.75).abs() < 1e-4, "over-supply vout={vout}");
    assert!(
        diags.iter().any(|d| d["code"] == "rating.voltage"),
        "expected over-supply rating.voltage, got {diags:?}"
    );

    // Temperature outside -40..125 C (140 C and -45 C): keeps working and warns rating.temperature.
    let (vout_hot, _, diags_hot) = run_bench(5.0, 140.0, None);
    assert!((vout_hot - 1.90).abs() < 1e-4, "hot vout={vout_hot}");
    assert!(
        diags_hot.iter().any(|d| d["code"] == "rating.temperature"),
        "expected rating.temperature at 140 C, got {diags_hot:?}"
    );

    let (vout_cold, _, diags_cold) = run_bench(5.0, -45.0, None);
    assert!((vout_cold - 0.05).abs() < 1e-4, "cold vout={vout_cold}");
    assert!(
        diags_cold.iter().any(|d| d["code"] == "rating.temperature"),
        "expected rating.temperature at -45 C, got {diags_cold:?}"
    );
}
