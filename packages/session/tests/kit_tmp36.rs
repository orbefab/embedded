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

fn try_open_scene(scene: Value) -> (Live, orbe_session::Reply) {
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
    (live, reply)
}

fn open_scene(scene: Value) -> Live {
    let (live, reply) = try_open_scene(scene);
    assert!(reply.ok(), "{}", reply.text());
    live
}

fn run(live: &mut Live, line: &str) -> orbe_session::Reply {
    let reply = live.run(&Command::parse(line, Path::new(".")).unwrap().unwrap());
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply
}

fn bench_scene(supply_v: f64, temp_c: f64, load_ohm: Option<&str>) -> Value {
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
    json!({
        "format": 1,
        "parts": parts,
        "ground": "vs.neg",
        "world": { "temperature": temp_c },
        "wires": wires
    })
}

fn run_bench(supply_v: f64, temp_c: f64, load_ohm: Option<&str>) -> (f64, f64, Vec<Value>) {
    let mut live = open_scene(bench_scene(supply_v, temp_c, load_ohm));
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
fn tmp36_output_resistance_uses_high_temp_row_above_105c() {
    // Table 1 p3 (-40 to +105 C): 6 mC/uA typ * 10 mV/C = 60 ohm.
    let (vout_105, _, diags_105) = run_bench(5.0, 105.0, Some("10k"));
    let expected_105 = 1.55 * 10_000.0 / 10_060.0;
    assert!(
        (vout_105 - expected_105).abs() < 1e-4,
        "at 105 C got {vout_105} V, want {expected_105} V (60 ohm)"
    );
    assert!(diags_105.is_empty(), "105 C warned: {diags_105:?}");

    // Table 1 p3 (+105 to +125 C): 25 mC/uA typ * 10 mV/C = 250 ohm.
    let (vout_120, _, diags_120) = run_bench(5.0, 120.0, Some("10k"));
    let expected_120 = 1.70 * 10_000.0 / 10_250.0;
    assert!(
        (vout_120 - expected_120).abs() < 1e-4,
        "at 120 C got {vout_120} V, want {expected_120} V (250 ohm)"
    );
    assert!(diags_120.is_empty(), "120 C warned: {diags_120:?}");

    // Live temperature step across 105 C moves output resistance from 60 ohm to 250 ohm.
    let mut live = open_scene(bench_scene(5.0, 25.0, Some("10k")));
    run(&mut live, "step 5ms");
    let v_25 = run(&mut live, "get @tmp.VOUT").json()["data"]["value"]["v"]
        .as_f64()
        .unwrap();
    let expected_25 = 0.75 * 10_000.0 / 10_060.0;
    assert!(
        (v_25 - expected_25).abs() < 1e-4,
        "live 25 C got {v_25} V, want {expected_25} V"
    );
    run(&mut live, "set temperature 120");
    run(&mut live, "step 5ms");
    let v_120 = run(&mut live, "get @tmp.VOUT").json()["data"]["value"]["v"]
        .as_f64()
        .unwrap();
    assert!(
        (v_120 - expected_120).abs() < 1e-4,
        "live 120 C got {v_120} V, want {expected_120} V"
    );
}

#[test]
fn tmp36_supply_range_warns_operating_above_5v5_abs_max_above_7v_and_no_invented_floor() {
    // Operating supply boundaries (2.7 V, 5.0 V, 5.5 V): accurate and quiet.
    for supply in [2.7, 5.0, 5.5] {
        let (vout, iin, diags) = run_bench(supply, 25.0, None);
        assert!((vout - 0.75).abs() < 1e-4, "supply={supply} vout={vout}");
        assert!((iin - 50e-6).abs() < 1e-7, "supply={supply} iin={iin}");
        assert!(
            diags.is_empty(),
            "supply={supply} unexpectedly warned: {diags:?}"
        );
    }

    // Unpowered (0 V) with 10k load: 0 V output, 0 A draw, quiet.
    let (vout, iin, diags) = run_bench(0.0, 25.0, Some("10k"));
    assert!(vout.abs() < 1e-6, "unpowered vout={vout}");
    assert!(iin.abs() < 1e-9, "unpowered iin={iin}");
    assert!(diags.is_empty(), "unpowered warned: {diags:?}");

    // Below operating supply (1.2 V < 2.7 V) at 100 C (target 1.5 V): clamps to VS and warns.
    let (vout, _, diags) = run_bench(1.2, 100.0, None);
    assert!((vout - 1.2).abs() < 5e-3, "clamped vout={vout}");
    assert_eq!(diags.len(), 1, "expected 1 warning at 1.2 V, got {diags:?}");
    assert_eq!(diags[0]["code"], "rating.supply_low");
    assert_eq!(diags[0]["refs"], json!(["#tmp.pin.vs"]));
    assert!(
        diags[0]["message"]
            .as_str()
            .unwrap_or("")
            .contains("below its 2.7 V minimum"),
        "expected the 2.7 V minimum supply warning, got {diags:?}"
    );

    // Above operating supply (5.6 V, 6.0 V, 7.0 V): keeps working and warns operating ceiling.
    for supply in [5.6, 6.0, 7.0] {
        let (vout, _, diags) = run_bench(supply, 25.0, None);
        assert!((vout - 0.75).abs() < 1e-4, "supply={supply} vout={vout}");
        assert_eq!(
            diags.len(),
            1,
            "expected 1 warning at {supply} V, got {diags:?}"
        );
        assert_eq!(diags[0]["code"], "rating.voltage");
        assert!(
            diags[0]["message"]
                .as_str()
                .unwrap_or("")
                .contains("maximum operating supply"),
            "supply={supply}: expected maximum operating supply warning, got {diags:?}"
        );
    }

    // Past absolute maximum (8.0 V > 7.0 V): keeps working, warns 7 V ceiling, no invented -0.3 V.
    let (vout, _, diags) = run_bench(8.0, 25.0, None);
    assert!((vout - 0.75).abs() < 1e-4, "over-supply vout={vout}");
    assert_eq!(diags.len(), 1, "expected 1 warning at 8 V, got {diags:?}");
    assert_eq!(diags[0]["code"], "rating.voltage");
    let msg = diags[0]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("7 V ceiling"),
        "expected 7 V ceiling warning, got {msg:?}"
    );
    assert!(
        !msg.contains("-0.3"),
        "must not invent -0.3 V floor not in Rev. H: {msg:?}"
    );

    // Reversed supply (-5.0 V): Rev. H states no reverse supply rating; output stays 0 V, no invented warning.
    let (vout, _, diags) = run_bench(-5.0, 25.0, None);
    assert!(vout.abs() < 1e-6, "reversed vout={vout}");
    assert!(
        diags.is_empty(),
        "reversed supply must not emit an invented rating warning: {diags:?}"
    );
}

