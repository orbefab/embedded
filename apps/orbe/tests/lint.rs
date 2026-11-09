//! `orbe lint`: the check's errors when a scene does not check, else warnings that never fail.

#[path = "../../../packages/scene/tests/common/mod.rs"]
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

/// A project with every catalog part and the button firmware, and `scene` as its scene.
fn project(scene: &str) -> tempfile::TempDir {
    let dir = common::project("nano-button");
    fs::write(dir.path().join("scene.jsonc"), scene).unwrap();
    dir
}

fn lines_warned(run: &Run, code: &str) -> Vec<usize> {
    warned(&run.stdout)
        .into_iter()
        .filter(|(_, c)| c == code)
        .map(|(line, _)| line)
        .collect()
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
fn lint_warns_on_the_nano_block_exits_zero_and_writes_nothing() {
    let repo = common::repo();
    let block = repo.join("blocks/nano-button-led");
    let before = files(&block);
    let run = orbe(&repo, &["lint", "blocks/nano-button-led/scene.jsonc"]);
    assert_eq!(files(&block), before, "lint wrote nothing");
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);

    let scene = fs::read_to_string(block.join("scene.jsonc")).unwrap();
    let at = |text: &str| line_of(&scene, text);
    let nano = at("\"nano\":");
    let want: Vec<(usize, String)> = [(at("{"), "scene.ground_unnamed"), (nano, "net.floating")]
        .into_iter()
        .map(|(l, c)| (l, c.into()))
        .collect();
    assert_eq!(warned(&run.stdout), want, "{}", run.stdout);
    assert!(
        run.stdout
            .ends_with("2 warnings in blocks/nano-button-led/scene.jsonc\n")
    );
    assert!(
        run.stdout
            .contains("add \"GND\": \"nano.GND\" to the scene's \"nets\"")
    );

    let envelope = json(&orbe(
        &repo,
        &["lint", "blocks/nano-button-led/scene.jsonc", "--json"],
    ));
    assert_eq!(envelope["ok"], true);
    let found = envelope["data"]["diagnostics"].as_array().unwrap();
    assert_eq!(found.len(), 2);
    for d in found {
        let keys: Vec<&str> = d.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["code", "column", "file", "hint", "line", "message"]);
        assert!(!d["hint"].as_str().unwrap().is_empty(), "{d}");
    }
    assert_eq!(found[0]["code"], "scene.ground_unnamed");
    assert_eq!(found[0]["file"], "blocks/nano-button-led/scene.jsonc");
}

