//! Commands the `orbe` binary runs, and what they print.

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
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use harness::{Run, Runtime, wait_until};
use serde_json::Value;
use shown::*;

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
    let mut want = vec!["check", "lint", "nets", "add", "part", "search", "blocks"];
    want.extend(orbe_session::COMMANDS.iter().map(|s| s.name));
    want.extend(["show", "test", "skills", "registry", "help"]);
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

    let listed = runtime
        .orbe(here, &["--json", "skills", "list"])
        .ok()
        .json();
    assert_eq!(listed["ok"], true);
    assert!(
        listed["data"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|skill| skill["name"] == "core"),
        "{listed}"
    );
    assert!(runtime.daemons().is_empty(), "help starts no session");
}

#[test]
fn orbe_exits_1_for_a_failed_check_2_for_usage_and_3_for_a_missing_file() {
    let runtime = Runtime::new();
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bad.jsonc"), "{ not json").unwrap();
    assert_eq!(runtime.orbe(dir.path(), &["check", "bad.jsonc"]).code, 1);
    assert_eq!(runtime.orbe(dir.path(), &["step", "2h"]).code, 2);
    assert_eq!(runtime.orbe(dir.path(), &["check", "nope.jsonc"]).code, 3);
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
    let board_name = |fqbn: &str| {
        let details = Command::new("arduino-cli")
            .args(["board", "details", "-b", fqbn, "--format", "json"])
            .output()
            .unwrap();
        let details: serde_json::Value = serde_json::from_slice(&details.stdout).unwrap();
        details["name"].as_str().unwrap_or_default().to_string()
    };
    assert_eq!(board_name(fqbn), "Arduino Nano", "{fqbn}");
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
    let written: BTreeSet<String> = fs::read_dir(dir.join("build"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        written,
        BTreeSet::from([
            "button.ino.hex".to_string(),
            "button.ino.with_bootloader.hex".to_string(),
            "button.ino.with_bootloader.bin".to_string(),
            "button.ino.elf".to_string(),
            "button.ino.eep".to_string(),
        ])
    );

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
    assert!(
        dir.join("build/other.ino.hex").is_file(),
        "the hex follows the sketch folder"
    );
}

/// The Block commands in the arduino skill, run as written on `nano-led-fade`, write the
/// hex its scene already names, and the Block's shipped test passes on that hex.
#[test]
fn the_block_compile_commands_write_the_hex_the_scene_names() {
    let avr = Command::new("arduino-cli").args(["core", "list"]).output();
    if !avr.is_ok_and(|o| {
        o.status.success() && String::from_utf8_lossy(&o.stdout).contains("arduino:avr ")
    }) {
        let skipped = "the_block_compile_commands_write_the_hex_the_scene_names: skipped, arduino-cli with the arduino:avr core is not on PATH\n";
        std::io::stderr().write_all(skipped.as_bytes()).unwrap();
        return;
    }
    let commands = blocks(&skill("arduino"))
        .into_iter()
        .find(|(info, body)| info == "sh" && body.contains("firmware/fade"))
        .map(|(_, body)| body)
        .expect("arduino shows the Block compile commands");
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let runtime = Runtime::new();
    runtime.orbe(dir, &["add", "nano-led-fade"]).ok();
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    assert!(
        scene.contains("\"firmware\": \"build/fade.ino.hex\""),
        "{scene}"
    );
    for line in commands.lines().filter(|line| !line.is_empty()) {
        let built = Command::new("sh")
            .args(["-c", line])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{line}\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
    }
    assert!(dir.join("build/fade.ino.hex").is_file());
    let run = runtime.orbe(dir, &["test"]).ok();
    assert!(
        run.stdout.contains("PASS tests/fade.test\n")
            && run.stdout.ends_with("1 passed, 0 failed\n"),
        "{}",
        run.stdout
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
        "set light 50",
        "get @led1",
        "step 10ms",
        "step --until @led1 lit",
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
}

#[test]
fn the_session_name_comes_from_the_project_folder() {
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
}

#[test]
fn check_reports_part_missing_and_not_simulated_and_ignores_an_edited_part() {
    let project = worked_example();
    let dir = project.path();
    let runtime = Runtime::new();

    let led = dir.join("parts/led-5mm");
    let kept = dir.join("led-5mm.kept");
    fs::rename(&led, &kept).unwrap();
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
    assert_eq!(json["error"]["code"], "part_missing", "{json}");
    fs::rename(&kept, &led).unwrap();

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
    fs::write(dir.join("scene.jsonc"), &scene).unwrap();

    for id in ["led-5mm", "nano-board"] {
        let file = dir.join("parts").join(id).join("part.jsonc");
        let text = fs::read_to_string(&file).unwrap();
        fs::write(&file, format!("{text}\n")).unwrap();
        let run = runtime.orbe(dir, &["check", "scene.jsonc"]);
        assert_eq!(run.code, 0, "{id}: {}", run.stdout);
        assert!(
            !run.stdout.contains("part_modified"),
            "{id}: {}",
            run.stdout
        );
        assert!(!run.stdout.contains("note:"), "{id}: {}", run.stdout);
        fs::write(&file, text).unwrap();
    }
}

/// A flag before the command name runs that command. `--json` prints one JSON envelope,
/// `--session` opens that session, and an `add` writes the folder its `-C` names.
/// The same session name twice counts once. Two names are a usage error.
#[test]
fn a_flag_before_the_command_runs_that_command() {
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
            2,
            None,
            "error[usage]: the argument '--session <NAME>' cannot be used multiple times",
        ),
        (
            "--session a --session a open",
            "open --session a",
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
        let printed = format!("{}{}", run.stdout, run.stderr);
        assert_eq!(run.code, code, "orbe {written}: {printed}");
        assert!(printed.starts_with(says), "orbe {written}: {printed}");
        if let Some(folder) = lock {
            let id = line.split(' ').nth(1).unwrap();
            assert!(
                dir.join(folder)
                    .join("parts")
                    .join(id)
                    .join("part.jsonc")
                    .is_file(),
                "orbe {line} copies {id} into {folder}"
            );
        }
        if code == 0 && written.split_whitespace().any(|word| word == "--json") {
            let reply = one_envelope(&run, &format!("orbe {written}"));
            assert!(reply.get("ok").is_some(), "orbe {written}: {reply}");
        }
        if code == 0 {
            let mut session = None;
            let mut words = written.split_whitespace();
            while let Some(word) = words.next() {
                let name = if word == "--session" {
                    words.next().unwrap()
                } else if let Some(name) = word.strip_prefix("--session=") {
                    name
                } else {
                    continue;
                };
                assert_eq!(session.unwrap_or(name), name, "orbe {written}");
                session = Some(name);
            }
            if let Some(name) = session {
                let socket = format!("{name}/sock");
                assert!(
                    runtime.sockets().iter().any(|sock| sock == &socket),
                    "orbe {written} opens `{name}`, sockets {:?}",
                    runtime.sockets()
                );
            }
        }
    }
    runtime.orbe(dir, &["close", "--all"]).ok();
}

/// `orbe --json open <scene>` prints the open, and nothing else.
#[test]
fn a_flag_before_open_prints_one_json_envelope() {
    let project = worked_example();
    let dir = project.path();
    let runtime = Runtime::new();
    let run = shell(&runtime, dir, "orbe --json open scene.jsonc");
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let reply = one_envelope(&run, "orbe --json open scene.jsonc");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"]["scene"], "scene.jsonc", "{reply}");
    assert_eq!(reply["state"], "paused", "{reply}");
    assert_eq!(reply["t_ps"], 0, "{reply}");
    runtime.orbe(dir, &["close", "--all"]).ok();
}

/// A word that is not a flag is left for clap. A flag in front of `--` stays a flag.
/// The same session name on both sides of a live command counts once, and two names do not.
#[test]
fn only_a_flag_moves_ahead_of_the_command() {
    let project = worked_example();
    let dir = project.path();
    let runtime = Runtime::new();
    let usage = |line: &str| {
        let run = shell(&runtime, dir, &format!("orbe {line}"));
        let printed = format!("{}{}", run.stdout, run.stderr);
        assert_eq!(run.code, 2, "orbe {line}: {printed}");
        let reply = one_envelope(&run, &format!("orbe {line}"));
        assert_eq!(reply["ok"], false, "orbe {line}: {reply}");
        assert_eq!(reply["error"]["code"], "usage", "orbe {line}: {reply}");
        assert!(
            !reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("no scene file"),
            "orbe {line}: {reply}"
        );
        reply
    };
    let help = usage("--json help open");
    assert!(
        help["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unexpected argument '--json'"),
        "{help}"
    );
    let shifted = usage("--json scene.jsonc check");
    assert!(
        shifted["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unexpected argument '--json'"),
        "{shifted}"
    );

    let conflict = shell(&runtime, dir, "orbe --session a open --session b");
    assert_eq!(conflict.code, 2, "{}{}", conflict.stdout, conflict.stderr);
    assert!(conflict.stdout.is_empty(), "{}", conflict.stdout);
    assert!(
        conflict.stderr.starts_with(
            "error[usage]: the argument '--session <NAME>' cannot be used multiple times"
        ),
        "{}",
        conflict.stderr
    );
    assert!(
        runtime
            .sockets()
            .iter()
            .all(|sock| sock != "a/sock" && sock != "b/sock"),
        "a conflicting session opens nothing, sockets {:?}",
        runtime.sockets()
    );

    let same = shell(&runtime, dir, "orbe --session a open --session a");
    assert_eq!(same.code, 0, "{}{}", same.stdout, same.stderr);
    assert!(same.stdout.starts_with("opened "), "{}", same.stdout);
    assert!(
        runtime.sockets().iter().any(|sock| sock == "a/sock"),
        "the one name opens session a, sockets {:?}",
        runtime.sockets()
    );

    let search = shell(&runtime, dir, "orbe --json search -- --foo");
    assert_eq!(search.code, 0, "{}{}", search.stdout, search.stderr);
    let found = one_envelope(&search, "orbe --json search -- --foo");
    assert_eq!(found["ok"], true, "{found}");
    assert!(found["data"].get("groups").is_none(), "{found}");
    assert_eq!(
        found["data"]["parts"].as_array().unwrap().len(),
        0,
        "{found}"
    );
    assert_eq!(
        found["data"]["blocks"].as_array().unwrap().len(),
        0,
        "{found}"
    );

    let missing = shell(&runtime, dir, "orbe --json check -- x");
    assert_eq!(missing.code, 3, "{}{}", missing.stdout, missing.stderr);
    let reply = one_envelope(&missing, "orbe --json check -- x");
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["error"]["code"], "io_error", "{reply}");
    assert_eq!(reply["error"]["message"], "no scene file at x", "{reply}");

    runtime.orbe(dir, &["close", "--all"]).ok();
}

/// Each input on a bouncing switch lands just after the window of the one before, and a read
/// past the last input's time plus the window sees the switch settled. Two windows, with a
/// press last and a release last.
#[test]
fn a_bouncing_switch_lands_after_the_previous_window() {
    let (_, diff) = blocks(&skill("sessions-and-time"))
        .into_iter()
        .find(|(info, _)| info == "diff scene.jsonc")
        .expect("sessions-and-time shows its bounce change");
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
                Some(&"press") => ("high", "lit"),
                _ => ("low", "off"),
            };
            orbe(&["expect", "@nano.D2", pin]);
            now = orbe(&["step", "20ms"])["t_ps"].as_u64().unwrap();
            orbe(&["expect", "@led1", led]);
        }
        orbe(&["close"]);
    }
}