#[test]
fn world_temperature_stays_within_minus_40_to_125c() {
    // Ends of the world range (-40 C and 125 C) open and run cleanly.
    for (temp, expected_v) in [(-40.0, 0.10), (25.0, 0.75), (40.0, 0.90), (125.0, 1.75)] {
        let (vout, _, diags) = run_bench(5.0, temp, None);
        assert!((vout - expected_v).abs() < 1e-4, "temp={temp} vout={vout}");
        assert!(diags.is_empty(), "temp={temp} warned: {diags:?}");
    }

    // Outside -40..125 C (-45 C and 140 C) is refused at scene open.
    for temp in [-45.0, 140.0] {
        let (_, reply) = try_open_scene(bench_scene(5.0, temp, None));
        assert!(
            !reply.ok(),
            "world temperature {temp} C must be refused, got: {}",
            reply.text()
        );
    }
}

#[test]
fn tmp36_part_text_cites_max_short_circuit_current_and_reversed_supply() {
    let text = std::fs::read_to_string(repo().join("parts/tmp36/part.jsonc")).unwrap();
    assert!(
        !text.contains("250 uA typical"),
        "Table 1 lists 250 uA short-circuit current in the Max column, not typical"
    );
    assert!(
        text.contains("250 uA") && text.contains("Note 2"),
        "short-circuit current note should cite Table 1 Max (Note 2)"
    );
    assert!(
        text.contains("reversed supply"),
        "not_simulated should document reversed supply"
    );
}
