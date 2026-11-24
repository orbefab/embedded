//! `orbe lint`: the check's errors when a scene does not check, else warnings that never fail.

#[path = "../../../packages/scene/tests/common/mod.rs"]
mod common;
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "session/harness.rs"]
mod harness;
#[cfg(target_os = "linux")]
#[path = "registry_env.rs"]
mod registry_env;

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
    assert!(
        run.stdout
            .ends_with("blocks/nano-button-led/scene.jsonc: no warnings\n"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("warning["), "{}", run.stdout);

    let envelope = json(&orbe(
        &repo,
        &["lint", "blocks/nano-button-led/scene.jsonc", "--json"],
    ));
    assert_eq!(envelope["ok"], true);
    let found = envelope["data"]["diagnostics"].as_array().unwrap();
    assert!(found.is_empty(), "{found:?}");
}

/// Each Block names its ground and leaves AREF open, so lint is quiet. The breadboard's empty
/// holes warn of nothing.
#[test]
fn lint_on_the_uno_block_warns_only_as_the_nano_block_does() {
    let repo = common::repo();
    for block in [
        "devkitc-button-led",
        "devkitc-gy-bmp280",
        "nano-7seg-count",
        "nano-button-led",
        "nano-buzzer",
        "nano-lcd1602-i2c",
        "nano-led-fade",
        "nano-light-comparator",
        "nano-neopixel-stick",
        "nano-oled",
        "nano-relay-led",
        "nano-servo-sweep",
        "nano-shift-leds",
        "uno-button-led",
    ] {
        let run = orbe(&repo, &["lint", &format!("blocks/{block}/scene.jsonc")]);
        assert_eq!(run.code, 0, "{block}: {}{}", run.stdout, run.stderr);
        assert!(
            run.stdout
                .ends_with(&format!("blocks/{block}/scene.jsonc: no warnings\n")),
            "{block}: {}",
            run.stdout
        );
        assert!(!run.stdout.contains("warning["), "{block}: {}", run.stdout);
    }
}

/// A pin a part lists in `open` is left out of `net.floating` when no scene wire reaches it.
/// A wire from AREF to an open pin still warns, and so does a wire from AREF to a hole, and a
/// wire that joins two boards' AREF pins.
#[test]
fn a_board_net_with_no_scene_wire_is_quiet_and_a_wire_to_it_still_warns() {
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;
    let bare = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"nano\": {{ {nano} }} }}, \"nets\": {{ \"GND\": \"nano.GND\" }} }}\n"
    ));
    let run = orbe(bare.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("net.floating"), "{}", run.stdout);

    let uno = project(
        "{ \"format\": 1, \"parts\": { \"uno\": { \"type\": \"uno-board\", \"firmware\": \"build/button.ino.hex\" } }, \"nets\": { \"GND\": \"uno.GND\" } }\n",
    );
    let run = orbe(uno.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("net.floating"), "{}", run.stdout);

    let wired = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"nano\": {{ {nano} }}, \"r\": {{ \"type\": \"resistor-axial\", \"resistance\": \"10k\" }} }}, \"nets\": {{ \"GND\": \"nano.GND\" }}, \"wires\": [[\"nano.AREF\", \"r.a\"]] }}\n"
    ));
    let run = orbe(wired.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("warning[net.floating]: #net.nano__AREF has no DC path to ground"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("warning[net.floating]: #net.r__b has no DC path to ground"),
        "{}",
        run.stdout
    );

    let holed = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"bb\": {{ \"type\": \"breadboard-400\" }}, \"nano\": {{ {nano} }} }}, \"nets\": {{ \"GND\": \"nano.GND\" }}, \"layout\": {{ \"bb\": {{ \"at\": [0, 0, 0] }} }}, \"wires\": [[\"nano.AREF\", \"bb.a1\"]] }}\n"
    ));
    let run = orbe(holed.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("warning[net.floating]: #net.nano__AREF has no DC path to ground"),
        "{}",
        run.stdout
    );

    let shared = project(
        "{ \"format\": 1, \"parts\": { \"nano\": { \"type\": \"nano-board\", \"firmware\": \"build/button.ino.hex\" }, \"uno\": { \"type\": \"uno-board\", \"firmware\": \"build/button.ino.hex\" } }, \"nets\": { \"GND\": \"nano.GND\" }, \"wires\": [[\"nano.GND\", \"uno.GND\"], [\"nano.AREF\", \"uno.AREF\"]] }\n",
    );
    let run = orbe(shared.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("warning[net.floating]: #net.nano__AREF"),
        "{}",
        run.stdout
    );

    let apart = project(
        "{ \"format\": 1, \"parts\": { \"nano\": { \"type\": \"nano-board\", \"firmware\": \"build/button.ino.hex\" }, \"uno\": { \"type\": \"uno-board\", \"firmware\": \"build/button.ino.hex\" } }, \"nets\": { \"GND\": \"nano.GND\" }, \"wires\": [[\"nano.GND\", \"uno.GND\"]] }\n",
    );
    let run = orbe(apart.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("net.floating"), "{}", run.stdout);
}

/// The ground net is named `GND` when `nets` leaves that name free. A scene net that already
/// has `GND` keeps the flattened name. AREF stays quiet either way.
#[test]
fn the_ground_net_is_gnd_unless_a_scene_net_has_that_name() {
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;
    let plain = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"nano\": {{ {nano} }} }} }}\n"
    ));
    let run = orbe(plain.path(), &["check", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains(", ground GND\n"), "{}", run.stdout);
    let run = orbe(plain.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("net.floating"), "{}", run.stdout);

    let taken = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"nano\": {{ {nano} }} }}, \"nets\": {{ \"GND\": \"nano.D13\" }} }}\n"
    ));
    let run = orbe(taken.path(), &["check", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains(", ground nano__GND\n"),
        "{}",
        run.stdout
    );

    let named = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"nano\": {{ {nano} }} }}, \"nets\": {{ \"VGND\": \"nano.GND\" }} }}\n"
    ));
    let run = orbe(named.path(), &["check", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains(", ground VGND\n"), "{}", run.stdout);
}