#[test]
fn a_command_sent_while_a_step_runs_waits_for_it() {
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
fn a_failed_expect_names_the_line_and_the_actual_state() {
    let test = blocks(&skill("core"))
        .into_iter()
        .find(|(info, _)| info == "text tests/button.test")
        .unwrap()
        .1;
    let project = worked_example();
    let dir = project.path();
    let failing = test.replacen("expect @led1 off", "expect @led1 lit", 1);
    write_file(dir, "tests/button.test", &failing);
    let run = Runtime::new().orbe(dir, &["test"]);
    assert_eq!(run.code, 1);
    let printed = format!("{}{}", run.stdout, run.stderr);
    assert!(printed.contains("FAIL tests/button.test:"), "{printed}");
    assert!(printed.contains("error[expect_failed]:"), "{printed}");
    assert!(printed.contains("expected: @led1 lit"), "{printed}");
    assert!(printed.contains("led1 off"), "{printed}");
    assert!(printed.contains("0 passed, 1 failed"), "{printed}");
}

#[test]
fn a_load_over_the_usb_supply_warns_and_a_wire_across_it_is_a_short() {
    let project = worked_example();
    let dir = project.path();
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    let cathode = "[\"led1.cathode\", \"nano.GND\"],";
    let loaded = scene
        .replace(
            "\"led1\": {",
            "\"rl\": { \"type\": \"resistor-axial\", \"resistance\": \"2.5\" },\n    \"led1\": {",
        )
        .replace(
            cathode,
            &format!("{cathode} [\"nano.5V\", \"rl.a\"], [\"rl.b\", \"nano.GND\"],"),
        );
    assert!(loaded.contains("\"rl.a\"") && loaded.contains("\"rl\": {"));
    fs::write(dir.join("scene.jsonc"), &loaded).unwrap();
    let runtime = Runtime::new();
    let orbe = |args: &[&str]| runtime.orbe(dir, args);
    let open = orbe(&["open", "--json"]).ok().json();
    let over = |part: &str| {
        open["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| {
                d["code"] == "rating.current" && d["message"].as_str().unwrap().starts_with(part)
            })
            .unwrap_or_else(|| panic!("{open}"))
            .clone()
    };
    // The same current passes the USB supply and the Schottky behind 5V, each rated 0.5 A. The
    // supply's rating is the RMS over 10.2 ms, so at open it is over now. The diode's is checked
    // on each sample.
    let supply = over("#nano__usb__host.i (nano.usb) ");
    let diode = over("#nano__diode.i (nano.diode) ");
    orbe(&["close"]).ok();
    assert_eq!(supply["data"]["now"], true, "{supply}");
    let sample = supply["data"]["sample"].as_f64().unwrap();
    assert!(sample > 1.5, "{sample}");
    assert!(diode["data"]["peak"].as_f64().unwrap() > 1.5, "{diode}");
    assert_eq!(supply["data"]["limit"], 0.5);
    assert_eq!(diode["data"]["limit"], 0.5);

    let shorted = scene.replace(cathode, &format!("{cathode} [\"nano.5V\", \"nano.GND\"],"));
    fs::write(dir.join("scene.jsonc"), &shorted).unwrap();
    let check = orbe(&["check", "scene.jsonc"]);
    assert_eq!(check.code, 1);
    for said in [
        "error[net.short]: the wire from `nano.5V` to `nano.GND` shorts the supply `nano.usb`, which holds the two apart",
        "hint: remove the wire; a load from `nano.5V` to `nano.GND` needs a part that limits its current, such as a resistor",
    ] {
        assert!(check.stdout.contains(said), "{}", check.stdout);
    }
    assert_eq!(orbe(&["open"]).code, 1);
}

