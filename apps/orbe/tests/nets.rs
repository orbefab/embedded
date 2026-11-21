//! `orbe nets`: each net, the pins on it, and a pin on no net.

#[path = "../../../packages/scene/tests/common/mod.rs"]
mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn orbe(dir: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

const SCENE: &str = r#"{
  "format": 1,
  "parts": {
    "bb": { "type": "breadboard-400" },
    "hdr": { "type": "pin-header-2x3" },
    "r": { "type": "resistor-axial", "resistance": "220" },
    "spare": { "type": "resistor-axial", "resistance": "220" },
  },
  "wires": [
    ["r.b", "bb.a1"],
  ],
  "nets": { "GND": "r.a" },
  "ground": "r.a",
}
"#;

fn project() -> tempfile::TempDir {
    let dir = common::project("nano-button");
    fs::write(dir.path().join("scene.jsonc"), SCENE).unwrap();
    dir
}

#[test]
fn nets_names_each_net_a_floating_one_and_a_pin_on_none() {
    let dir = project();
    let run = orbe(dir.path(), &["nets"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stdout,
        "\
GND (floating): r.a
r__b (floating): bb.a1 bb.b1 bb.c1 bb.d1 bb.e1 r.b
spare__a (floating): spare.a
spare__b (floating): spare.b
not connected: hdr.1 hdr.2 hdr.3 hdr.4 hdr.5 hdr.6
"
    );
    assert!(!run.stdout.contains("bb.a2"), "{}", run.stdout);

    let run = orbe(dir.path(), &["nets", "--json"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let envelope: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(
        envelope,
        serde_json::json!({
            "ok": true,
            "data": {
                "scene": "scene.jsonc",
                "ground": "GND",
                "nets": [
                    { "name": "GND", "pins": ["r.a"], "floating": true },
                    {
                        "name": "r__b",
                        "pins": ["bb.a1", "bb.b1", "bb.c1", "bb.d1", "bb.e1", "r.b"],
                        "floating": true
                    },
                    { "name": "spare__a", "pins": ["spare.a"], "floating": true },
                    { "name": "spare__b", "pins": ["spare.b"], "floating": true }
                ],
                "unconnected": ["hdr.1", "hdr.2", "hdr.3", "hdr.4", "hdr.5", "hdr.6"]
            }
        })
    );

    let missing = orbe(dir.path(), &["nets", "missing.jsonc"]);
    assert_eq!(missing.code, 3, "{}", missing.stdout);
    assert!(
        missing.stderr.contains("no scene file at missing.jsonc"),
        "{}",
        missing.stderr
    );
}