/// A ground pin that is a hole or a lead does not lend the net its leaf name. `bb.a5`, `r1.b`
/// and `bb.X1` all leave the net `GND` while that name is free.
#[test]
fn a_ground_pointed_at_a_hole_or_a_lead_is_named_gnd() {
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;
    let scenes = [
        format!(
            r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }} }}, "wires": [["nano.GND", "bb.a5"]], "ground": "bb.a5" }}"#
        ),
        format!(
            r#"{{ "format": 1, "parts": {{ "nano": {{ {nano} }}, "r1": {{ "type": "resistor-axial", "resistance": "10k" }} }}, "ground": "r1.b" }}"#
        ),
        format!(
            r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }} }}, "wires": [["nano.GND", "bb.X1"]], "ground": "bb.X1" }}"#
        ),
    ];
    for scene in scenes {
        let dir = project(&scene);
        let run = orbe(dir.path(), &["check", "scene.jsonc"]);
        assert_eq!(run.code, 0, "{}{}\n{scene}", run.stdout, run.stderr);
        assert!(
            run.stdout.contains(", ground GND\n"),
            "{scene}\n{}",
            run.stdout
        );
    }
}

/// A part whose `at` puts a leg in a breadboard hole, with no `on`, warns that the leg joins nothing.
/// One off the board, one above the holes, one seated, and one over a board that has no `at`
/// stay quiet.
#[test]
fn a_part_drawn_over_a_hole_warns_until_it_is_seated() {
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;
    let resistor = r#""type": "resistor-axial", "resistance": "10k""#;
    let scene = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }}, "r_far": {{ {resistor} }}, "r_high": {{ {resistor} }}, "r_on": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, 8.5] }}, "r_far": {{ "at": [200, 0, 8.5] }}, "r_high": {{ "at": [-31.75, -13.97, 100] }}, "r_on": {{ "on": {{ "a": "bb.a10" }} }} }} }}"#
    );
    let dir = project(&scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains(
            "warning[scene.unplaced]: `r1.a` is drawn in `bb.a1` and joins nothing there"
        ),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("hint: seat it with \"on\", as \"r1\": { \"on\": { \"a\": \"bb.a1\" } }"),
        "{}",
        run.stdout
    );
    let drawn: Vec<_> = run
        .stdout
        .lines()
        .filter(|line| line.contains("is drawn in"))
        .collect();
    assert_eq!(drawn.len(), 1, "{}", run.stdout);
    assert!(
        drawn[0].ends_with(
            "warning[scene.unplaced]: `r1.a` is drawn in `bb.a1` and joins nothing there"
        ),
        "{}",
        run.stdout
    );
    let envelope = json(&orbe(dir.path(), &["lint", "scene.jsonc", "--json"]));
    let codes: Vec<&str> = envelope["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["message"].as_str().unwrap().contains("drawn in"))
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["scene.unplaced"]);

    let loose = project(&format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "layout": {{ "nano": {{ "at": [0, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, 8.5] }} }} }}"#
    ));
    let run = orbe(loose.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("drawn in"), "{}", run.stdout);
}

/// A non-zero `rotate` is part of the hint. Without it, seating `a` on `a5` puts `b` in `a9`.
#[test]
fn an_unseated_leg_hint_keeps_a_nonzero_rotate() {
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;
    let resistor = r#""type": "resistor-axial", "resistance": "10k""#;
    let scene = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, 8.5], "rotate": [0, 0, 180] }} }} }}"#
    );
    let dir = project(&scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let hint = "hint: seat it with \"on\", as \"r1\": { \"on\": { \"a\": \"bb.a5\" }, \"rotate\": [0, 0, 180] }";
    assert!(run.stdout.contains(hint), "{}", run.stdout);

    let seated = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "on": {{ "a": "bb.a5" }}, "rotate": [0, 0, 180] }} }} }}"#
    );
    let dir = project(&seated);
    let run = orbe(dir.path(), &["nets", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let row = |pin: &str| {
        run.stdout
            .lines()
            .find(|line| line.split_whitespace().any(|word| word == pin))
            .unwrap_or_else(|| panic!("no net holds {pin}\n{}", run.stdout))
    };
    assert!(row("r1.a").contains("bb.a5"), "{}", run.stdout);
    assert!(row("r1.b").contains("bb.a1"), "{}", run.stdout);
    assert!(!row("r1.b").contains("bb.a9"), "{}", run.stdout);

    let flat = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, 8.5], "rotate": [0, 0, 0] }} }} }}"#
    );
    let dir = project(&flat);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("hint: seat it with \"on\", as \"r1\": { \"on\": { \"a\": \"bb.a1\" } }"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("rotate"), "{}", run.stdout);
}

/// A leg already on the hole's net is not unseated. The part's other leg, lined up with
/// another hole, still is. Both legs on their holes' nets stay quiet.
#[test]
fn a_leg_already_on_the_holes_net_is_not_an_unseated_leg() {
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;
    let resistor = r#""type": "resistor-axial", "resistance": "10k""#;
    let scene = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "wires": [["r1.a", "bb.a1", {{ "kind": "contact" }}]], "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, 8.5] }} }} }}"#
    );
    let dir = project(&scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains(
            "warning[scene.unplaced]: `r1.b` is drawn in `bb.a5` and joins nothing there"
        ),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(
            "hint: join it with a contact wire, as [\"r1.b\", \"bb.a5\", { \"kind\": \"contact\" }]"
        ),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("\"on\""), "{}", run.stdout);

    let joined = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "wires": [["r1.a", "bb.a1", {{ "kind": "contact" }}], ["r1.b", "bb.a5", {{ "kind": "contact" }}]], "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, 8.5] }} }} }}"#
    );
    let dir = project(&joined);
    let run = orbe(dir.path(), &["check", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);

    let seated = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "wires": [["r1.a", "bb.a1", {{ "kind": "contact" }}]], "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "on": {{ "b": "bb.a5" }} }} }} }}"#
    );
    let dir = project(&seated);
    let run = orbe(dir.path(), &["check", "scene.jsonc"]);
    assert_ne!(
        run.code, 0,
        "seating b into a5 also claims a1\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("scene.layout") && run.stdout.contains("bb.a1"),
        "{}",
        run.stdout
    );

    let both = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "wires": [["r1.a", "bb.a1", {{ "kind": "contact" }}], ["r1.b", "bb.a5", {{ "kind": "contact" }}]], "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, 8.5] }} }} }}"#
    );
    let dir = project(&both);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        !run.stdout.contains("joins nothing there"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("drawn in"), "{}", run.stdout);

    let elsewhere = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ {resistor} }} }}, "nets": {{ "GND": "nano.GND" }}, "wires": [["r1.a", "bb.a30", {{ "kind": "contact" }}]], "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, 8.5] }} }} }}"#
    );
    let dir = project(&elsewhere);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("`r1.a` is drawn in `bb.a1` and joins nothing there"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("hint: seat it with \"on\", as \"r1\": { \"on\": { \"a\": \"bb.a1\" } }"),
        "{}",
        run.stdout
    );
}