#[test]
fn a_button_with_no_pull_down_warns_that_its_input_floats() {
    let project = worked_example();
    let dir = project.path();
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    let bare: String = scene
        .lines()
        .filter(|l| !l.contains("\"r2"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert!(bare.contains("\"btn1\"") && !bare.contains("r2"));
    fs::write(dir.join("scene.jsonc"), &bare).unwrap();
    let runtime = Runtime::new();
    let orbe = |args: &[&str]| runtime.orbe(dir, args);
    orbe(&["open"]).ok();
    let step = orbe(&["step", "20ms"]).ok();
    orbe(&["close"]).ok();
    let said =
        "warning[chip.floating_input] #nano__mcu.pin.PD2 (nano.D2) floats on #net.nano__D2 at ";
    assert!(step.stdout.contains(said), "{}", step.stdout);
    assert!(
        step.stdout
            .contains("add a pull-up or a pull-down resistor, or turn the pin's pull-up on"),
        "{}",
        step.stdout
    );

    fs::write(dir.join("scene.jsonc"), &scene).unwrap();
    orbe(&["open"]).ok();
    let step = orbe(&["step", "20ms"]).ok();
    orbe(&["close"]).ok();
    assert!(
        !step.stdout.contains("chip.floating_input"),
        "{}",
        step.stdout
    );
}

#[test]
fn an_led_with_no_series_resistor_warns_and_holds_its_pin_low() {
    let project = worked_example();
    let dir = project.path();
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
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
    // Each rating warning a step lists, as `(code, ref, over now)`.
    let listed = |step: &Value| -> Vec<(String, String, bool)> {
        step["data"]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|change| change["kind"] == "diagnostic")
            .map(|change| &change["diagnostic"])
            .filter(|d| d["code"].as_str().unwrap().starts_with("rating."))
            .map(|d| {
                (
                    d["code"].as_str().unwrap().to_string(),
                    d["refs"][0].as_str().unwrap().to_string(),
                    d["data"]["now"].as_bool().unwrap(),
                )
            })
            .collect()
    };
    let owned = |code: &str, r: &str, now: bool| (code.to_string(), r.to_string(), now);
    let step = orbe(&[
        "step",
        "--until",
        "@led1",
        "lit",
        "--timeout",
        "50ms",
        "--json",
    ])
    .ok()
    .json();
    // At the pin's edge the LED carries about 94 mA, over its 30 mA rating on each sample.
    // The pin's 40 mA and the LED's 75 mW are over 10.2 ms, so they are over now.
    let mut found = listed(&step);
    found.sort();
    assert_eq!(
        found,
        [
            owned("rating.current", "#led1.i", false),
            owned("rating.current", "#nano__mcu.pin.PB5", true),
            owned("rating.power", "#led1.p", true),
        ],
        "{step}"
    );
    // The pin's RMS over the last 10.2 ms crosses 40 mA (40 / 94)^2 of the window after the
    // press, 1.8 ms. The LED's power average crosses 75 mW about 3.5 ms in.
    let held = orbe(&["step", "20ms", "--json"]).ok().json();
    let mut found = listed(&held);
    found.sort();
    assert_eq!(
        found,
        [
            owned("rating.current", "#nano__mcu.pin.PB5", false),
            owned("rating.power", "#led1.p", false),
        ],
        "{held}"
    );
    let led = orbe(&["get", "@led1", "--json"]).ok().json()["data"]["value"].clone();
    let pin = orbe(&["get", "@nano.D13", "--json"]).ok().json()["data"]["value"].clone();
    assert_eq!(pin["level"], "low", "{pin}");
    assert_eq!(led["state"], "lit", "{led}");
    let current_ma = led["i"].as_f64().unwrap() * 1e3;
    assert!(current_ma > 40.0, "{current_ma}");
    let volts = pin["v"].as_f64().unwrap();
    assert!((2.0..=2.6).contains(&volts), "{volts}");
    let expect = orbe(&["expect", "no", "diagnostics", "--json"]);
    assert_eq!(expect.code, 1);
    assert_eq!(
        expect.json()["data"]["actual"],
        "rating.current, rating.current, rating.power",
        "{}",
        expect.stdout
    );
    orbe(&["close"]).ok();

    let card = runtime.orbe(dir, &["part", "led-5mm"]).ok().stdout;
    assert!(
        card.contains("ratings\n  current  30 mA\n  power  75 mW\n"),
        "{card}"
    );
}

#[test]
fn a_resistor_over_its_power_rating_warns() {
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
    let open = runtime
        .orbe(dir, &["open", "hot.jsonc", "--json"])
        .ok()
        .json();
    let [power] = open["data"]["diagnostics"].as_array().unwrap().as_slice() else {
        panic!("{open}");
    };
    // 10 ohm on 5V is about 2.16 W from the first step, over the 0.25 W rating now. Held, the
    // average over 10.2 ms crosses 0.25 W in about 1.2 ms.
    assert_eq!(power["code"], "rating.power");
    assert_eq!(power["refs"][0], "#r9.p");
    let data = &power["data"];
    assert_eq!(data["now"], true, "{power}");
    assert!(
        (data["sample"].as_f64().unwrap() - 2.158).abs() < 1e-3,
        "{power}"
    );
    let held = data["held_ps"].as_f64().unwrap();
    assert!((held / 1.18e9 - 1.0).abs() < 0.02, "{power}");
    runtime.orbe(dir, &["close"]).ok();
    let card = runtime.orbe(dir, &["part", "resistor-axial"]).ok().stdout;
    assert!(card.contains("ratings\n  power  250 mW\n"), "{card}");
}

#[test]
fn part_readings_show_a_pin_edge_where_until_stops_and_led_l_follows_d13() {
    let project = worked_example();
    let dir = project.path();
    let runtime = Runtime::new();
    let orbe = |args: &[&str]| runtime.orbe(dir, args).ok();
    orbe(&["open"]);
    orbe(&["step", "100ms"]);
    for (input, level, before, after, cmp) in [
        ("press", "high", "off", "lit", ">"),
        ("release", "low", "lit", "off", "<"),
    ] {
        for led in ["@led1", "@nano.led_l"] {
            orbe(&["expect", led, before]);
        }
        orbe(&[input, "@btn1"]);
        orbe(&["step", "--until", "@nano.D13", level]);
        for led in ["@led1", "@nano.led_l"] {
            orbe(&["expect", led, before]);
        }
        let bound = if cmp == ">" { "5mA" } else { "100uA" };
        orbe(&["expect", "@led1.i", cmp, bound]);
        orbe(&["step", "20ms"]);
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
    wait_until("orbe test leaves no session", || {
        runtime.daemons().is_empty()
    });
    let close = runtime.orbe(dir, &["close", "--json"]);
    assert_eq!(close.code, 1);
    assert_eq!(close.json()["error"]["code"], "no_session");
}

#[test]
fn expect_no_diagnostics_names_how_late_a_chip_warning_can_show() {
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
/// own with no model, a turn past 180 degrees, and the resistor left over plugged into one strip of
/// a breadboard. Each code `orbe lint` prints is in the ratings skill's warnings table, the table
/// names no other, and `orbe open` shows only `net.floating`.
#[test]
fn the_warnings_table_lists_the_codes_lint_prints_and_open_shows_only_the_engines() {
    let project = worked_example();
    let dir = project.path();
    add_parts(dir, &["breadboard-400"]);
    let scene = fs::read_to_string(dir.join("scene.jsonc")).unwrap();
    let broken = scene
        .replace("[\"nano.D13\", \"r1.a\"]", "[\"nano.D13\", \"led1.anode\"]")
        .replace("[\"r1.b\", \"led1.anode\"],", "")
        .replace(
            "\"parts\": {",
            "\"parts\": {\n    \"jump\": { \"type\": \"./jumper.jsonc\" },\n    \"bb\": { \"type\": \"breadboard-400\" },",
        )
        .replace(
            "\"wires\": [",
            "\"layout\": {\n    \"btn1\": { \"at\": [0, 0, 0], \"rotate\": [0, 0, 270] },\n    \"bb\": { \"at\": [0, 60, 0] },\n    \"r1\": { \"on\": { \"a\": \"bb.a5\" }, \"rotate\": [0, 0, 90] },\n  },\n  \"wires\": [",
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
    let ratings = skill("ratings");
    let table: BTreeSet<String> = ratings
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
}

/// `orbe skills get core` starts from a Block, and `orbe part` prints a breadboard's holes as
/// ranges, a part's pins, params and an LED's brightness ranges.
#[test]
fn orbe_part_prints_the_holes_pins_and_readings() {
    let runtime = Runtime::new();
    let part = |id: &str| runtime.orbe(Path::new("."), &["part", id]).ok().stdout;
    for (id, rows) in [
        (
            "breadboard-400",
            "rows a to j at columns 1 to 30; rows W to Z at columns 1 to 5, 7 to 11",
        ),
        (
            "breadboard-830",
            "rows a to j at columns 1 to 63; rows W to Z at columns 3 to 7, 9 to 13",
        ),
    ] {
        let card = part(id);
        let holes = card
            .lines()
            .skip_while(|line| !line.starts_with("holes "))
            .nth(1)
            .unwrap();
        assert!(holes.contains(rows), "{holes}");
        assert!(
            holes.len() < 400,
            "{id} holes line is {} bytes",
            holes.len()
        );
    }
    let led = part("led-5mm");
    assert!(led.contains("pins\n  anode  1\n  cathode  2\n"), "{led}");
    assert!(
        led.contains(
            "  lit while that average is above 2 mA\n  dim while that average is above 100 uA and at or below 2 mA\n  off while that average is at or below 100 uA\n"
        ),
        "{led}"
    );
    assert!(part("nano-board").contains("\nNano (CH340 clone)\n"));
    let trimmer = part("trimmer-3362p");
    assert!(
        trimmer.contains(
            "  resistance: a value in ohm above 0, required, example 10 kohm\n  position: a number above 0 and below 1, 0.5 by default, and `set` changes it while a session runs\n"
        ),
        "{trimmer}"
    );
}
