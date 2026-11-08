//! The skills `orbe skills` prints, held to this binary. The core skill's worked example runs
//! from an empty folder and prints what the skill shows, and the stub's commands work.

#![cfg(target_os = "linux")]

#[path = "../../../packages/scene/tests/common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "session/harness.rs"]
mod harness;

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use harness::{Run, Runtime};

fn skill(name: &str) -> String {
    let runtime = Runtime::new();
    runtime
        .orbe(Path::new("."), &["skills", "get", name])
        .ok()
        .stdout
}

/// Each fenced block with its info string, such as `jsonc scene.jsonc`.
fn blocks(text: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let mut open: Option<(String, String)> = None;
    for line in text.lines() {
        match (line.strip_prefix("```"), open.take()) {
            (Some(info), None) => open = Some((info.to_string(), String::new())),
            (Some(_), Some(block)) => blocks.push(block),
            (None, Some((info, mut body))) => {
                body.push_str(line);
                body.push('\n');
                open = Some((info, body));
            }
            (None, None) => {}
        }
    }
    blocks
}

/// Whether `actual` is `shown`, where a shown line `...` stands for any lines.
fn matches(shown: &[&str], actual: &[&str]) -> bool {
    match shown.split_first() {
        None => actual.is_empty(),
        Some((&"...", rest)) => (0..=actual.len()).any(|skip| matches(rest, &actual[skip..])),
        Some((line, rest)) => actual
            .split_first()
            .is_some_and(|(first, more)| first == line && matches(rest, more)),
    }
}

/// Runs a shell line with this build's `orbe` first on `PATH`.
fn shell(runtime: &Runtime, dir: &Path, line: &str) -> Run {
    let bin = Path::new(env!("CARGO_BIN_EXE_orbe")).parent().unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let output = Command::new("sh")
        .args(["-c", line])
        .current_dir(dir)
        .env("PATH", path)
        .env("XDG_RUNTIME_DIR", runtime.dir.path())
        .env_remove("ORBE_SESSION")
        .env_remove("ORBE_TEST_IDLE_TIMEOUT")
        .env_remove("ORBE_TEST_STOP")
        .output()
        .unwrap();
    Run::new(output)
}

/// Applies a `diff` block: each `-` line of the file becomes the `+` line after it.
fn apply_diff(dir: &Path, file: &str, body: &str) {
    let path = dir.join(file);
    let mut text = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    for pair in lines.chunks(2) {
        let old = pair[0].strip_prefix('-');
        let new = pair.get(1).and_then(|l| l.strip_prefix('+'));
        let (Some(old), Some(new)) = (old, new) else {
            panic!("a diff block pairs each `-` line with a `+` line: {pair:?}");
        };
        assert_eq!(text.matches(old).count(), 1, "{file} has one `{old}`");
        text = text.replace(old, new);
    }
    fs::write(path, text).unwrap();
}

fn write_file(dir: &Path, file: &str, body: &str) {
    fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
    fs::write(dir.join(file), body).unwrap();
}

/// Runs each `$ ` line of a `console` block and compares what it prints with the lines shown.
fn run_shown(runtime: &Runtime, dir: &Path, transcript: &str) {
    let mut steps: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in transcript.lines() {
        match line.strip_prefix("$ ") {
            Some(command) => steps.push((command, Vec::new())),
            None => steps.last_mut().expect("a command first").1.push(line),
        }
    }
    for (command, shown) in &steps {
        assert!(command.starts_with("orbe "), "{command}");
        let run = shell(runtime, dir, command);
        let printed = format!("{}{}", run.stdout, run.stderr);
        let actual: Vec<&str> = printed.lines().collect();
        assert!(
            matches(shown, &actual),
            "`{command}` (exit {}) printed\n{printed}\nbut core shows\n{}",
            run.code,
            shown.join("\n")
        );
    }
}

/// The core skill's worked example, with its parts copied from the first target.
fn worked_example() -> tempfile::TempDir {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let first = common::repo().join("examples/first-target");
    common::copy_dir(&first.join("parts"), &dir.join("parts"));
    fs::copy(first.join("parts.lock"), dir.join("parts.lock")).unwrap();
    let (_, scene) = blocks(&skill("core"))
        .into_iter()
        .find(|(info, _)| info == "jsonc scene.jsonc")
        .unwrap();
    fs::write(dir.join("scene.jsonc"), scene).unwrap();
    fs::create_dir_all(dir.join("build")).unwrap();
    fs::copy(common::firmware(), dir.join("build/button.ino.hex")).unwrap();
    fs::create_dir(dir.join("tests")).unwrap();
    project
}

fn says(skill: &str, sentence: &str) {
    assert!(
        skill.contains(sentence),
        "the skill does not say: {sentence}"
    );
}

/// Every block of the core skill, in order, in one folder: each file block is written and
/// each `console` block runs as shown.
#[test]
fn the_core_examples_run_from_an_empty_folder_as_shown() {
    let text = skill("core");
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir_all(dir.join("build")).unwrap();
    fs::copy(common::firmware(), dir.join("build/button.ino.hex")).unwrap();
    let runtime = Runtime::new();
    let mut sessions = Vec::new();
    for (info, body) in blocks(&text) {
        match info.split_once(' ') {
            Some(("diff", file)) => apply_diff(dir, file, &body),
            Some((_, file)) => write_file(dir, file, &body),
            None if info == "console" => {
                run_shown(&runtime, dir, &body);
                sessions.push(body);
            }
            None => {}
        }
    }
    assert!(sessions.len() >= 2, "{} sessions", sessions.len());
    assert!(
        sessions[0].matches("$ orbe ").count() >= 10,
        "the worked example"
    );
    assert!(sessions[0].contains("\n$ orbe test\n"));
    assert!(
        runtime.daemons().is_empty(),
        "each example closes its session"
    );
}