/// The contact wire in the first hint for a leg drawn in a hole it does not join.
fn drawn_contact(stdout: &str) -> Option<String> {
    let mut want = false;
    for line in stdout.lines() {
        if want {
            let hint = line.trim().strip_prefix("hint:").unwrap().trim();
            let wire = hint
                .strip_prefix("join it with a contact wire, as ")
                .unwrap_or_else(|| {
                    panic!("a drawn leg's hint seats a held hole: {hint}\n{stdout}")
                });
            return Some(wire.to_string());
        }
        want = line.contains("joins nothing there");
    }
    None
}

/// A hole a jumper or another part already holds is not where the hint seats the leg. The hint
/// is a contact wire to a free hole in that strip, and each hint in turn leaves a scene that
/// `check` accepts.
#[test]
fn a_held_hole_hints_a_free_hole_and_following_the_hints_checks() {
    let nano = serde_json::json!({
        "type": "nano-board",
        "firmware": "build/button.ino.hex"
    });
    let resistor = serde_json::json!({ "type": "resistor-axial", "resistance": "10k" });
    let drawn = serde_json::json!({ "at": [-31.75, -13.97, 8.5] });
    let mut base = serde_json::json!({
        "format": 1,
        "parts": {
            "bb": { "type": "breadboard-400" },
            "nano": nano,
            "r1": resistor.clone()
        },
        "nets": { "GND": "nano.GND" },
        "layout": {
            "bb": { "at": [0, 0, 0] },
            "nano": { "at": [200, 0, 0] },
            "r1": drawn
        }
    });
    let contact =
        |leg: &str, hole: &str| format!("[\"{leg}\", \"{hole}\", {{ \"kind\": \"contact\" }}]");
    let jumper_a5 = serde_json::json!(["bb.a5", "nano.D2"]);
    let jumper_a1 = serde_json::json!(["bb.a1", "nano.D2"]);
    let seated = base.clone();
    base["wires"] = serde_json::json!([["r1.a", "bb.a1", { "kind": "contact" }], jumper_a5]);
    let held_by_jumper_at_the_other_leg = base.clone();
    base["wires"] = serde_json::json!([jumper_a1]);
    let held_by_a_jumper = base;
    let mut held_by_a_leg = seated;
    held_by_a_leg["parts"]["r2"] = resistor;
    held_by_a_leg["layout"]["r2"] = serde_json::json!({ "on": { "a": "bb.a1" } });
    let cases = [
        (
            held_by_jumper_at_the_other_leg,
            "`r1.b` is drawn in `bb.a5` and joins nothing there",
            contact("r1.b", "bb.b5"),
            vec![contact("r1.b", "bb.b5")],
        ),
        (
            held_by_a_jumper,
            "`r1.a` is drawn in `bb.a1` and joins nothing there",
            contact("r1.a", "bb.b1"),
            vec![contact("r1.a", "bb.b1"), contact("r1.b", "bb.a5")],
        ),
        (
            held_by_a_leg,
            "`r1.a` is drawn in `bb.a1` and joins nothing there",
            contact("r1.a", "bb.b1"),
            vec![contact("r1.a", "bb.b1"), contact("r1.b", "bb.b5")],
        ),
    ];
    for (mut scene, warning, hint, wires) in cases {
        let dir = project(&serde_json::to_string(&scene).unwrap());
        let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        assert!(
            run.stdout
                .contains(&format!("warning[scene.unplaced]: {warning}")),
            "{}",
            run.stdout
        );
        assert!(
            run.stdout
                .contains(&format!("hint: join it with a contact wire, as {hint}")),
            "{}",
            run.stdout
        );
        let mut applied = Vec::new();
        for _ in 0..8 {
            let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
            assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
            let Some(wire) = drawn_contact(&run.stdout) else {
                break;
            };
            applied.push(wire.clone());
            let wires = scene
                .as_object_mut()
                .unwrap()
                .entry("wires")
                .or_insert_with(|| serde_json::json!([]));
            wires
                .as_array_mut()
                .unwrap()
                .push(serde_json::from_str(&wire).unwrap());
            fs::write(
                dir.path().join("scene.jsonc"),
                serde_json::to_string(&scene).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(applied, wires, "{}", warning);
        let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
        assert!(
            !run.stdout.contains("joins nothing there"),
            "{}",
            run.stdout
        );
        let run = orbe(dir.path(), &["check", "scene.jsonc"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    }
}

/// A part whose legs are further below the holes than the board is thick is not drawn in them.
#[test]
fn a_part_below_the_board_is_not_an_unseated_leg() {
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;
    let scene = format!(
        r#"{{ "format": 1, "parts": {{ "bb": {{ "type": "breadboard-400" }}, "nano": {{ {nano} }}, "r1": {{ "type": "resistor-axial", "resistance": "10k" }} }}, "nets": {{ "GND": "nano.GND" }}, "layout": {{ "bb": {{ "at": [0, 0, 0] }}, "nano": {{ "at": [200, 0, 0] }}, "r1": {{ "at": [-31.75, -13.97, -30] }} }} }}"#
    );
    let dir = project(&scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("drawn in"), "{}", run.stdout);
    assert!(
        !run.stdout.contains("joins nothing there"),
        "{}",
        run.stdout
    );
}

/// A hole that only seats an open pin, with nothing else in its strip, does not reach the net.
/// Another part in that strip does.
#[test]
fn a_hole_that_only_seats_an_open_pin_is_quiet_and_another_part_in_the_strip_warns() {
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;
    let seated = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"bb\": {{ \"type\": \"breadboard-400\" }}, \"nano\": {{ {nano} }} }}, \"nets\": {{ \"GND\": \"nano.GND\" }}, \"layout\": {{ \"bb\": {{ \"at\": [0, 0, 0] }}, \"nano\": {{ \"on\": {{ \"D13\": \"bb.d1\" }} }} }} }}\n"
    ));
    let run = orbe(seated.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.ends_with("scene.jsonc: no warnings\n"),
        "{}",
        run.stdout
    );

    let shared = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"bb\": {{ \"type\": \"breadboard-400\" }}, \"nano\": {{ {nano} }}, \"r\": {{ \"type\": \"resistor-axial\", \"resistance\": \"10k\" }} }}, \"nets\": {{ \"GND\": \"nano.GND\" }}, \"layout\": {{ \"bb\": {{ \"at\": [0, 0, 0] }}, \"nano\": {{ \"on\": {{ \"D13\": \"bb.d1\" }} }}, \"r\": {{ \"on\": {{ \"a\": \"bb.b3\" }}, \"rotate\": [0, 0, -90] }} }} }}\n"
    ));
    let run = orbe(shared.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("warning[net.floating]: #net.nano__AREF has no DC path to ground"),
        "{}",
        run.stdout
    );
}