/// The Uno's parts all have a model, a reason or `joins_only`, and its L LED has its resistor,
/// so its Block warns only of what the Nano Block also leaves as it is: the ground's name, and
/// AREF with nothing wired to it. The breadboard's empty holes warn of nothing.
#[test]
fn lint_on_the_uno_block_warns_only_as_the_nano_block_does() {
    let repo = common::repo();
    let run = orbe(&repo, &["lint", "blocks/uno-button-led/scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let codes: Vec<String> = warned(&run.stdout).into_iter().map(|(_, c)| c).collect();
    assert_eq!(
        codes,
        ["scene.ground_unnamed", "net.floating"],
        "{}",
        run.stdout
    );
}

/// A board runs its chip from its own supply, so one alone, blinking its L LED, is a whole
/// circuit. A resistor alone does nothing.
#[test]
fn a_board_alone_is_not_unconnected_and_a_resistor_alone_is() {
    for (part, ground, unconnected) in [
        (
            r#""type": "uno-board", "firmware": "build/blink.hex""#,
            "",
            false,
        ),
        (
            r#""type": "nano-board", "firmware": "build/blink.hex""#,
            "",
            false,
        ),
        (
            r#""type": "resistor-axial", "resistance": "10k""#,
            r#", "ground": "alone.a""#,
            true,
        ),
    ] {
        let dir = project(&format!(
            "{{ \"format\": 1, \"parts\": {{ \"alone\": {{ {part} }} }}{ground} }}\n"
        ));
        fs::copy(
            common::repo().join("packages/engine/tests/firmware/sketch/blink.hex"),
            dir.path().join("build/blink.hex"),
        )
        .unwrap();
        let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        assert_eq!(
            !lines_warned(&run, "scene.unconnected").is_empty(),
            unconnected,
            "{part}: {}",
            run.stdout
        );
    }
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
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "led1": { "type": "led-5mm", "color": "red" },
    "led2": { "type": "led-5mm", "color": "red" },
    "hdr": { "type": "pin-header-2x3" },
    "r9": { "type": "resistor-axial", "resistance": "10k" },
    "r8": { "type": "resistor-axial", "resistance": "10k" },
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.D13", "led1.anode"],
    ["led1.cathode", "nano.GND"],
    ["nano.D2", "r9.a"],
    ["r9.b", "nano.GND"],
    ["hdr.1", "nano.D3"],
    ["nano.D4", "r8.a"],
  ],
  "layout": {
    "nano": { "at": [0, 0, 0], "rotate": [0, 0, 270] },
    "led1": { "at": [0, 0, 0], "rotate": [-180, 0, 540] },
    "hdr": { "at": [0, 0, 0] },
    "r9": { "at": [0, 0, 0], "rotate": [0, 0, 90] },
    "r8": { "at": [0, 0, 0] },
  },
}
"#;
    let dir = project(scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let at = |text: &str| line_of(scene, text);
    let want = [
        (at("\"nano\": { \"type\""), "net.floating"),
        (at("\"led1\": { \"type\""), "scene.led_no_resistor"),
        (at("\"led2\""), "scene.unplaced"),
        (at("\"led2\""), "scene.unconnected"),
        (at("\"led2\""), "net.floating"),
        (at("\"led2\""), "net.floating"),
        (at("\"r8\": { \"type\""), "scene.unconnected"),
        (at("[0, 0, 270]"), "scene.rotation"),
        (at("[-180, 0, 540]"), "scene.rotation"),
    ];
    let want: Vec<(usize, String)> = want.into_iter().map(|(l, c)| (l, c.into())).collect();
    assert_eq!(warned(&run.stdout), want, "{}", run.stdout);
    for line in [
        "  hint: write \"rotate\": [0, 0, -90]",
        "  hint: write \"rotate\": [180, 0, 180]",
        "warning[scene.unconnected]: nothing is wired to `led2`",
        "warning[scene.unconnected]: `r8` has no wire on b",
        "warning[scene.unplaced]: `led2` has no position",
        "warning[scene.led_no_resistor]: nothing limits the current through `led1`: it joins `GND` and `nano__D13` with no resistor in series",
    ] {
        assert!(run.stdout.contains(line), "no `{line}` in\n{}", run.stdout);
    }
}

#[test]
fn a_part_with_few_nets_warns_of_an_open_one_counted_by_net_not_by_pin() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "half": { "type": "./four-legs.jsonc" },
    "whole": { "type": "./four-legs.jsonc" },
    "btn": { "type": "tactile-switch-6mm" },
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.D2", "half.a1"],
    ["nano.D3", "half.a2"],
    ["nano.D4", "whole.a1"],
    ["whole.b2", "nano.GND"],
    ["nano.D5", "btn.a"],
  ],
}
"#;
    let dir = project(scene);
    fs::write(
        dir.path().join("four-legs.jsonc"),
        r#"{
  "format": 1,
  "parts": { "sw": { "type": "tactile-switch-6mm" } },
  "pins": { "a1": "sw.a", "a2": "sw.a", "b1": "sw.b", "b2": "sw.b" },
}
"#,
    )
    .unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.unconnected"),
        [line_of(scene, "\"half\""), line_of(scene, "\"btn\"")],
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("warning[scene.unconnected]: `half` has no wire on b1\n"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_part_that_only_joins_wires_has_no_model_warning_and_each_hint_checks() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "hdr": { "type": "pin-header-2x3" },
    "row": { "type": "pin-header-1x15" },
    "r": { "type": "resistor-axial", "resistance": "220" },
    "led": { "type": "led-5mm", "color": "red" },
    "jump": { "type": "./jumper.jsonc" },
    "spare": { "type": "./jumper.jsonc" },
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.D13", "r.a"],
    ["r.b", "hdr.1"],
    ["hdr.2", "row.1"],
    ["row.2", "jump.a"],
    ["jump.b", "led.anode"],
    ["hdr.1", "hdr.2"],
    ["row.1", "row.2"],
    ["led.cathode", "nano.GND"],
  ],
}
"#;
    let dir = project(scene);
    let jumper =
        "{ \"format\": 1, \"pins\": { \"a\": [1], \"b\": [2] }, \"wires\": [[\"a\", \"b\"]] }\n";
    fs::write(dir.path().join("jumper.jsonc"), jumper).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.no_model"),
        [line_of(scene, "\"jump\""), line_of(scene, "\"spare\"")],
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains(
        "use a part with a model if `jump` should act in the circuit, or add \"joins_only\": true to its part file, ./jumper.jsonc, if it only joins wires\n"
    ), "{}", run.stdout);
    assert!(
        run.stdout
            .contains("add \"not_simulated\": \"<why>\" to `spare` to keep it for drawing only\n"),
        "{}",
        run.stdout
    );

    let joins = jumper.replace("\"format\": 1,", "\"format\": 1, \"joins_only\": true,");
    fs::write(dir.path().join("jumper.jsonc"), joins).unwrap();
    let marked = scene.replace(
        "\"spare\": { \"type\": \"./jumper.jsonc\" }",
        "\"spare\": { \"type\": \"./jumper.jsonc\", \"not_simulated\": \"a spare\" }",
    );
    fs::write(dir.path().join("scene.jsonc"), &marked).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.no_model"),
        [0; 0],
        "{}",
        run.stdout
    );
}

