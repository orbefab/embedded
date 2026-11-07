//! `orbe lint`: the check's errors when a scene does not check, else warnings that never fail.

#[path = "../../orbe-scene/tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
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

fn json(run: &Run) -> Value {
    serde_json::from_str(&run.stdout).unwrap()
}

/// Each warning line's line number and code, as `orbe lint` prints them.
fn warned(stdout: &str) -> Vec<(usize, String)> {
    stdout
        .lines()
        .filter_map(|l| {
            let (place, rest) = l.split_once(": warning[")?;
            let line = place.split(':').nth(1)?.parse().ok()?;
            Some((line, rest.split(']').next()?.to_string()))
        })
        .collect()
}

/// The 1-based line of the scene that holds `text`.
fn line_of(scene: &str, text: &str) -> usize {
    scene
        .lines()
        .position(|l| l.contains(text))
        .unwrap_or_else(|| panic!("the scene has no `{text}`"))
        + 1
}

/// A project with the first target's parts and firmware, and `scene` as its scene.
fn project(scene: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let first = common::repo().join("examples/first-target");
    common::copy_dir(&first.join("parts"), &dir.path().join("parts"));
    fs::copy(first.join("parts.lock"), dir.path().join("parts.lock")).unwrap();
    fs::create_dir(dir.path().join("build")).unwrap();
    fs::copy(common::firmware(), dir.path().join("build/button.hex")).unwrap();
    fs::write(dir.path().join("scene.jsonc"), scene).unwrap();
    dir
}

fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files(&path));
        } else {
            found.insert(path.clone(), fs::read(&path).unwrap());
        }
    }
    found
}

#[test]
fn lint_warns_on_the_first_target_exits_zero_and_writes_nothing() {
    let repo = common::repo();
    let example = repo.join("examples/first-target");
    let before = files(&example);
    let run = orbe(&repo, &["lint", "examples/first-target/scene.jsonc"]);
    assert_eq!(files(&example), before, "lint wrote nothing");
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);

    let scene = fs::read_to_string(example.join("scene.jsonc")).unwrap();
    let at = |text: &str| line_of(&scene, text);
    let nano = at("\"nano\":");
    let mut want = vec![
        (at("{"), "scene.ground_unnamed"),
        (nano, "scene.unplaced"),
        (nano, "scene.unconnected"),
        (nano, "scene.unsimulated_pins"),
        (nano, "net.floating"),
    ];
    for part in ["btn1", "r2", "r1", "led1"] {
        want.push((at(&format!("\"{part}\":")), "scene.unplaced"));
    }
    let want: Vec<(usize, String)> = want.into_iter().map(|(l, c)| (l, c.into())).collect();
    assert_eq!(warned(&run.stdout), want, "{}", run.stdout);
    assert!(
        run.stdout
            .ends_with("9 warnings in examples/first-target/scene.jsonc\n")
    );

    let unwired = run
        .stdout
        .lines()
        .find(|l| l.contains("warning[scene.unconnected]"))
        .unwrap();
    for pin in ["D3", "A0", "AREF", "RESET"] {
        assert!(unwired.contains(&format!(" {pin},")), "{unwired}");
    }
    for pin in ["D2", "D13", "5V", "GND", "3V3", "VIN"] {
        assert!(!unwired.contains(&format!(" {pin},")), "{unwired}");
        assert!(!unwired.contains(&format!(" {pin}\n")), "{unwired}");
    }
    assert!(run.stdout.contains(
        "warning[scene.unsimulated_pins]: nano-board does not simulate 3V3, A6, A7 and VIN."
    ));
    assert!(
        run.stdout
            .contains("add \"GND\": \"nano.GND\" to the scene's \"nets\"")
    );

    let envelope = json(&orbe(
        &repo,
        &["lint", "examples/first-target/scene.jsonc", "--json"],
    ));
    assert_eq!(envelope["ok"], true);
    let found = envelope["data"]["diagnostics"].as_array().unwrap();
    assert_eq!(found.len(), 9);
    for d in found {
        let keys: Vec<&str> = d.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["code", "column", "file", "hint", "line", "message"]);
        assert!(!d["hint"].as_str().unwrap().is_empty(), "{d}");
    }
    assert_eq!(found[0]["code"], "scene.ground_unnamed");
    assert_eq!(found[0]["file"], "examples/first-target/scene.jsonc");
}