/// A homemade board that holds a USB source and an ATmega, and leaves `AVCC` open, still warns.
/// Listing `AREF` in `open` quiets AREF only.
#[test]
fn an_open_avcc_inside_a_board_warns_and_a_listed_aref_does_not() {
    let dir = project(
        "{ \"format\": 1, \"parts\": { \"board\": { \"type\": \"./home.jsonc\", \"firmware\": \"build/button.ino.hex\" } }, \"nets\": { \"GND\": \"board.GND\" } }\n",
    );
    fs::write(dir.path().join("home.jsonc"), home_part(false)).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("warning[net.floating]:") && run.stdout.contains("AVCC"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("AREF"), "{}", run.stdout);

    fs::write(dir.path().join("home.jsonc"), home_part(true)).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("warning[net.floating]:") && run.stdout.contains("AVCC"),
        "{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("AREF"),
        "AREF is listed in open: {}",
        run.stdout
    );
}

/// An unwired op-amp input inside a part that also holds a source and a chip still warns.
#[test]
fn an_unwired_op_amp_input_inside_a_part_warns() {
    let dir = project(
        "{ \"format\": 1, \"parts\": { \"board\": { \"type\": \"./home.jsonc\", \"firmware\": \"build/button.ino.hex\" } }, \"nets\": { \"GND\": \"board.GND\" } }\n",
    );
    fs::write(dir.path().join("home.jsonc"), amp_part()).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    for pin in ["IN1_P", "IN2_P", "IN2_N"] {
        assert!(
            run.stdout.contains("warning[net.floating]:") && run.stdout.contains(pin),
            "{pin}: {}",
            run.stdout
        );
    }
}

fn home_part(open_aref: bool) -> String {
    let open = if open_aref {
        "\n  \"open\": [\"AREF\"],"
    } else {
        ""
    };
    format!(
        r#"{{
  "format": 1,
  "params": {{ "firmware": {{}} }},
  "ground": "usb.GND",
  "parts": {{
    "usb": {{ "type": "usb-micro-b" }},
    "mcu": {{ "type": "atmega328p-dip28", "firmware": {{ "param": "firmware" }} }},
    "hi": {{ "type": "resistor-axial", "resistance": "10k" }},
    "lo": {{ "type": "resistor-axial", "resistance": "10k" }}
  }},
  "wires": [
    ["usb.VBUS", "mcu.VCC"],
    ["usb.GND", "mcu.GND"],
    ["usb.VBUS", "hi.a"],
    ["hi.b", "mcu.PC0"],
    ["mcu.PC0", "lo.a"],
    ["lo.b", "usb.GND"]
  ],
  "pins": {{ "GND": "usb.GND", "AREF": "mcu.AREF" }},{open}
}}
"#
    )
}

fn amp_part() -> String {
    r#"{
  "format": 1,
  "params": { "firmware": {} },
  "ground": "usb.GND",
  "parts": {
    "usb": { "type": "usb-micro-b" },
    "mcu": { "type": "atmega328p-dip28", "firmware": { "param": "firmware" } },
    "amp": { "type": "lm358-dip8" }
  },
  "wires": [
    ["usb.GND", "mcu.GND"],
    ["usb.GND", "amp.GND"],
    ["usb.VBUS", "mcu.VCC"],
    ["usb.VBUS", "mcu.AVCC"],
    ["usb.VBUS", "amp.VCC"],
    ["amp.IN1_N", "amp.OUT1"],
    ["amp.OUT1", "mcu.PC0"]
  ],
  "pins": { "GND": "usb.GND" }
}
"#
    .to_string()
}

/// `orbe open` reports the same `net.floating` nets lint does: an open `AVCC` warns, a bare
/// Nano's AREF does not, and a wire to AREF warns.
#[cfg(target_os = "linux")]
#[test]
fn open_reports_the_same_floating_nets_lint_does() {
    let runtime = harness::Runtime::new();
    let nano = r#""type": "nano-board", "firmware": "build/button.ino.hex""#;

    let bare = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"nano\": {{ {nano} }} }}, \"nets\": {{ \"GND\": \"nano.GND\" }} }}\n"
    ));
    let text = opened(&runtime, bare.path());
    assert!(!text.contains("net.floating"), "{text}");

    let wired = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"nano\": {{ {nano} }}, \"r\": {{ \"type\": \"resistor-axial\", \"resistance\": \"10k\" }} }}, \"nets\": {{ \"GND\": \"nano.GND\" }}, \"wires\": [[\"nano.AREF\", \"r.a\"]] }}\n"
    ));
    let text = opened(&runtime, wired.path());
    assert!(
        text.contains("warning[net.floating]") && text.contains("nano__AREF"),
        "{text}"
    );

    let seated = project(&format!(
        "{{ \"format\": 1, \"parts\": {{ \"bb\": {{ \"type\": \"breadboard-400\" }}, \"nano\": {{ {nano} }} }}, \"nets\": {{ \"GND\": \"nano.GND\" }}, \"layout\": {{ \"bb\": {{ \"at\": [0, 0, 0] }}, \"nano\": {{ \"on\": {{ \"D13\": \"bb.d1\" }} }} }} }}\n"
    ));
    let text = opened(&runtime, seated.path());
    assert!(!text.contains("net.floating"), "{text}");

    let board = project(
        "{ \"format\": 1, \"parts\": { \"board\": { \"type\": \"./home.jsonc\", \"firmware\": \"build/button.ino.hex\" } }, \"nets\": { \"GND\": \"board.GND\" } }\n",
    );
    fs::write(board.path().join("home.jsonc"), home_part(true)).unwrap();
    let text = opened(&runtime, board.path());
    assert!(
        text.contains("warning[net.floating]") && text.contains("AVCC"),
        "{text}"
    );
    assert!(!text.contains("AREF"), "{text}");
}