#[test]
fn the_arduino_sketch_is_the_one_the_prebuilt_firmware_was_built_from() {
    let text = skill("arduino");
    let (_, sketch) = blocks(&text)
        .into_iter()
        .find(|(info, _)| info == "cpp button/button.ino")
        .expect("the arduino skill shows the sketch");
    let built_from = common::repo().join("packages/engine/tests/firmware/sketch/button/button.ino");
    assert_eq!(sketch, fs::read_to_string(built_from).unwrap());
    assert!(text.contains("\"firmware\": \"build/button.ino.hex\""));
}

#[test]
fn the_nano_pins_the_skills_call_not_simulated_are_the_ones_the_reader_leaves_out() {
    let scene = common::repo().join("examples/first-target/scene.jsonc");
    let netlist = orbe_scene::read(&scene).expect("the first target checks");
    let left_out: BTreeSet<&str> = netlist.refs.parts["nano"]
        .pins
        .iter()
        .filter(|(_, net)| net.is_none())
        .map(|(pin, _)| pin.as_str())
        .collect();
    assert_eq!(left_out, BTreeSet::from(["3V3", "A6", "A7", "VIN"]));
    assert!(skill("core").contains("The Nano's 3V3, VIN, A6 and A7 pins"));
    assert!(skill("arduino").contains("A6, A7, 3V3 and VIN are not simulated"));
}

#[test]
fn the_uno_pins_the_skills_call_not_simulated_are_the_ones_the_reader_leaves_out() {
    let scene = common::repo().join("examples/uno-button/scene.jsonc");
    let netlist = orbe_scene::read(&scene).expect("the uno example checks");
    let left_out: BTreeSet<&str> = netlist.refs.parts["uno"]
        .pins
        .iter()
        .filter(|(_, net)| net.is_none())
        .map(|(pin, _)| pin.as_str())
        .collect();
    assert_eq!(left_out, BTreeSet::from(["VIN"]));
    let (core, arduino) = (skill("core"), skill("arduino"));
    says(&core, "The Uno's VIN pin and barrel jack.");
    says(&core, "The Uno's 3V3 works.");
    says(&arduino, "On the Uno, VIN is not simulated");
    says(&arduino, "3V3 comes from the board's 3.3 V regulator.");
}

/// The arduino skill says the Uno's L LED pulls D13 to about 1.45 V while D13 is an input
/// with its pull-up on, so it reads low.
#[test]
fn the_uno_d13_with_its_pull_up_reads_low_as_the_arduino_skill_says() {
    says(
        &skill("arduino"),
        "With `INPUT_PULLUP` on D13 the LED pulls the pin to about 1.45 V, so it reads low",
    );
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let example = common::repo().join("examples/uno-button");
    common::copy_dir(&example.join("parts"), &dir.join("parts"));
    fs::copy(example.join("parts.lock"), dir.join("parts.lock")).unwrap();
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
    assert_eq!(names, ["core", "arduino"]);

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
        "error[usage]: no skill named `cor`\n  hint: the skills are core, arduino\n"
    );
    let typo = runtime
        .orbe(here, &["skills", "get", "cor", "--json"])
        .json();
    assert_eq!(typo["error"]["code"], "usage");
}

#[test]
fn help_lists_every_command_and_each_command_prints_its_usage() {
    let runtime = Runtime::new();
    let here = Path::new(".");
    let help = runtime.orbe(here, &["--help"]).ok().stdout;
    let listed: Vec<&str> = help
        .lines()
        .skip_while(|l| *l != "Commands:")
        .skip(1)
        .take_while(|l| !l.is_empty())
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    let mut want = vec!["check", "lint", "add", "part", "search"];
    want.extend(orbe_session::COMMANDS.iter().map(|s| s.name));
    want.extend(["show", "test", "skills", "help"]);
    assert_eq!(listed, want);
    for name in &want {
        for flag in ["--help", "-h"] {
            let run = runtime.orbe(here, &[name, flag]).ok();
            assert!(
                run.stdout.contains(&format!("Usage: orbe {name}")),
                "`orbe {name} {flag}`:\n{}{}",
                run.stdout,
                run.stderr
            );
        }
    }
    let step = runtime.orbe(here, &["help", "step"]).ok().stdout;
    assert!(step.contains("orbe step --until <condition> [--timeout <duration>] [OPTIONS]"));
    assert_eq!(runtime.orbe(here, &["help"]).ok().stdout, help);
    for flag in ["--help", "-h"] {
        let run = runtime.orbe(here, &["help", "step", flag]).ok();
        assert_eq!(run.stdout, step);
    }

    let early = runtime.orbe(here, &["--session", "x", "open"]);
    assert_eq!(early.code, 2);
    assert_eq!(
        early.stderr,
        "error[usage]: `--session x` is before the command name\n  hint: put it after the command name, as `orbe open --session x`\n"
    );
    let early = runtime.orbe(here, &["--json", "skills", "list"]).json();
    assert_eq!(early["error"]["code"], "usage");
    assert_eq!(
        early["error"]["hint"],
        "put it after the command name, as `orbe skills list --json`"
    );
    assert!(runtime.daemons().is_empty(), "help starts no session");
}

