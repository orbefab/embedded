//! Bench-honesty checks for the 4x4 membrane matrix keypad (`keypad-4x4`).
//! Verifies contact resistance, insulation resistance, current/voltage rating warnings,
//! optional bounce forwarding, unprotected-matrix 3-key ghosting, and viewer switch bindings.

use std::path::{Path, PathBuf};

use orbe_session::viewer;
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

fn open_scene(scene: Value, extra: Vec<(String, Vec<u8>)>) -> Live {
    let block = json!({
        "format": 1,
        "block": "test",
        "version": "0.0.0",
        "title": "test",
        "board": "nano-board",
        "license": "MIT OR Apache-2.0"
    });
    let mut entries = vec![
        ("block.jsonc".to_string(), block.to_string().into_bytes()),
        (
            "source.jsonc".to_string(),
            DC_SOURCE_PART.as_bytes().to_vec(),
        ),
        ("scene.jsonc".to_string(), scene.to_string().into_bytes()),
    ];
    entries.extend(extra);
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

fn number(live: &mut Live, target: &str) -> f64 {
    let reply = run(live, &format!("get {target}"));
    let val = &reply.json()["data"]["value"];
    val.as_f64()
        .or_else(|| val["v"].as_f64())
        .or_else(|| val["i"].as_f64())
        .unwrap_or_else(|| panic!("{target}: {}", reply.text()))
}

#[test]
fn keypad_contact_insulation_and_current_and_voltage_ratings_behave_like_the_sheet() {
    // 1. Contact resistance (100 ohm closed) and insulation resistance (100 Mohm open)
    //    under safe bias (2 V across R1-C1 -> 20 mA closed, 20 nA open, no warnings).
    let safe_scene = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "2" },
            "pad": { "type": "keypad-4x4" },
        },
        "wires": [
            ["v1.pos", "pad.R1"],
            ["pad.C1", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(safe_scene, vec![]);
    run(&mut live, "step 10ms");
    let i_open = number(&mut live, "@pad.k1.i").abs();
    assert!(
        (1.9e-8..2.1e-8).contains(&i_open),
        "open insulation current at 2 V should be ~20 nA (100 Mohm), got {i_open} A"
    );
    run(&mut live, "press @pad.k1");
    run(&mut live, "step 10ms");
    let i_closed = number(&mut live, "@pad.k1.i").abs();
    assert!(
        (0.0195..0.0205).contains(&i_closed),
        "closed contact current at 2 V should be ~20 mA (100 ohm), got {i_closed} A"
    );
    run(&mut live, "expect no diagnostics");

    // 2. Press past rated 30 mA (5 V across 100 ohm = 50 mA) warns rating.current.
    let over_i_scene = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "5" },
            "pad": { "type": "keypad-4x4" },
        },
        "wires": [
            ["v1.pos", "pad.R1"],
            ["pad.C1", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(over_i_scene, vec![]);
    run(&mut live, "step 10ms");
    run(&mut live, "expect no diagnostics");
    run(&mut live, "press @pad.k1");
    let step = run(&mut live, "step 10ms");
    assert!(
        step.text().contains("rating.current"),
        "5 V across closed keypad contact (50 mA > 30 mA) must warn rating.current: {}",
        step.text()
    );
    let i_over = number(&mut live, "@pad.k1.i").abs();
    assert!(
        (0.049..0.051).contains(&i_over),
        "expected 50 mA, got {i_over}"
    );

    // 3. Voltage past rated 24 V (30 V across open contact) warns rating.voltage.
    let over_v_scene = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "30" },
            "pad": { "type": "keypad-4x4" },
        },
        "wires": [
            ["v1.pos", "pad.R1"],
            ["pad.C1", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(over_v_scene, vec![]);
    run(&mut live, "step 10ms");
    let diags = run(&mut live, "snapshot").json()["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        diags.iter().any(|d| d["code"] == "rating.voltage"
            && d["message"]
                .as_str()
                .unwrap_or("")
                .contains("reaches 30 V, outside its -24 V to 24 V rating")),
        "30 V across open keypad contact (> 24 V) must warn rating.voltage, got {diags:?}"
    );
}