#[cfg(target_os = "linux")]
fn opened(runtime: &harness::Runtime, dir: &Path) -> String {
    let run = runtime.orbe(dir, &["open", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let closed = runtime.orbe(dir, &["close"]);
    assert_eq!(closed.code, 0, "{}{}", closed.stdout, closed.stderr);
    run.stdout
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
        "error[io_error]: no scene file at nope.jsonc\n  hint: write the scene, or name its path\n"
    );
}

#[test]
fn each_rule_warns_where_the_scene_goes_wrong() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "led1": { "type": "led-5mm" },
    "led2": { "type": "led-5mm" },
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
    "led1": { "at": [40, 0, 0], "rotate": [-180, 0, 540] },
    "hdr": { "at": [80, 0, 0] },
    "r9": { "at": [120, 0, 0], "rotate": [0, 0, 90] },
    "r8": { "at": [160, 0, 0] },
  },
}
"#;
    let dir = project(scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let at = |text: &str| line_of(scene, text);
    let want = [
        (at("\"led1\": { \"type\""), "scene.led_no_resistor"),
        (at("\"led2\""), "scene.unplaced"),
        (at("\"led2\""), "scene.unconnected"),
        (at("\"led2\""), "net.floating"),
        (at("\"led2\""), "net.floating"),
        (at("\"r8\": { \"type\""), "scene.unconnected"),
    ];
    let want: Vec<(usize, String)> = want.into_iter().map(|(l, c)| (l, c.into())).collect();
    assert_eq!(warned(&run.stdout), want, "{}", run.stdout);
    for line in [
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
    "led": { "type": "led-5mm" },
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
    "from_5v": { "type": "led-5mm" },
    "r_5v": { "type": "resistor-axial", "resistance": "220" },
    "from_pin": { "type": "led-5mm" },
    "r_pin": { "type": "resistor-axial", "resistance": "220" },
    "from_pot": { "type": "led-5mm" },
    "pot": { "type": "potentiometer-wh148", "resistance": "10k" },
    "bare": { "type": "led-5mm" },
    "pulled": { "type": "led-5mm" },
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
    "held_on": { "type": "led-5mm" },
    "q1": { "type": "npn-2n2222-to92" },
    "rb1": { "type": "resistor-axial", "resistance": "10k" },
    "on_pin": { "type": "led-5mm" },
    "q2": { "type": "npn-2n2222-to92" },
    "rb2": { "type": "resistor-axial", "resistance": "10k" },
    "limited": { "type": "led-5mm" },
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
    "beside": { "type": "led-5mm" },
    "r_beside": { "type": "resistor-axial", "resistance": "1k" },
    "above": { "type": "led-5mm" },
    "r_above": { "type": "resistor-axial", "resistance": "220" },
    "below": { "type": "led-5mm" },
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

/// The module's indicator: 1 kohm from VCC to the LED, and a diode from VCC to the LED's
/// cathode beside the coil. The resistor is in series. The same LED with no resistor warns.
#[test]
fn a_diode_back_to_the_supply_does_not_hide_a_resistor_in_series_with_the_led() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "series": { "type": "led-5mm" },
    "rled": { "type": "resistor-axial", "resistance": "1k" },
    "d1": { "type": "diode-1n4007" },
    "k1": { "type": "relay-srd-05vdc-sl-c" },
    "q1": { "type": "npn-2n2222-to92" },
    "rb1": { "type": "resistor-axial", "resistance": "1k" },
    "bare": { "type": "led-5mm" },
    "d2": { "type": "diode-1n4007" },
    "k2": { "type": "relay-srd-05vdc-sl-c" },
    "q2": { "type": "npn-2n2222-to92" },
    "rb2": { "type": "resistor-axial", "resistance": "1k" },
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.5V", "rled.a"],
    ["rled.b", "series.anode"],
    ["series.cathode", "q1.collector"],
    ["d1.cathode", "nano.5V"],
    ["d1.anode", "q1.collector"],
    ["k1.coil_a", "nano.5V"],
    ["k1.coil_b", "q1.collector"],
    ["q1.emitter", "nano.GND"],
    ["nano.D13", "rb1.a"],
    ["rb1.b", "q1.base"],
    ["nano.5V", "bare.anode"],
    ["bare.cathode", "q2.collector"],
    ["d2.cathode", "nano.5V"],
    ["d2.anode", "q2.collector"],
    ["k2.coil_a", "nano.5V"],
    ["k2.coil_b", "q2.collector"],
    ["q2.emitter", "nano.GND"],
    ["nano.D12", "rb2.a"],
    ["rb2.b", "q2.base"],
  ],
}
"#;
    let dir = project(scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.led_no_resistor"),
        [line_of(scene, "\"bare\":")],
        "{}",
        run.stdout
    );
}