#[test]
fn the_exit_codes_the_core_skill_states_are_the_ones_orbe_exits_with() {
    let runtime = Runtime::new();
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bad.jsonc"), "{ not json").unwrap();
    let failed = runtime.orbe(dir.path(), &["check", "bad.jsonc"]).code;
    let usage = runtime.orbe(dir.path(), &["step", "2h"]).code;
    let io = runtime.orbe(dir.path(), &["check", "nope.jsonc"]).code;
    let stated = format!(
        "Exit codes: 0 ok, {failed} a failed check, `expect` or command, {usage} a usage error, {io} a file or system error."
    );
    assert!(
        skill("core").contains(&stated),
        "core does not say {stated}"
    );
}

/// The arduino skill's compile command, run as written in a folder that holds only its
/// sketch, writes the firmware its scene line names, and the core example passes on it.
#[test]
fn the_arduino_compile_command_writes_the_firmware_the_scene_names() {
    let avr = Command::new("arduino-cli").args(["core", "list"]).output();
    if !avr.is_ok_and(|o| {
        o.status.success() && String::from_utf8_lossy(&o.stdout).contains("arduino:avr ")
    }) {
        let skipped = "the_arduino_compile_command_writes_the_firmware_the_scene_names: skipped, arduino-cli with the arduino:avr core is not on PATH\n";
        std::io::stderr().write_all(skipped.as_bytes()).unwrap();
        return;
    }
    let text = skill("arduino");
    let shown = blocks(&text);
    let block = |info: &str| {
        shown
            .iter()
            .find(|(i, _)| i == info)
            .map(|(_, body)| body.as_str())
            .unwrap_or_else(|| panic!("arduino shows no `{info}` block"))
    };
    let compile = shown
        .iter()
        .flat_map(|(_, body)| body.lines())
        .find(|l| l.starts_with("arduino-cli compile "))
        .expect("arduino shows the compile command");
    let fqbn = compile
        .split(' ')
        .skip_while(|w| *w != "--fqbn")
        .nth(1)
        .expect("the compile command names an FQBN");
    assert!(text.contains(&format!("The Nano's FQBN is `{fqbn}`")));
    let board_name = |fqbn: &str| {
        let details = Command::new("arduino-cli")
            .args(["board", "details", "-b", fqbn, "--format", "json"])
            .output()
            .unwrap();
        let details: serde_json::Value = serde_json::from_slice(&details.stdout).unwrap();
        details["name"].as_str().unwrap_or_default().to_string()
    };
    assert_eq!(board_name(fqbn), "Arduino Nano", "{fqbn}");
    says(&text, "For the Uno, use `arduino:avr:uno` instead.");
    assert_eq!(board_name("arduino:avr:uno"), "Arduino UNO");

    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir(dir.join("button")).unwrap();
    fs::write(
        dir.join("button/button.ino"),
        block("cpp button/button.ino"),
    )
    .unwrap();
    let built = Command::new("sh")
        .args(["-c", compile])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let named = block("jsonc");
    let firmware = named
        .split("\"firmware\": \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the scene line names the firmware");
    assert!(
        dir.join(firmware).is_file(),
        "{compile} wrote no {firmware}"
    );
    let listed = text
        .lines()
        .find(|l| l.starts_with("`build/` then holds "))
        .expect("arduino lists what build/ holds");
    let listed: BTreeSet<String> = listed
        .split('`')
        .skip(3)
        .step_by(2)
        .map(str::to_string)
        .collect();
    let written: BTreeSet<String> = fs::read_dir(dir.join("build"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(listed, written);

    let core = skill("core");
    for (info, body) in blocks(&core) {
        match info.split_once(' ') {
            Some(("diff", _)) | None => {}
            Some((_, file)) => write_file(dir, file, &body),
        }
    }
    let runtime = Runtime::new();
    let add = core
        .lines()
        .find_map(|l| l.strip_prefix("$ orbe add "))
        .expect("core shows its orbe add");
    runtime
        .orbe(
            dir,
            &[&["add"][..], &add.split(' ').collect::<Vec<_>>()].concat(),
        )
        .ok();
    let run = runtime.orbe(dir, &["test"]).ok();
    assert!(
        run.stdout.ends_with("1 passed, 0 failed\n"),
        "{}",
        run.stdout
    );

    fs::create_dir(dir.join("other")).unwrap();
    fs::copy(dir.join("button/button.ino"), dir.join("other/other.ino")).unwrap();
    let other = compile.replace(" button", " other");
    let built = Command::new("sh")
        .args(["-c", &other])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(text.contains("`build/<sketch>.ino.hex`"));
    assert!(
        dir.join("build/other.ino.hex").is_file(),
        "the hex follows the sketch folder"
    );
}

#[test]
fn a_flag_on_a_test_line_is_a_usage_error_except_as_serial_text() {
    let project = worked_example();
    let dir = project.path();
    let lines = [
        "open",
        "close",
        "press @btn1",
        "release @btn1",
        "get @led1",
        "step 10ms",
        "step --until @led1 on",
        "snapshot",
        "expect @led1 off",
    ];
    let mut files = 0;
    for line in lines {
        let name = line.split(' ').next().unwrap();
        for flag in ["--json", "--session x", "--help", "-h"] {
            for written in [format!("{name} {flag}"), format!("{line} {flag}")] {
                files += 1;
                let opened = if name == "open" { "" } else { "open\n" };
                fs::write(
                    dir.join(format!("tests/f{files}.test")),
                    format!("{opened}{written}\n"),
                )
                .unwrap();
            }
        }
    }
    let runtime = Runtime::new();
    let run = runtime.orbe(dir, &["test"]);
    assert_eq!(run.code, 2);
    let failed: Vec<&str> = run
        .stdout
        .lines()
        .filter(|l| l.starts_with("FAIL "))
        .collect();
    assert_eq!(failed.len(), files, "{}", run.stdout);
    let usage = run.stdout.matches("\n  error[usage]: ").count();
    assert_eq!(usage, files, "{}", run.stdout);

    fs::remove_dir_all(dir.join("tests")).unwrap();
    fs::create_dir(dir.join("tests")).unwrap();
    fs::write(
        dir.join("tests/text.test"),
        "open\nexpect serial contains --help\n",
    )
    .unwrap();
    let run = runtime.orbe(dir, &["test"]);
    assert!(
        run.stdout
            .contains("error[expect_failed]: expected serial contains \"--help\""),
        "{}",
        run.stdout
    );
    let live = runtime
        .orbe(dir, &["expect", "serial", "contains", "--help"])
        .ok();
    assert!(live.stdout.contains("Usage: orbe expect"));
    fs::write(
        dir.join("tests/text.test"),
        "open\nexpect serial contains \"a\\nb\"\n",
    )
    .unwrap();
    let run = runtime.orbe(dir, &["test"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stdout
            .contains("error[usage]: only `\\\"` and `\\\\` escape inside quotes"),
        "{}",
        run.stdout
    );

    let core = skill("core");
    says(
        &core,
        "Each line is a live command without `orbe`. `--json`, `--session`, `--help` or `-h` on a line is a usage error, except as the text after `serial contains`. Live, `--help` anywhere prints help, so `orbe expect serial contains --help` prints help, while the same test line looks for that text.",
    );
    says(
        &core,
        "`\"...\"` holds spaces, and inside it only `\\\"` and `\\\\` escape, so a test cannot name a line ending.",
    );
}

#[test]
fn the_session_name_rule_core_states_is_the_one_orbe_follows() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let project = repo.join("proj");
    let plain = root.path().join("plain");
    for dir in [
        repo.join(".git"),
        repo.join("notes"),
        project.join("tests"),
        project.join("build"),
        plain.clone(),
    ] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(project.join("scene.jsonc"), "{}").unwrap();
    let runtime = Runtime::new();
    let name = |dir: &Path| {
        let json = runtime.orbe(dir, &["snapshot", "--json"]).json();
        assert_eq!(json["error"]["code"], "no_session");
        json["session"].as_str().unwrap().to_string()
    };
    let stem = |name: &str| name.rsplit_once('-').unwrap().0.to_string();
    assert_eq!(stem(&name(&project)), "proj");
    assert_eq!(name(&project.join("tests")), name(&project));
    assert_eq!(name(&project.join("build")), name(&project));
    assert_eq!(stem(&name(&repo.join("notes"))), "repo");
    assert_eq!(stem(&name(&plain)), "plain");
    says(
        &skill("core"),
        "By default it comes from the project folder, the nearest one up with `scene.jsonc` or `parts.lock`, so each project gets its own, and its `tests/` folder shares it. Outside a project it comes from the git repository's root folder, else the current folder.",
    );
}

#[test]
fn the_check_findings_core_states_are_the_ones_orbe_reports() {
    let project = worked_example();
    let dir = project.path();
    let runtime = Runtime::new();
    let core = skill("core");

    fs::rename(dir.join("parts.lock"), dir.join("parts.lock.kept")).unwrap();
    let json = runtime
        .orbe(dir, &["check", "scene.jsonc", "--json"])
        .json();
    let found = &json["data"]["diagnostics"][0];
    assert_eq!(found["code"], "part_missing", "{json}");
    assert!(
        found["hint"]
            .as_str()
            .unwrap()
            .starts_with("run `orbe add ")
    );
    says(
        &core,
        &format!(
            "A failed `orbe check scene.jsonc --json` is `{}`, with each finding, its code and hint in `data.diagnostics`.",
            json["error"]["code"].as_str().unwrap()
        ),
    );
    fs::rename(dir.join("parts.lock.kept"), dir.join("parts.lock")).unwrap();

    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    let drawn = scene.replace(
        "\"resistance\": \"10k\" }",
        "\"resistance\": \"10k\", \"not_simulated\": \"drawn only\" }",
    );
    assert_ne!(drawn, scene);
    fs::write(dir.join("scene.jsonc"), &drawn).unwrap();
    let run = runtime.orbe(dir, &["check", "scene.jsonc"]);
    assert!(
        run.stdout
            .contains("error[not_simulated]: r2.a: drawn only"),
        "{}",
        run.stdout
    );
    says(
        &core,
        "Nothing may be wired to it. A wire to one of its pins is a `not_simulated` error that gives `<why>`.",
    );
    fs::write(dir.join("scene.jsonc"), &scene).unwrap();

    for (id, code) in [("led-5mm", 1), ("nano-board", 0)] {
        let file = dir.join("parts").join(id).join("part.jsonc");
        let text = fs::read_to_string(&file).unwrap();
        fs::write(&file, format!("{text}\n")).unwrap();
        let run = runtime.orbe(dir, &["check", "scene.jsonc"]);
        assert_eq!(run.code, code, "{id}: {}", run.stdout);
        let shown = if code == 1 {
            "error[part_modified]"
        } else {
            "note: `nano-board` differs from its parts.lock line"
        };
        assert!(run.stdout.contains(shown), "{id}: {}", run.stdout);
        fs::write(&file, text).unwrap();
    }
    says(
        &core,
        "A copy that differs, even in spacing, is a `part_modified` error. A board built from other parts, such as `nano-board`, is the exception. An edit to it checks, with a note.",
    );
}

/// A flag before the command name gets a hint that moves the words before it to after it, as
/// typed, and drops only a repeat clap refuses. The hinted line runs, or fails with the
/// command's own error, and an `add` writes the folder its `-C` names.
#[test]
fn a_flag_before_the_command_gets_a_hint_whose_line_runs() {
    let project = worked_example();
    let dir = project.path();
    for folder in ["check", "dir", "-proj", "a", "b"] {
        write_file(dir, &format!("{folder}/scene.jsonc"), "{ \"format\": 1 }\n");
    }
    let runtime = Runtime::new();
    runtime.orbe(dir, &["open"]).ok();
    let unexpected = "error: unexpected argument '--session' found";
    let twice = "error: the argument '-C <DIR>' cannot be used multiple times";
    for (written, line, code, lock, says) in [
        ("--session test open", "open --session test", 0, None, ""),
        (
            "--session x check scene.jsonc",
            "check scene.jsonc --session x",
            2,
            None,
            unexpected,
        ),
        (
            "-C check add led-5mm",
            "add led-5mm -C check",
            0,
            Some("check"),
            "",
        ),
        (
            "--timeout 50ms step --until @led1 off",
            "step --until @led1 off --timeout 50ms",
            0,
            None,
            "",
        ),
        (
            "--timeout=50ms step --until @led1 off",
            "step --until @led1 off --timeout=50ms",
            0,
            None,
            "",
        ),
        (
            "-Cdir add resistor-axial",
            "add resistor-axial -Cdir",
            0,
            Some("dir"),
            "",
        ),
        (
            "-C./dir add led-5mm",
            "add led-5mm -C./dir",
            0,
            Some("dir"),
            "",
        ),
        (
            "--json -Cdir add ams1117-sot223",
            "add ams1117-sot223 --json -Cdir",
            0,
            Some("dir"),
            "",
        ),
        (
            "-C -proj add capacitor-0603",
            "add capacitor-0603 -C -proj",
            0,
            Some("-proj"),
            "",
        ),
        (
            "-C a -C b add resonator-cstce",
            "add resonator-cstce -C a -C b",
            2,
            None,
            twice,
        ),
        (
            "-C a -C a add led-5mm",
            "add led-5mm -C a",
            0,
            Some("a"),
            "",
        ),
        (
            "--json --json check scene.jsonc",
            "check scene.jsonc --json",
            0,
            None,
            "",
        ),
        (
            "--session p --session q check scene.jsonc",
            "check scene.jsonc --session p --session q",
            2,
            None,
            unexpected,
        ),
        (
            "--session a --session b open",
            "open --session a --session b",
            0,
            None,
            "",
        ),
        (
            "--json expect @led1.i '<' 1A",
            "expect @led1.i '<' 1A --json",
            0,
            None,
            "",
        ),
    ] {
        let run = shell(&runtime, dir, &format!("orbe {written}"));
        assert_eq!(run.code, 2, "orbe {written}");
        let (message, hint) = if written.contains("--json") {
            let error = &run.json()["error"];
            let text = |key: &str| error[key].as_str().unwrap().to_string();
            (text("message"), text("hint"))
        } else {
            let (message, hint) = run.stderr.trim_end().split_once("\n  hint: ").unwrap();
            (message.replace("error[usage]: ", ""), hint.to_string())
        };
        assert!(
            message.ends_with("` is before the command name"),
            "{message}"
        );
        let hinted = format!("put it after the command name, as `orbe {line}`");
        assert_eq!(hint, hinted, "orbe {written}");
        let run = shell(&runtime, dir, &format!("orbe {line}"));
        let printed = format!("{}{}", run.stdout, run.stderr);
        assert_eq!(run.code, code, "orbe {line}: {printed}");
        assert!(printed.starts_with(says), "orbe {line}: {printed}");
        if let Some(folder) = lock {
            let id = line.split(' ').nth(1).unwrap();
            let lock = fs::read_to_string(dir.join(folder).join("parts.lock")).unwrap();
            assert!(lock.contains(id), "orbe {line} adds {id} to {folder}");
        }
    }
    runtime.orbe(dir, &["close", "--all"]).ok();
}

/// Each input on a bouncing switch lands just after the window of the one before, and a read
/// past the last input's time plus the window sees the switch settled. Two windows, with a
/// press last and a release last.
#[test]
fn a_bouncing_switch_reads_settled_where_core_says() {
    let core = skill("core");
    for sentence in [
        "an input on it lands after the window of the one before.",
        "a `release` and a `press` with no `step` between land 2 ms apart, and `step --until @led1 on` stops at the first contact, which opens again inside the window",
        "To read a bouncing switch settled, step past the time in the last input's reply plus the window.",
    ] {
        says(&core, sentence);
    }
    let (_, diff) = blocks(&core)
        .into_iter()
        .find(|(info, _)| info == "diff scene.jsonc")
        .expect("core shows its bounce change");
    let project = worked_example();
    let dir = project.path();
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    let runtime = Runtime::new();
    let orbe = |args: &[&str]| runtime.orbe(dir, &[args, &["--json"]].concat()).ok().json();
    for (window, window_ps) in [("2ms", 2_000_000_000u64), ("200us", 200_000_000)] {
        fs::write(dir.join("scene.jsonc"), &scene).unwrap();
        apply_diff(dir, "scene.jsonc", &diff.replace("2ms", window));
        orbe(&["open"]);
        let mut now = orbe(&["step", "100ms"])["t_ps"].as_u64().unwrap();
        for inputs in [&["press"][..], &["release", "press"], &["release"]] {
            let mut last: Option<u64> = None;
            for input in inputs {
                let landed = orbe(&[input, "@btn1"])["data"]["landed_ps"]
                    .as_u64()
                    .unwrap();
                match last {
                    None => assert!(landed >= now, "{window}: {input} lands at {landed}"),
                    Some(before) => assert!(
                        landed > before + window_ps && landed <= before + window_ps + 1_000_000,
                        "{window}: {input} lands at {landed}, {before} + the window before"
                    ),
                }
                last = Some(landed);
            }
            let past = last.unwrap() + window_ps + 1;
            orbe(&["step", &format!("{}ps", past - now)]);
            let (pin, led) = match inputs.last() {
                Some(&"press") => ("high", "on"),
                _ => ("low", "off"),
            };
            orbe(&["expect", "@nano.D2", pin]);
            now = orbe(&["step", "1ms"])["t_ps"].as_u64().unwrap();
            orbe(&["expect", "@led1", led]);
        }
        orbe(&["close"]);
    }
}

#[test]
fn a_command_sent_while_a_step_runs_waits_for_it() {
    says(
        &skill("core"),
        "A command sent while a `step` runs waits for it.",
    );
    let project = worked_example();
    let dir = project.path();
    let runtime = Runtime::new();
    runtime.orbe(dir, &["open"]).ok();
    let stepping = runtime.spawn(dir, &["step", "600s", "--json"]);
    std::thread::sleep(Duration::from_millis(300));
    let mut reading = runtime.spawn(dir, &["snapshot", "--json"]);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        reading.try_wait().unwrap().is_none(),
        "snapshot waits while the step runs"
    );
    harness::kill("-INT", stepping.id());
    let reached = Run::new(stepping.wait_with_output().unwrap()).json()["t_ps"].clone();
    let read = Run::new(reading.wait_with_output().unwrap()).ok().json();
    assert_eq!(
        read["t_ps"], reached,
        "snapshot reads where the step stopped"
    );
    runtime.orbe(dir, &["close"]).ok();
}

/// `flood.ino` prints one line in `loop`, built by arduino-cli as the arduino skill says.
#[test]
fn snapshot_joins_the_last_serial_lines_with_a_newline_and_a_println_in_loop_repeats() {
    says(
        &skill("core"),
        "`snapshot` shows the last 10 lines of it, joined with `\\n`.",
    );
    says(
        &skill("arduino"),
        "A `println` in `loop` prints on every pass, so its text repeats.",
    );
    let project = worked_example();
    let dir = project.path();
    let flood = common::repo().join("apps/orbe/tests/firmware/flood/flood.hex");
    fs::copy(flood, dir.join("build/button.ino.hex")).unwrap();
    let runtime = Runtime::new();
    runtime.orbe(dir, &["open"]).ok();
    runtime.orbe(dir, &["step", "50ms"]).ok();
    let snapshot = runtime.orbe(dir, &["snapshot", "--json"]).ok().json();
    let serial = snapshot["data"]["serial"].as_str().unwrap();
    assert_eq!(serial.split('\n').count(), 10, "{serial:?}");
    assert!(!serial.contains('\r'), "{serial:?}");
    let looped = "abcdefghijklmnopqrstuvwxyz0123456789";
    assert!(
        serial.split('\n').take(9).all(|l| l == looped),
        "{serial:?}"
    );
    runtime
        .orbe(dir, &["expect", "serial", "contains", "ready"])
        .ok();
    runtime.orbe(dir, &["close"]).ok();
}

#[test]
fn a_failed_test_prints_what_core_shows() {
    let core = skill("core");
    says(
        &core,
        "With the first `expect` in `tests/button.test` changed to `expect @led1 on`:",
    );
    let blocks = blocks(&core);
    let block =
        |found: fn(&(String, String)) -> bool| blocks.iter().find(|b| found(b)).unwrap().1.clone();
    let test = block(|(info, _)| info == "text tests/button.test");
    let shown = block(|(info, body)| info.is_empty() && body.starts_with("FAIL "));
    let project = worked_example();
    let dir = project.path();
    let failing = test.replacen("expect @led1 off", "expect @led1 on", 1);
    write_file(dir, "tests/button.test", &failing);
    let run = Runtime::new().orbe(dir, &["test"]);
    assert_eq!(run.code, 1);
    assert_eq!(format!("{}{}", run.stdout, run.stderr), shown);
}

#[test]
fn an_led_with_no_series_resistor_warns_and_holds_its_pin_low() {
    let project = worked_example();
    let dir = project.path();
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    let series = scene
        .lines()
        .find(|l| l.contains("\"r1\""))
        .and_then(|l| l.split("\"resistance\": \"").nth(1))
        .and_then(|v| v.split('"').next())
        .unwrap()
        .to_owned();
    let cathode = "[\"led1.cathode\", \"nano.GND\"],";
    let bare: String = scene
        .lines()
        .filter(|l| !l.contains("\"r1"))
        .map(|l| format!("{l}\n"))
        .collect::<String>()
        .replace(
            cathode,
            &format!("[\"nano.D13\", \"led1.anode\"], {cathode}"),
        );
    assert!(!bare.contains("r1") && bare.contains("\"led1.anode\"]"));
    fs::write(dir.join("scene.jsonc"), bare).unwrap();
    let runtime = Runtime::new();
    let orbe = |args: &[&str]| runtime.orbe(dir, args);
    let lint = orbe(&["lint", "scene.jsonc"]).ok().stdout;
    let caught = "warning[scene.led_no_resistor]: nothing limits the current through `led1`";
    assert!(lint.contains(caught), "{lint}");
    let open = orbe(&["open"]).ok().stdout;
    assert!(!open.contains("scene.led_no_resistor"), "{open}");
    orbe(&["step", "100ms"]).ok();
    orbe(&["press", "@btn1"]).ok();
    let step = orbe(&["step", "--until", "@led1", "on", "--timeout", "50ms"])
        .ok()
        .stdout;
    for warning in [
        "warning[rating.current] #led1.i peaks at ",
        "A, over its 0.03 A rating",
        "warning[rating.power] #led1.p peaks at ",
        "W, over its 0.075 W rating",
    ] {
        assert!(step.contains(warning), "{step}");
    }
    let led = orbe(&["get", "@led1", "--json"]).ok().json()["data"]["value"].clone();
    let pin = orbe(&["get", "@nano.D13", "--json"]).ok().json()["data"]["value"].clone();
    let expect = orbe(&["expect", "no", "diagnostics"]);
    assert_eq!(expect.code, 1);
    assert!(
        expect
            .stdout
            .contains("actual:   rating.current, rating.power"),
        "{}",
        expect.stdout
    );
    orbe(&["close"]).ok();

    let core = skill("core");
    says(
        &core,
        &format!(
            "With D13 wired straight to `led1.anode` and no `r1`, the LED draws {:.0} mA. `step` lists `warning[rating.current]` and `warning[rating.power]` for `led1`, and `expect no diagnostics` fails. The LED holds D13 at {:.1} V, so D13 reads `{}` while the LED is {}.",
            led["i"].as_f64().unwrap() * 1e3,
            pin["v"].as_f64().unwrap(),
            pin["level"].as_str().unwrap(),
            if led["on"] == true { "on" } else { "off" },
        ),
    );
    says(
        &core,
        "`orbe lint` warns of it before any run, with `warning[scene.led_no_resistor]` on `led1`.",
    );
    says(
        &core,
        &format!("Give each LED a series resistor, such as {series} ohm from a 5 V pin."),
    );
    says(
        &core,
        "- It warns `rating.current` above 30 mA and `rating.power` above 75 mW.",
    );
}

#[test]
fn a_resistor_over_its_power_rating_warns_as_its_parts_line_says() {
    let project = worked_example();
    let dir = project.path();
    write_file(
        dir,
        "hot.jsonc",
        r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "r9": { "type": "resistor-axial", "resistance": "10" },
  },
  "wires": [["nano.5V", "r9.a"], ["r9.b", "nano.GND"]],
}
"#,
    );
    let runtime = Runtime::new();
    let open = runtime.orbe(dir, &["open", "hot.jsonc"]).ok().stdout;
    assert!(
        open.contains("warning[rating.power] #r9.p peaks at 2.5 W, over its 0.25 W rating"),
        "{open}"
    );
    runtime.orbe(dir, &["close"]).ok();
    says(&skill("core"), "- It warns `rating.power` above 250 mW.");
}

