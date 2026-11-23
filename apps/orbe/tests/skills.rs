//! The skills `orbe skills` prints, held to this binary. The core skill's worked example runs
//! from an empty folder and prints what the skill shows, each topic's examples run too, and
//! core stays within 8 KB and every other skill within 6 KB.

#![cfg(target_os = "linux")]

#[path = "../../../packages/scene/tests/common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "session/harness.rs"]
mod harness;
#[path = "registry_env.rs"]
mod registry_env;
#[path = "shown/mod.rs"]
mod shown;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use harness::Runtime;
use orbe_scene::Files;
use shown::*;

#[test]
fn the_core_examples_run_from_an_empty_folder_as_shown() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir_all(dir.join("build")).unwrap();
    fs::copy(common::firmware(), dir.join("build/button.ino.hex")).unwrap();
    let sessions = run_blocks(&skill("core"), dir);
    assert!(
        sessions[0].matches("$ orbe ").count() >= 10,
        "the worked example"
    );
    assert!(sessions[0].contains("\n$ orbe test\n"));
}

/// Each topic's console examples run as shown. `parts` starts from an empty folder. The
/// others start from the core skill's worked example.
#[test]
fn the_topic_examples_run_as_shown() {
    let runtime = Runtime::new();
    let names: Vec<String> = runtime
        .orbe(Path::new("."), &["skills", "list"])
        .ok()
        .stdout
        .lines()
        .map(|line| line.split(' ').next().unwrap().to_string())
        .filter(|name| name != "core")
        .collect();
    assert!(names.len() > 3, "{names:?}");
    for name in &names {
        let owned;
        let dir = if name == "parts" {
            owned = tempfile::tempdir().unwrap();
            owned.path()
        } else {
            owned = worked_example();
            owned.path()
        };
        let sessions = run_blocks(&skill(name), dir);
        if matches!(name.as_str(), "parts" | "breadboards" | "sessions-and-time") {
            assert!(!sessions.is_empty(), "{name} shows no console example");
        }
    }
}

/// Core stays short enough to read before every build. A topic stays short enough to load
/// for one question.
#[test]
fn each_skill_stays_within_its_budget() {
    let runtime = Runtime::new();
    let list = runtime.orbe(Path::new("."), &["skills", "list"]).ok();
    for name in list.stdout.lines().map(|l| l.split(' ').next().unwrap()) {
        let size = skill(name).len();
        let limit = if name == "core" { 8 * 1024 } else { 6 * 1024 };
        assert!(
            size <= limit,
            "`orbe skills get {name}` prints {size} bytes, over {limit}"
        );
    }
}

#[test]
fn the_arduino_sketch_is_the_one_the_prebuilt_firmware_was_built_from() {
    let text = skill("arduino");
    let (_, sketch) = blocks(&text)
        .into_iter()
        .find(|(info, _)| info == "cpp button/button.ino")
        .expect("the arduino skill shows the sketch");
    let built_from = common::repo().join("blocks/nano-button-led/firmware/button/button.ino");
    assert_eq!(sketch, fs::read_to_string(built_from).unwrap());
}

#[test]
fn the_nano_pins_the_skills_call_not_simulated_are_the_ones_the_reader_leaves_out() {
    let scene = common::repo().join("blocks/nano-button-led/scene.jsonc");
    let netlist = orbe_scene::read(&Files::disk(), &scene).expect("the Nano Block checks");
    let left_out: BTreeSet<&str> = netlist.refs.parts["nano"]
        .pins
        .iter()
        .filter(|(_, net)| net.is_none())
        .map(|(pin, _)| pin.as_str())
        .collect();
    assert_eq!(left_out, BTreeSet::from(["3V3"]));
}