/// The contact conducts like a switch. The shipped Block keeps 220 ohm in series and stays
/// quiet. That Block with the resistor removed, and a bare SRD from 5 V on COM into an LED
/// through NO or through NC, warn.
#[test]
fn a_relay_contact_into_an_led_warns_without_a_series_resistor() {
    let repo = common::repo();
    let shipped = orbe(&repo, &["lint", "blocks/nano-relay-led/scene.jsonc"]);
    assert_eq!(shipped.code, 0, "{}{}", shipped.stdout, shipped.stderr);
    assert!(
        shipped
            .stdout
            .ends_with("blocks/nano-relay-led/scene.jsonc: no warnings\n"),
        "{}",
        shipped.stdout
    );

    let scene = fs::read_to_string(repo.join("blocks/nano-relay-led/scene.jsonc")).unwrap();
    let dropped = scene
        .lines()
        .filter(|line| !line.contains("\"r1\":"))
        .map(|line| {
            line.replace(
                "[\"relay.NO\", \"r1.a\"],",
                "[\"relay.NO\", \"led1.anode\"],",
            )
        })
        .filter(|line| !line.contains("r1.b"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("build/relay.ino.hex", "build/button.ino.hex");
    assert!(
        !dropped.contains("r1") && dropped.contains("[\"relay.NO\", \"led1.anode\"]"),
        "{dropped}"
    );
    let dir = project(&dropped);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.led_no_resistor"),
        [line_of(&dropped, "\"led1\":")],
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(
            "nothing limits the current through `led1`: it joins `GND` and `nano__5V` with no resistor in series"
        ),
        "{}",
        run.stdout
    );

    for (supply, load) in [("COM", "NO"), ("COM", "NC"), ("NO", "COM")] {
        let bare = format!(
            r#"{{
  "format": 1,
  "parts": {{
    "nano": {{ "type": "nano-board", "firmware": "build/button.ino.hex" }},
    "k": {{ "type": "relay-srd-05vdc-sl-c" }},
    "led1": {{ "type": "led-5mm" }}
  }},
  "nets": {{ "GND": "nano.GND" }},
  "wires": [
    ["nano.5V", "k.{supply}"],
    ["k.{load}", "led1.anode"],
    ["led1.cathode", "nano.GND"],
    ["nano.5V", "k.coil_a"],
    ["k.coil_b", "nano.GND"]
  ],
  "layout": {{
    "nano": {{ "at": [0, 0, 0] }},
    "k": {{ "at": [40, 0, 0] }},
    "led1": {{ "at": [80, 0, 0] }}
  }}
}}
"#
        );
        let dir = project(&bare);
        let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
        assert_eq!(
            run.code, 0,
            "{supply}->{load}: {}{}",
            run.stdout, run.stderr
        );
        assert_eq!(
            lines_warned(&run, "scene.led_no_resistor"),
            [line_of(&bare, "\"led1\":")],
            "{supply}->{load}: {}",
            run.stdout
        );
        assert!(
            run.stdout.contains(
                "nothing limits the current through `led1`: it joins `GND` and `nano__5V` with no resistor in series"
            ),
            "{supply}->{load}: {}",
            run.stdout
        );
    }
}

/// 5 V on NO, the LED on NC, COM left open. The coil is tied so its nets are not the
/// question. The two throws never conduct through COM, so the LED is not across the
/// supply and lint stays quiet.
#[test]
fn an_led_on_the_other_relay_contact_from_the_supply_stays_quiet() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "k": { "type": "relay-srd-05vdc-sl-c" },
    "led1": { "type": "led-5mm" }
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.5V", "k.NO"],
    ["k.NC", "led1.anode"],
    ["led1.cathode", "nano.GND"],
    ["nano.5V", "k.coil_a"],
    ["k.coil_b", "nano.GND"]
  ],
  "layout": {
    "nano": { "at": [0, 0, 0] },
    "k": { "at": [40, 0, 0] },
    "led1": { "at": [80, 0, 0] }
  }
}
"#;
    let dir = project(scene);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.ends_with("scene.jsonc: no warnings\n"),
        "{}",
        run.stdout
    );
}