#[test]
fn lint_on_a_scene_that_does_not_check_prints_and_exits_as_check_does() {
    let dir = project("{ \"format\": 1, \"parts\": { \"led1\": { \"type\": \"led-5mm\" } } }\n");
    for json in [false, true] {
        let flag: &[&str] = if json { &["--json"] } else { &[] };
        let check = orbe(dir.path(), &[&["check", "scene.jsonc"], flag].concat());
        let lint = orbe(dir.path(), &[&["lint", "scene.jsonc"], flag].concat());
        assert_eq!(check.code, 1);
        assert_eq!(
            (lint.code, &lint.stdout, &lint.stderr),
            (check.code, &check.stdout, &check.stderr)
        );
        assert!(!lint.stdout.contains("warning["), "{}", lint.stdout);
    }
    let missing = orbe(dir.path(), &["lint", "nope.jsonc"]);
    assert_eq!(missing.code, 3);
    assert_eq!(
        missing.stderr,
        "error[io_error]: no scene file at nope.jsonc\n"
    );
}

#[test]
fn each_rule_warns_where_the_scene_goes_wrong() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.hex" },
    "led1": { "type": "led-5mm", "color": "red" },
    "led2": { "type": "led-5mm", "color": "red" },
    "hdr": { "type": "pin-header-2x3" },
    "r9": { "type": "resistor-axial", "resistance": "10k" },
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.D13", "led1.anode"],
    ["led1.cathode", "nano.GND"],
    ["nano.D2", "r9.a"],
    ["r9.b", "nano.GND"],
    ["hdr.1", "nano.D3"],
  ],
  "layout": {
    "nano": { "at": [0, 0, 0], "rotate": [0, 0, 270] },
    "led1": { "at": [0, 0, 0], "rotate": [-180, 0, 540] },
    "hdr": { "at": [0, 0, 0] },
    "r9": { "at": [0, 0, 0], "rotate": [0, 0, 90] },
  },
}
"#;
    let dir = project(scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let at = |text: &str| line_of(scene, text);
    let want = [
        (at("\"nano\": { \"type\""), "scene.unconnected"),
        (at("\"nano\": { \"type\""), "scene.unsimulated_pins"),
        (at("\"nano\": { \"type\""), "net.floating"),
        (at("\"led1\": { \"type\""), "scene.led_no_resistor"),
        (at("\"led2\""), "scene.unplaced"),
        (at("\"led2\""), "scene.unconnected"),
        (at("\"led2\""), "net.floating"),
        (at("\"led2\""), "net.floating"),
        (at("\"hdr\": { \"type\""), "scene.unconnected"),
        (at("\"hdr\": { \"type\""), "scene.no_model"),
        (at("[0, 0, 270]"), "scene.rotation"),
        (at("[-180, 0, 540]"), "scene.rotation"),
    ];
    let want: Vec<(usize, String)> = want.into_iter().map(|(l, c)| (l, c.into())).collect();
    assert_eq!(warned(&run.stdout), want, "{}", run.stdout);
    for line in [
        "  hint: write \"rotate\": [0, 0, -90]",
        "  hint: write \"rotate\": [180, 0, 180]",
        "warning[scene.unconnected]: nothing is wired to `led2`",
        "warning[scene.led_no_resistor]: nothing limits the current through `led1`: it joins `GND` and `nano__D13` with no resistor in series",
        "warning[scene.no_model]: `hdr` is a pin-header-2x3, which has no engine model",
    ] {
        assert!(run.stdout.contains(line), "no `{line}` in\n{}", run.stdout);
    }
}

#[test]
fn a_resistor_beside_an_led_limits_nothing_and_one_in_series_on_either_side_does() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.hex" },
    "btn1": { "type": "tactile-switch-6mm" },
    "beside": { "type": "led-5mm", "color": "red" },
    "r_beside": { "type": "resistor-axial", "resistance": "1k" },
    "above": { "type": "led-5mm", "color": "red" },
    "r_above": { "type": "resistor-axial", "resistance": "220" },
    "below": { "type": "led-5mm", "color": "red" },
    "r_below": { "type": "resistor-axial", "resistance": "220" },
  },
  "wires": [
    ["nano.5V", "btn1.a"],
    ["btn1.b", "beside.anode"],
    ["beside.cathode", "nano.GND"],
    ["btn1.b", "r_beside.a"],
    ["r_beside.b", "nano.GND"],
    ["nano.D12", "r_above.a"],
    ["r_above.b", "above.anode"],
    ["above.cathode", "nano.GND"],
    ["nano.D11", "below.anode"],
    ["below.cathode", "r_below.a"],
    ["r_below.b", "nano.GND"],
  ],
}
"#;
    let dir = project(scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let unlimited: Vec<usize> = warned(&run.stdout)
        .into_iter()
        .filter(|(_, code)| code == "scene.led_no_resistor")
        .map(|(line, _)| line)
        .collect();
    assert_eq!(unlimited, [line_of(scene, "\"beside\":")], "{}", run.stdout);
}