#[test]
fn part_readings_show_a_pin_edge_where_until_stops_and_led_l_follows_d13() {
    let core = skill("core");
    says(
        &core,
        "A pin condition stops 1 ps after the exact edge, and part readings such as `@led1 on` already show the edge there, with no extra `step`.",
    );
    says(
        &core,
        "- `@nano.mcu.PB5`, `@nano.led_l`: a part inside a part, by its name in that part's file. On the Nano and the Uno the chip is `mcu`, and `led_l` is the L LED, which D13 drives.",
    );
    let project = worked_example();
    let dir = project.path();
    let runtime = Runtime::new();
    let orbe = |args: &[&str]| runtime.orbe(dir, args).ok();
    orbe(&["open"]);
    orbe(&["step", "100ms"]);
    for (input, level, before, after) in [
        ("press", "high", "off", "on"),
        ("release", "low", "on", "off"),
    ] {
        for led in ["@led1", "@nano.led_l"] {
            orbe(&["expect", led, before]);
        }
        orbe(&[input, "@btn1"]);
        orbe(&["step", "--until", "@nano.D13", level]);
        for led in ["@led1", "@nano.led_l"] {
            orbe(&["expect", led, after]);
        }
    }
    orbe(&["close"]);
}

#[test]
fn orbe_test_leaves_an_open_session_alone_and_none_to_close() {
    let core = skill("core");
    let project = worked_example();
    let dir = project.path();
    let (_, test) = blocks(&core)
        .into_iter()
        .find(|(info, _)| info == "text tests/button.test")
        .unwrap();
    write_file(dir, "tests/button.test", &test);
    let runtime = Runtime::new();
    let live = ["--session", "sg-live"];
    runtime.orbe(dir, &[&["open"][..], &live].concat()).ok();
    runtime
        .orbe(dir, &[&["step", "7ms"][..], &live].concat())
        .ok();
    let run = Run::new(
        runtime
            .command(dir, &["test"])
            .env("ORBE_SESSION", "sg-live")
            .output()
            .unwrap(),
    );
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.ends_with("1 passed, 0 failed\n"),
        "{}",
        run.stdout
    );
    let flagged = runtime.orbe(dir, &[&["test"][..], &live].concat());
    assert_eq!(flagged.code, 2);
    assert!(
        flagged
            .stderr
            .contains("unexpected argument '--session' found"),
        "{}",
        flagged.stderr
    );
    let kept = runtime
        .orbe(dir, &[&["snapshot", "--json"][..], &live].concat())
        .ok()
        .json();
    assert_eq!(
        kept["t_ps"], 7_000_000_000u64,
        "the open session is left at 7ms"
    );
    runtime.orbe(dir, &[&["close"][..], &live].concat()).ok();

    runtime.orbe(dir, &["test"]).ok();
    assert!(runtime.daemons().is_empty(), "orbe test leaves no session");
    let close = runtime.orbe(dir, &["close", "--json"]);
    assert_eq!(close.code, 1);
    let code = close.json()["error"]["code"].as_str().unwrap().to_string();
    says(
        &core,
        &format!(
            "8. Run `orbe close` when the live work is done. A `{code}` error from it means nothing was open.\n9. Write what worked as `tests/<name>.test` and run `orbe test`. It leaves no session to close."
        ),
    );
    says(
        &core,
        "Each file's session lives inside `orbe test` and ends with it. A session that `orbe open` started keeps running, untouched. `orbe test` takes no `--session` and ignores `ORBE_SESSION`.",
    );
}