#[test]
fn keypad_reproduces_real_unprotected_matrix_ghosting_with_three_keys_held() {
    // Classic 4x4 matrix scan: drive C1 LOW (0 V), pull R1 and R2 HIGH (5 V via 10 kohm),
    // leave C2 floating (hi-Z, as Keypad.h does for inactive columns).
    // With k1 (R1-C1), k2 (R1-C2) and k5 (R2-C2) held simultaneously, R2 has a sneak path
    // through k5 -> C2 -> k2 -> R1 -> k1 -> C1 (300 ohm total against 10 kohm pull-up),
    // pulling R2 to ~0.146 V (LOW) so k4 (R2-C1) ghosts as pressed even though k4 is open!
    let scene = json!({
        "format": 1,
        "parts": {
            "v5": { "type": "./source.jsonc", "voltage": "5" },
            "pr1": { "type": "resistor-axial", "resistance": "10k" },
            "pr2": { "type": "resistor-axial", "resistance": "10k" },
            "pad": { "type": "keypad-4x4" },
        },
        "wires": [
            ["v5.pos", "pr1.a"], ["pr1.b", "pad.R1"],
            ["v5.pos", "pr2.a"], ["pr2.b", "pad.R2"],
            ["pad.C1", "v5.neg"],
        ],
        "ground": "v5.neg",
    });
    let mut live = open_scene(scene, vec![]);
    run(&mut live, "step 5ms");
    // Nothing pressed: R1 and R2 sit near 5 V.
    assert!(number(&mut live, "@pad.R1") > 4.9);
    assert!(number(&mut live, "@pad.R2") > 4.9);

    // Hold two corners (k1 on R1-C1 and k2 on R1-C2): R1 goes LOW, R2 stays HIGH (no ghost yet).
    run(&mut live, "press @pad.k1");
    run(&mut live, "press @pad.k2");
    run(&mut live, "step 5ms");
    assert!(
        number(&mut live, "@pad.R1") < 0.1,
        "R1 should be LOW via k1"
    );
    assert!(
        number(&mut live, "@pad.R2") > 4.9,
        "R2 must stay HIGH with only k1+k2 held"
    );

    // Now press the third corner k5 (R2-C2): sneak path R2 -> k5 -> C2 -> k2 -> R1 -> k1 -> C1
    // pulls R2 LOW (~0.15 V) even though k4 (R2-C1) is NOT pressed!
    run(&mut live, "press @pad.k5");
    run(&mut live, "step 5ms");
    let v_r2_ghost = number(&mut live, "@pad.R2");
    assert!(
        v_r2_ghost < 0.25,
        "holding k1+k2+k5 must ghost k4 (pull R2 LOW via 300 ohm sneak path), got R2 = {v_r2_ghost} V"
    );

    // Releasing k2 breaks the sneak path so R2 returns HIGH immediately.
    run(&mut live, "release @pad.k2");
    run(&mut live, "step 5ms");
    let v_r2_clear = number(&mut live, "@pad.R2");
    assert!(
        v_r2_clear > 4.9,
        "releasing k2 must clear the ghost on R2, got {v_r2_clear} V"
    );
    run(&mut live, "expect no diagnostics");
}

#[test]
fn keypad_viewer_channels_expose_all_16_keys_and_accept_clicks() {
    let hex = std::fs::read(repo().join("blocks/nano-keypad/build/keypad.ino.hex")).unwrap();
    let scene = json!({
        "format": 1,
        "parts": {
            "nano": { "type": "nano-board", "firmware": "build/keypad.ino.hex" },
            "pad": { "type": "keypad-4x4" },
        },
        "wires": [
            ["nano.D11", "pad.R1"],
            ["nano.D10", "pad.R2"],
            ["nano.D9", "pad.R3"],
            ["nano.D8", "pad.R4"],
            ["nano.D5", "pad.C1"],
            ["nano.D4", "pad.C2"],
            ["nano.D3", "pad.C3"],
            ["nano.D2", "pad.C4"],
        ],
    });
    let mut live = open_scene(scene, vec![("build/keypad.ino.hex".into(), hex)]);
    let scene_msg = live.scene().unwrap();
    let switches: Vec<&str> = scene_msg["channels"]["switches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for key in [
        "@pad.k1",
        "@pad.k2",
        "@pad.k3",
        "@pad.ka",
        "@pad.k4",
        "@pad.k5",
        "@pad.k6",
        "@pad.kb",
        "@pad.k7",
        "@pad.k8",
        "@pad.k9",
        "@pad.kc",
        "@pad.kstar",
        "@pad.k0",
        "@pad.khash",
        "@pad.kd",
    ] {
        assert!(
            switches.contains(&key),
            "viewer switches missing {key}: {switches:?}"
        );
    }

    // Drive a click via viewer protocol commands (as clicking in the 3D viewer does).
    run(
        &mut live,
        "step --until serial contains \"ready\" --timeout 100ms",
    );
    let (r1, _) = viewer::answer(&mut live, r#"{"id": 1, "cmd": "press", "ref": "@pad.k5"}"#);
    assert!(r1.ok(), "{}", r1.text());
    run(
        &mut live,
        "step --until serial contains \"key=5\" --timeout 100ms",
    );
    let (r2, _) = viewer::answer(
        &mut live,
        r#"{"id": 2, "cmd": "release", "ref": "@pad.k5"}"#,
    );
    assert!(r2.ok(), "{}", r2.text());
    run(&mut live, "step 30ms");
    run(&mut live, "expect no diagnostics");

    // Bounce param forwards from keypad-4x4 to child keys (5 ms window).
    let bounced = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "2" },
            "pad": { "type": "keypad-4x4", "bounce": "5ms" },
        },
        "wires": [
            ["v1.pos", "pad.R1"],
            ["pad.C1", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(bounced, vec![]);
    run(&mut live, "step 1ms");
    run(&mut live, "press @pad.k1");
    run(&mut live, "step 10ms");
    let i_settled = number(&mut live, "@pad.k1.i").abs();
    assert!((0.0195..0.0205).contains(&i_settled), "got {i_settled} A");
    run(&mut live, "expect no diagnostics");
}