#[test]
fn the_uno_pins_the_skills_call_not_simulated_are_the_ones_the_reader_leaves_out() {
    let scene = common::repo().join("blocks/uno-button-led/scene.jsonc");
    let netlist = orbe_scene::read(&Files::disk(), &scene).expect("the Uno Block checks");
    let left_out: BTreeSet<&str> = netlist.refs.parts["uno"]
        .pins
        .iter()
        .filter(|(_, net)| net.is_none())
        .map(|(pin, _)| pin.as_str())
        .collect();
    assert_eq!(left_out, BTreeSet::from(["VIN"]));
}

/// INPUT_PULLUP on the Uno's D13 pulls the pin to about 1.45 V through the L LED, so it reads low.
#[test]
fn the_uno_d13_with_its_pull_up_reads_low() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    add_parts(dir, &["uno-board"]);
    fs::copy(
        common::repo().join("packages/engine/tests/firmware/pullup.hex"),
        dir.join("pullup.hex"),
    )
    .unwrap();
    write_file(
        dir,
        "scene.jsonc",
        r#"{ "format": 1, "parts": { "uno": { "type": "uno-board", "firmware": "pullup.hex" } } }"#,
    );
    write_file(
        dir,
        "tests/pullup.test",
        "open\nstep 10ms\nexpect @uno.D13 low\nexpect @uno.D13 > 1.4V\nexpect @uno.D13 < 1.5V\n",
    );
    let run = Runtime::new().orbe(dir, &["test"]).ok();
    assert!(
        run.stdout.ends_with("1 passed, 0 failed\n"),
        "{}",
        run.stdout
    );
}

#[test]
fn the_stub_follows_the_agent_skills_format_and_its_commands_work() {
    let path = common::repo().join("skills/orbe/SKILL.md");
    let stub = fs::read_to_string(&path).unwrap();
    let front = stub
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .expect("the stub opens with frontmatter")
        .0;
    let field = |key: &str| {
        front
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}: ")))
            .unwrap_or_else(|| panic!("no `{key}`"))
    };
    let folder = path.parent().unwrap().file_name().unwrap();
    assert_eq!(field("name"), folder.to_str().unwrap());
    assert!((1..=1024).contains(&field("description").len()));

    let runtime = Runtime::new();
    let commands: Vec<&str> = stub
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.starts_with("orbe "))
        .collect();
    assert!(commands.contains(&"orbe skills get core"));
    for command in commands {
        let words: Vec<&str> = command.split(' ').skip(1).collect();
        let run = runtime.orbe(Path::new("."), &words).ok();
        assert!(!run.stdout.is_empty(), "`{command}` printed nothing");
    }
}

#[test]
fn skills_list_and_get_print_text_and_json_and_name_the_skills_on_a_typo() {
    let runtime = Runtime::new();
    let here = Path::new(".");
    let list = runtime.orbe(here, &["skills", "list"]).ok();
    let names: Vec<&str> = list
        .stdout
        .lines()
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "core",
            "arduino",
            "parts",
            "scenes",
            "breadboards",
            "sessions-and-time",
            "tests",
            "ratings",
            "errors",
            "page",
            "not-simulated",
        ]
    );

    let json = runtime
        .orbe(here, &["skills", "list", "--json"])
        .ok()
        .json();
    assert_eq!(json["data"]["skills"][1]["name"], "arduino");

    let text = runtime.orbe(here, &["skills", "get", "core"]).ok().stdout;
    let json = runtime
        .orbe(here, &["skills", "get", "core", "--json"])
        .ok()
        .json();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["text"], text);

    let typo = runtime.orbe(here, &["skills", "get", "cor"]);
    assert_eq!(typo.code, 2);
    assert_eq!(
        typo.stderr,
        "error[usage]: no skill named `cor`\n  hint: the skills are core, arduino, parts, scenes, breadboards, sessions-and-time, tests, ratings, errors, page, not-simulated\n"
    );
    let typo = runtime
        .orbe(here, &["skills", "get", "cor", "--json"])
        .json();
    assert_eq!(typo["error"]["code"], "usage");
}