#[test]
fn no_diagnostics_says_a_chip_warning_can_show_one_engine_step_late() {
    says(
        &skill("core"),
        "A chip warning can show up to one engine step (10 ms) late, so step that far past its cause first.",
    );
    let project = worked_example();
    let dir = project.path();
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    let runtime = Runtime::new();
    for (sim, max_step) in [("", "10ms"), ("\"sim\": { \"max_step\": \"1ms\" },", "1ms")] {
        let set = scene.replacen("\"format\": 1,", &format!("\"format\": 1, {sim}"), 1);
        fs::write(dir.join("scene.jsonc"), set).unwrap();
        runtime.orbe(dir, &["open"]).ok();
        let reply = runtime
            .orbe(dir, &["expect", "no", "diagnostics", "--json"])
            .ok()
            .json();
        assert_eq!(
            reply["data"]["actual"],
            format!("no diagnostics; a chip warning can show up to {max_step} late")
        );
        runtime.orbe(dir, &["close"]).ok();
    }
}

/// The worked example with every lint rule broken once: D13 straight to the LED, a part of its
/// own with no model, and a turn past 180 degrees. Each code `orbe lint` prints is in the core skill's
/// warnings table, the table names no other, and `orbe open` shows only `net.floating`.
#[test]
fn the_warnings_table_lists_the_codes_lint_prints_and_open_shows_only_the_engines() {
    let project = worked_example();
    let dir = project.path();
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    let broken = scene
        .replace("[\"nano.D13\", \"r1.a\"]", "[\"nano.D13\", \"led1.anode\"]")
        .replace("[\"r1.b\", \"led1.anode\"],", "")
        .replace(
            "\"parts\": {",
            "\"parts\": {\n    \"jump\": { \"type\": \"./jumper.jsonc\" },",
        )
        .replace(
            "\"wires\": [",
            "\"layout\": { \"btn1\": { \"at\": [0, 0, 0], \"rotate\": [0, 0, 270] } },\n  \"wires\": [",
        );
    assert_ne!(broken, scene);
    fs::write(dir.join("scene.jsonc"), &broken).unwrap();
    fs::write(
        dir.join("jumper.jsonc"),
        "{ \"format\": 1, \"pins\": { \"a\": [1], \"b\": [2] }, \"wires\": [[\"a\", \"b\"]] }\n",
    )
    .unwrap();
    let runtime = Runtime::new();
    let lint = runtime.orbe(dir, &["lint", "scene.jsonc"]).ok().stdout;
    let code = |line: &str| {
        let rest = line.split_once("warning[")?.1;
        Some(rest.split(']').next()?.to_string())
    };
    let printed: BTreeSet<String> = lint.lines().filter_map(code).collect();
    let core = skill("core");
    let table: BTreeSet<String> = core
        .split("## Warnings")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .unwrap()
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split('`').next())
        .map(str::to_string)
        .collect();
    assert_eq!(printed, table, "{lint}");
    let warnings = lint.lines().filter(|l| l.contains(": warning[")).count();
    assert_eq!(lint.matches("\n  hint: ").count(), warnings, "{lint}");

    let open = runtime.orbe(dir, &["open"]).ok().stdout;
    runtime.orbe(dir, &["close"]).ok();
    let shown: BTreeSet<String> = open.lines().filter_map(code).collect();
    assert_eq!(
        shown,
        BTreeSet::from(["net.floating".to_string()]),
        "{open}"
    );
    says(
        &core,
        "`orbe open` shows none of these warnings except `net.floating`.",
    );
}