#[test]
fn an_led_fed_through_a_resistor_or_a_pot_stays_quiet_when_a_chip_pin_reads_its_node() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "from_5v": { "type": "led-5mm", "color": "red" },
    "r_5v": { "type": "resistor-axial", "resistance": "220" },
    "from_pin": { "type": "led-5mm", "color": "red" },
    "r_pin": { "type": "resistor-axial", "resistance": "220" },
    "from_pot": { "type": "led-5mm", "color": "red" },
    "pot": { "type": "potentiometer-wh148", "resistance": "10k" },
    "bare": { "type": "led-5mm", "color": "red" },
    "pulled": { "type": "led-5mm", "color": "red" },
    "r_down": { "type": "resistor-axial", "resistance": "10k" },
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.5V", "r_5v.a"],
    ["r_5v.b", "from_5v.anode"],
    ["from_5v.anode", "nano.A0"],
    ["from_5v.cathode", "nano.GND"],
    ["nano.D12", "r_pin.a"],
    ["r_pin.b", "from_pin.anode"],
    ["from_pin.anode", "nano.A1"],
    ["from_pin.cathode", "nano.GND"],
    ["nano.5V", "pot.a"],
    ["pot.b", "nano.GND"],
    ["pot.wiper", "from_pot.anode"],
    ["pot.wiper", "nano.A2"],
    ["from_pot.cathode", "nano.GND"],
    ["nano.D11", "bare.anode"],
    ["bare.cathode", "nano.GND"],
    ["nano.D10", "pulled.anode"],
    ["nano.D10", "r_down.a"],
    ["r_down.b", "nano.GND"],
    ["pulled.cathode", "nano.GND"],
  ],
}
"#;
    let dir = project(scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.led_no_resistor"),
        [line_of(scene, "\"bare\""), line_of(scene, "\"pulled\"")],
        "{}",
        run.stdout
    );
}