/// Twenty-four relays with their contacts tied to limited LEDs lint in polynomial time,
/// finishing well under a second even in a debug build.
#[test]
#[expect(clippy::disallowed_methods, reason = "the clock is the measurement")]
fn lint_stays_fast_with_twenty_four_relays() {
    let mut parts = vec![
        r#"    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" }"#.to_string(),
    ];
    let mut wires = Vec::new();
    let mut layout = vec![r#"    "nano": { "at": [0, 0, 0] }"#.to_string()];
    for i in 0..24 {
        let x = 40 * (i + 1);
        parts.push(format!(
            r#"    "k{i}": {{ "type": "relay-srd-05vdc-sl-c" }}"#
        ));
        parts.push(format!(
            r#"    "r{i}": {{ "type": "resistor-axial", "resistance": "220" }}"#
        ));
        parts.push(format!(r#"    "led{i}": {{ "type": "led-5mm" }}"#));
        wires.push(format!(r#"    ["nano.5V", "k{i}.coil_a"]"#));
        wires.push(format!(r#"    ["k{i}.coil_b", "nano.GND"]"#));
        wires.push(format!(r#"    ["nano.5V", "k{i}.COM"]"#));
        wires.push(format!(r#"    ["k{i}.NO", "r{i}.a"]"#));
        wires.push(format!(r#"    ["k{i}.NC", "r{i}.a"]"#));
        wires.push(format!(r#"    ["r{i}.b", "led{i}.anode"]"#));
        wires.push(format!(r#"    ["led{i}.cathode", "nano.GND"]"#));
        layout.push(format!(r#"    "k{i}": {{ "at": [{x}, 0, 0] }}"#));
        layout.push(format!(r#"    "r{i}": {{ "at": [{x}, 30, 0] }}"#));
        layout.push(format!(r#"    "led{i}": {{ "at": [{x}, 60, 0] }}"#));
    }
    let scene = format!(
        "{{\n  \"format\": 1,\n  \"parts\": {{\n{}\n  }},\n  \"nets\": {{ \"GND\": \"nano.GND\" }},\n  \"wires\": [\n{}\n  ],\n  \"layout\": {{\n{}\n  }}\n}}\n",
        parts.join(",\n"),
        wires.join(",\n"),
        layout.join(",\n"),
    );
    let dir = project(&scene);
    let start = std::time::Instant::now();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    let elapsed = start.elapsed();
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.ends_with("scene.jsonc: no warnings\n"),
        "{}",
        run.stdout
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "24 relays took {elapsed:?}"
    );
}

/// A 2N2222 with its leads formed to 2.54 mm sits one leg in each strip. Turned along a column,
/// the three legs share a strip and short. The Nano drives the base, and 220 ohm limits the LED.
#[test]
fn a_2n2222_formed_to_the_breadboard_pitch_seats_one_leg_per_strip() {
    for (rotate, shorted) in [("0", false), ("90", true)] {
        let scene = format!(
            r#"{{
  "format": 1,
  "parts": {{
    "bb": {{ "type": "breadboard-400" }},
    "nano": {{ "type": "nano-board", "firmware": "build/button.ino.hex" }},
    "q1": {{ "type": "npn-2n2222-to92" }},
    "rb": {{ "type": "resistor-axial", "resistance": "1k" }},
    "rc": {{ "type": "resistor-axial", "resistance": "220" }},
    "led": {{ "type": "led-5mm" }},
  }},
  "nets": {{ "GND": "nano.GND" }},
  "layout": {{
    "bb": {{ "at": [0, 0, 0] }},
    "nano": {{ "on": {{ "D13": "bb.d1" }} }},
    "q1": {{ "on": {{ "collector": "bb.c21" }}, "rotate": [0, 0, {rotate}] }},
    "rb": {{ "on": {{ "a": "bb.a22" }} }},
    "rc": {{ "on": {{ "a": "bb.c12" }} }},
    "led": {{ "on": {{ "anode": "bb.b16" }} }},
  }},
  "wires": [
    ["bb.c1", "bb.b26"],
    ["bb.a17", "bb.b21"],
    ["bb.b23", "bb.X23"],
    ["bb.a14", "bb.X14"],
  ],
}}
"#
        );
        let dir = project(&scene);
        let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let codes: Vec<String> = warned(&run.stdout).into_iter().map(|(_, c)| c).collect();
        let want: &[&str] = if shorted { &["scene.shorted"] } else { &[] };
        assert_eq!(codes, want, "rotate {rotate}: {}", run.stdout);
    }
}

/// A resistor plugged in along a column has both legs in one strip, which shorts it. Across the
/// strips it does not. The breadboard's hundreds of empty holes warn of nothing either way. The
/// jumpers use a free hole of each strip: along the row `r1.b` is `a34`, and along the column it
/// is `e30`.
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
  "wires": [["bb.c1", "bb.d30"], ["bb.e34", "bb.c14"], ["bb.b30", "bb.j40"], ["bb.b34", "bb.j41"]],
}}
"#
        );
        let dir = project(&scene);
        let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let codes: Vec<String> = warned(&run.stdout).into_iter().map(|(_, c)| c).collect();
        let want: &[&str] = if shorted {
            &["scene.shorted", "scene.covered_hole", "scene.covered_hole"]
        } else {
            &[]
        };
        assert_eq!(codes, want, "rotate {rotate}: {}", run.stdout);
        if shorted {
            assert!(
                run.stdout.contains(
                    "warning[scene.shorted]: `r1.a` and `r1.b` sit in one strip, bb.a30 to bb.e30, so the strip shorts them"
                ) && run.stdout.contains("\"rotate\": [0, 0, 0], and every leg lands in a hole"),
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

/// A DHT22 seated at `bb.c20` covers `bb.b21` with its body, so a jumper `["bb.j9", "bb.b21"]`
/// cannot reach that hole on a real breadboard. Moving the jumper to the clear hole `bb.e21`
/// silences the warning. A seated part's lead under the DHT22's body warns too.
#[test]
fn a_wire_or_lead_in_a_covered_breadboard_hole_warns() {
    let scene_with = |wire_end: &str, extra_part: &str, extra_layout: &str| {
        format!(
            r##"{{
  "format": 1,
  "parts": {{
    "bb": {{ "type": "breadboard-830" }},
    "nano": {{ "type": "nano-board", "firmware": "build/button.ino.hex" }},
    "dht": {{ "type": "dht22" }},
    "rpull": {{ "type": "resistor-axial", "resistance": "5.1k" }}{extra_part}
  }},
  "layout": {{
    "bb": {{ "at": [0, 0, 0] }},
    "nano": {{ "on": {{ "D13": "bb.d1" }} }},
    "dht": {{ "on": {{ "VDD": "bb.c20" }} }},
    "rpull": {{ "on": {{ "a": "bb.a21" }}, "rotate": [0, 0, -90] }}{extra_layout}
  }},
  "nets": {{ "GND": "nano.GND" }},
  "wires": [
    ["bb.a12", "bb.W11", {{ "color": "#d12b2b" }}],
    ["bb.a14", "bb.X13", {{ "color": "#161616" }}],
    ["bb.a20", "bb.W19", {{ "color": "#d12b2b" }}],
    ["bb.j9", "{wire_end}"],
    ["bb.a23", "bb.X23", {{ "color": "#161616" }}]
  ]
}}
"##
        )
    };

    let covered = scene_with("bb.b21", "", "");
    let dir = project(&covered);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.covered_hole"),
        [line_of(&covered, "bb.b21")],
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(
            "warning[scene.covered_hole]: wire end `bb.b21` sits under `dht`'s body, so a jumper cannot reach that hole"
        ),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(
            "hint: move the wire end from `bb.b21` to a clear hole in the same strip: bb.e21"
        ),
        "{}",
        run.stdout
    );

    let clear = scene_with("bb.e21", "", "");
    let dir = project(&clear);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.ends_with("scene.jsonc: no warnings\n"),
        "{}",
        run.stdout
    );

    // A seated part whose lead sits in a hole covered by the DHT22's body warns too.
    let under_lead = scene_with(
        "bb.e21",
        r#",
    "r2": { "type": "resistor-axial", "resistance": "10k" }"#,
        r#",
    "r2": { "on": { "a": "bb.d23" } }"#,
    );
    let dir = project(&under_lead);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.covered_hole"),
        [line_of(&under_lead, "\"r2\": { \"type\"")],
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(
            "warning[scene.covered_hole]: `r2.a` sits in `bb.d23`, which `dht`'s body covers"
        ),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("hint: move `r2` or `dht` so `dht` does not cover `bb.d23`"),
        "{}",
        run.stdout
    );
}

/// A DevKitC-1 placed with `at: [-80, 0, 0]` sits half inside an 830-tie breadboard at the
/// origin, and two parts placed with overlapping `at` positions collide too. Moving them clear
/// removes the warning.
#[test]
fn overlapping_placed_parts_warn_until_moved_apart() {
    let dir = project("");
    fs::copy(
        common::repo().join("blocks/devkitc-button-led/build/button.ino.bin"),
        dir.path().join("build/button.ino.bin"),
    )
    .unwrap();
    fs::copy(
        common::repo().join("blocks/devkitc-button-led/build/button.ino.partitions.bin"),
        dir.path().join("build/button.ino.partitions.bin"),
    )
    .unwrap();

    let devkit = |board_x: i32| {
        format!(
            r#"{{
  "format": 1,
  "parts": {{
    "bb": {{ "type": "breadboard-830" }},
    "board": {{
      "type": "esp32s3-devkitc-1",
      "app": "build/button.ino.bin",
      "partitions": "build/button.ino.partitions.bin"
    }},
    "btn1": {{ "type": "tactile-switch-6mm" }}
  }},
  "layout": {{
    "bb": {{ "at": [0, 0, 0] }},
    "board": {{ "at": [{board_x}, 0, 0] }},
    "btn1": {{ "on": {{ "a": "bb.f20" }}, "rotate": [0, 0, 90] }}
  }},
  "ground": "board.GND",
  "wires": [
    ["board.GPIO4", "bb.j20"],
    ["board.GND", "bb.j18"]
  ]
}}
"#
        )
    };

    let bad = devkit(-80);
    fs::write(dir.path().join("scene.jsonc"), &bad).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.overlap"),
        [line_of(&bad, "\"board\": {")],
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("warning[scene.overlap]: `bb` and `board` overlap in space"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(
            "hint: move `bb` or `board` in \"layout\" by at least 15.3 mm along X so their bodies do not overlap"
        ),
        "{}",
        run.stdout
    );

    fs::write(dir.path().join("scene.jsonc"), devkit(-100)).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.ends_with("scene.jsonc: no warnings\n"),
        "{}",
        run.stdout
    );

    // Two parts placed with `at` overlapping each other warn; touching edges within tolerance do not.
    let two_at = |x2: f64| {
        format!(
            r#"{{
  "format": 1,
  "parts": {{
    "nano": {{ "type": "nano-board", "firmware": "build/button.ino.hex" }},
    "btn1": {{ "type": "tactile-switch-6mm" }},
    "btn2": {{ "type": "tactile-switch-6mm" }}
  }},
  "layout": {{
    "nano": {{ "at": [0, 0, 0] }},
    "btn1": {{ "at": [40, 0, 0] }},
    "btn2": {{ "at": [{x2}, 0, 0] }}
  }},
  "nets": {{ "GND": "nano.GND" }},
  "wires": [
    ["nano.D2", "btn1.a"],
    ["btn1.b", "nano.GND"],
    ["nano.D3", "btn2.a"],
    ["btn2.b", "nano.GND"]
  ]
}}
"#
        )
    };

    let colliding = two_at(43.0);
    fs::write(dir.path().join("scene.jsonc"), &colliding).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        lines_warned(&run, "scene.overlap"),
        [line_of(&colliding, "\"btn2\": { \"type\"")],
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("warning[scene.overlap]: `btn1` and `btn2` overlap in space"),
        "{}",
        run.stdout
    );

    // Touching edges (6 mm wide switches 6 mm apart) stay clean.
    fs::write(dir.path().join("scene.jsonc"), two_at(46.0)).unwrap();
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.ends_with("scene.jsonc: no warnings\n"),
        "{}",
        run.stdout
    );
}

/// Tight valid placements stay clean: an LED next to its resistor, adjacent-row resistors, an IC
/// straddling the channel, a module seated on headers, a rotated breadboard, and the good
/// breadboard fixture.
#[test]
fn tight_valid_placements_and_good_fixtures_stay_clean() {
    let tight = r#"{
  "format": 1,
  "parts": {
    "bb": { "type": "breadboard-830" },
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "u1": { "type": "hc595-dip16" },
    "r1": { "type": "resistor-axial", "resistance": "220" },
    "r2": { "type": "resistor-axial", "resistance": "220" },
    "led1": { "type": "led-5mm" },
    "oled": { "type": "oled-ssd1306-128x64" }
  },
  "layout": {
    "bb": { "at": [10, 20, 0], "rotate": [0, 0, 90] },
    "nano": { "on": { "D13": "bb.d1" } },
    "u1": { "on": { "QB": "bb.e18" } },
    "r1": { "on": { "a": "bb.a30" } },
    "r2": { "on": { "a": "bb.b30" } },
    "led1": { "on": { "anode": "bb.c34" } },
    "oled": { "on": { "GND": "bb.a40" } }
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["bb.a12", "bb.W11"],
    ["bb.a14", "bb.X13"],
    ["bb.X15", "bb.Z15"],
    ["bb.i18", "bb.W19"],
    ["bb.a25", "bb.X25"],
    ["bb.i24", "bb.W23"],
    ["bb.i21", "bb.X21"],
    ["bb.i2", "bb.i20"],
    ["bb.i1", "bb.i23"],
    ["bb.i5", "bb.i22"],
    ["bb.c1", "bb.d30"],
    ["bb.d35", "bb.X35"],
    ["bb.b14", "bb.b40"],
    ["bb.b12", "bb.b41"],
    ["bb.b9", "bb.b42"],
    ["bb.b8", "bb.b43"]
  ]
}
"#;
    let dir = project(tight);
    let run = orbe(dir.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.ends_with("scene.jsonc: no warnings\n"),
        "{}",
        run.stdout
    );

    // Every good fixture that lays out parts on a breadboard stays clean.
    let fixture = common::overlaid(
        &common::repo().join("packages/scene/tests/good/ldr_and_trimmer_on_a_breadboard"),
    );
    let run = orbe(fixture.path(), &["lint", "scene.jsonc"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        !run.stdout.contains("scene.covered_hole") && !run.stdout.contains("scene.overlap"),
        "{}",
        run.stdout
    );
}
