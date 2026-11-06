//! The skills `orbe skills` prints, held to this binary. The core skill's worked example runs
//! from an empty folder and prints what the skill shows, and the stub's commands work.

#![cfg(target_os = "linux")]

#[path = "../../orbe-scene/tests/common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "session/harness.rs"]
mod harness;

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

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

#[test]
fn the_core_worked_example_runs_from_an_empty_folder_as_shown() {
    let text = skill("core");
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let mut files = 0;
    let mut transcript = None;
    for (info, body) in blocks(&text) {
        match info.split_once(' ') {
            Some((_, file)) => {
                let path = dir.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, body).unwrap();
                files += 1;
            }
            None if info == "console" => {
                assert!(transcript.is_none(), "core shows one session");
                transcript = Some(body);
            }
            None => {}
        }
    }
    assert_eq!(files, 2, "the scene and the test file");
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    assert!(scene.contains("\"firmware\": \"build/button.ino.hex\""));
    fs::create_dir_all(dir.join("build")).unwrap();
    fs::copy(common::firmware(), dir.join("build/button.ino.hex")).unwrap();

    let runtime = Runtime::new();
    let transcript = transcript.expect("core shows a session");
    let mut steps: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in transcript.lines() {
        match line.strip_prefix("$ ") {
            Some(command) => steps.push((command, Vec::new())),
            None => steps.last_mut().expect("a command first").1.push(line),
        }
    }
    assert!(steps.len() >= 10, "{} commands", steps.len());
    for (command, shown) in &steps {
        assert!(command.starts_with("orbe "), "{command}");
        let run = shell(&runtime, dir, command);
        let printed = format!("{}{}", run.stdout, run.stderr);
        let actual: Vec<&str> = printed.lines().collect();
        assert!(
            matches(shown, &actual),
            "`{command}` (exit {}) printed\n{printed}\nbut core shows\n{}",
            run.code,
            shown.join("\n")
        );
    }
    assert_eq!(steps.last().unwrap().0, "orbe test");
    assert!(
        runtime.daemons().is_empty(),
        "the example closes its session"
    );
}

#[test]
fn the_arduino_sketch_is_the_one_the_prebuilt_firmware_was_built_from() {
    let text = skill("arduino");
    let (_, sketch) = blocks(&text)
        .into_iter()
        .find(|(info, _)| info == "cpp button/button.ino")
        .expect("the arduino skill shows the sketch");
    let built_from = common::repo().join("engine/firmware/sketch/button/button.ino");
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
    let mut want = vec!["check", "add"];
    want.extend(orbe_session::COMMANDS.iter().map(|s| s.name));
    want.extend(["test", "skills", "help"]);
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
    let details = Command::new("arduino-cli")
        .args(["board", "details", "-b", fqbn, "--format", "json"])
        .output()
        .unwrap();
    let details: serde_json::Value = serde_json::from_slice(&details.stdout).unwrap();
    assert_eq!(details["name"], "Arduino Nano", "{fqbn}");

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
        if let Some((_, file)) = info.split_once(' ') {
            fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
            fs::write(dir.join(file), body).unwrap();
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