#[test]
fn an_led_on_a_transistor_switch_warns_with_no_collector_resistor() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "held_on": { "type": "led-5mm", "color": "red" },
    "q1": { "type": "npn-2n2222-to92" },
    "rb1": { "type": "resistor-axial", "resistance": "10k" },
    "on_pin": { "type": "led-5mm", "color": "red" },
    "q2": { "type": "npn-2n2222-to92" },
    "rb2": { "type": "resistor-axial", "resistance": "10k" },
    "limited": { "type": "led-5mm", "color": "red" },
    "rc3": { "type": "resistor-axial", "resistance": "220" },
    "q3": { "type": "npn-2n2222-to92" },
    "rb3": { "type": "resistor-axial", "resistance": "10k" },
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.5V", "held_on.anode"],
    ["held_on.cathode", "q1.collector"],
    ["q1.emitter", "nano.GND"],
    ["nano.5V", "rb1.a"],
    ["rb1.b", "q1.base"],
    ["nano.5V", "on_pin.anode"],
    ["on_pin.cathode", "q2.collector"],
    ["q2.emitter", "nano.GND"],
    ["nano.D13", "rb2.a"],
    ["rb2.b", "q2.base"],
    ["nano.5V", "rc3.a"],
    ["rc3.b", "limited.anode"],
    ["limited.cathode", "q3.collector"],
    ["q3.emitter", "nano.GND"],
    ["nano.D12", "rb3.a"],
    ["rb3.b", "q3.base"],
  ],
}
"#;
    let dir = project(scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.led_no_resistor"),
        [line_of(scene, "\"held_on\""), line_of(scene, "\"on_pin\"")],
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains(
        "nothing limits the current through `held_on`: it joins `GND` and `nano__5V` with no resistor in series"
    ), "{}", run.stdout);
}

#[test]
fn a_resistor_beside_an_led_limits_nothing_and_one_in_series_on_either_side_does() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
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
    assert_eq!(
        lines_warned(&run, "scene.led_no_resistor"),
        [line_of(scene, "\"beside\":")],
        "{}",
        run.stdout
    );
}

/// A resistor plugged in along a column has both legs in one strip, which shorts it. Across the
/// strips it does not. The breadboard's hundreds of empty holes warn of nothing either way.
#[test]
fn two_pins_of_a_part_in_one_breadboard_strip_warn_shorted() {
    for (rotate, shorted) in [("90", true), ("0", false)] {
        let scene = format!(
            r#"{{
  "format": 1,
  "parts": {{
    "bb": {{ "type": "breadboard-830" }},
    "nano": {{ "type": "nano-board", "firmware": "build/button.ino.hex" }},
    "r1": {{ "type": "resistor-axial", "resistance": "220" }},
  }},
  "nets": {{ "GND": "nano.GND" }},
  "layout": {{
    "bb": {{ "at": [0, 0, 0] }},
    "nano": {{ "on": {{ "D13": "bb.d1" }} }},
    "r1": {{ "on": {{ "a": "bb.a30" }}, "rotate": [0, 0, {rotate}] }},
  }},
  "wires": [["bb.c1", "bb.e30"], ["bb.e34", "bb.c14"], ["bb.b30", "bb.j40"], ["bb.a34", "bb.j41"]],
}}
"#
        );
        let dir = project(&scene);
        let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let codes: Vec<String> = warned(&run.stdout).into_iter().map(|(_, c)| c).collect();
        let want: &[&str] = if shorted {
            &["net.floating", "scene.shorted"]
        } else {
            &["net.floating"]
        };
        assert_eq!(codes, want, "rotate {rotate}: {}", run.stdout);
        if shorted {
            assert!(
                run.stdout.contains(
                    "warning[scene.shorted]: `r1.a` and `r1.b` sit in one strip, bb.a30 to bb.e30, so the strip shorts them"
                ) && run.stdout.contains("\"rotate\": [0, 0, 180]"),
                "{}",
                run.stdout
            );
            assert_eq!(
                lines_warned(&run, "scene.shorted"),
                [line_of(&scene, "\"r1\":")]
            );
        }
    }
}
