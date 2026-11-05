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

#[test]
fn json_usage_is_one_envelope_on_stdout() {
    let (output, stdout) = orbe(&["check", "--json"]);
    assert_eq!(output.status.code(), Some(2), "{stdout}");
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].is_object());
    assert_eq!(reply["error"]["code"], "usage");
    assert!(!reply["error"]["message"].as_str().unwrap_or("").is_empty());
    assert_eq!(stdout.lines().count(), 1, "{stdout}");

    let (output, stdout) = orbe(&["check", "--json", "scene.jsonc", "extra"]);
    assert_eq!(output.status.code(), Some(2), "{stdout}");
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "usage");
}

#[test]
fn help_is_not_a_usage_envelope() {
    let (output, stdout) = orbe(&["check", "--help"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("check"), "{stdout}");
}

#[test]
fn every_bad_fixture_exits_1_and_prints_a_hint() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../orbe-scene/tests/bad");
    let mut cases: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    for case in cases {
        let project = common::project("nano-button");
        for entry in fs::read_dir(&case).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap();
            if name == "expect" || name == "edit" {
                continue;
            }
            let target = project.path().join(name);
            if path.is_dir() {
                common::copy_dir(&path, &target);
            } else {
                fs::copy(&path, &target).unwrap();
            }
        }
        if let Ok(edit) = fs::read_to_string(case.join("edit")) {
            let lines: Vec<&str> = edit.lines().collect();
            let target = project.path().join(lines[0]);
            let text = fs::read_to_string(&target).unwrap();
            fs::write(&target, text.replacen(lines[1], lines[2], 1)).unwrap();
        }
        let scene = project.path().join("scene.jsonc");
        let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}: {stdout}",
            case.file_name().unwrap().to_string_lossy()
        );
        assert!(
            stdout.contains("\n  hint: ") && !stdout.contains("\n  hint: \n"),
            "{}:\n{stdout}",
            case.file_name().unwrap().to_string_lossy()
        );
    }
}
