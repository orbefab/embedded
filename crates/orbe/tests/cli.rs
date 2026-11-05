//! `orbe check` on a project built from the reader's fixture.

#[path = "../../orbe-scene/tests/common/mod.rs"]
mod common;

use std::fs;
use std::process::{Command, Output};

use serde_json::Value;

fn orbe(args: &[&str]) -> (Output, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    (output, stdout)
}

#[test]
fn a_scene_that_reads_is_ok() {
    let project = common::project("nano-button");
    let scene = project.path().join("scene.jsonc");
    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("ok, 18 engine parts, 31 nets, ground nano__GND"),
        "{stdout}"
    );

    let (output, stdout) = orbe(&["check", "--json", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    let reply: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["data"]["parts"], 18);
    assert_eq!(reply["data"]["nets"], 31);
    assert_eq!(reply["data"]["ground"], "nano__GND");
}

#[test]
fn a_finding_prints_its_place_code_and_hint() {
    let project = common::project("nano-button");
    let scene = project.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(&scene, text.replace("\"led1.anode\"", "\"led1.anod\"")).unwrap();
    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let expected = format!(
        "{}:17:14: error[scene.unknown_pin]: `led1` has no pin `anod`\n  hint: did you mean `led1.anode`? led1 is a led-5mm; its pins: anode, cathode\n",
        scene.display()
    );
    assert!(stdout.starts_with(&expected), "{stdout}");

    let (output, stdout) = orbe(&["check", "--json", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let reply: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "scene_invalid");
    let finding = &reply["data"]["diagnostics"][0];
    assert_eq!(finding["code"], "scene.unknown_pin");
    assert_eq!(finding["file"], scene.display().to_string());
    assert_eq!(
        (finding["line"].as_u64(), finding["column"].as_u64()),
        (Some(17), Some(14))
    );
    assert_eq!(finding["message"], "`led1` has no pin `anod`");
    assert_eq!(
        finding["hint"],
        "did you mean `led1.anode`? led1 is a led-5mm; its pins: anode, cathode"
    );
}

#[test]
fn a_missing_scene_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let (output, _) = orbe(&["check", dir.path().join("scene.jsonc").to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3));
}
