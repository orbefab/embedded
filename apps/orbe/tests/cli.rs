//! `orbe check` and `orbe add` on a project built from the reader's fixture.

#[path = "../../../packages/scene/tests/common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
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

fn orbe_in(dir: &Path, args: &[&str]) -> (Output, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    (output, stdout, stderr)
}

/// A folder with a scene and nothing else.
fn scene_only() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("scene.jsonc"), "{}\n").unwrap();
    dir
}

/// A fresh first-target project at `<workspace>/proj`, with its firmware and no parts.
fn first_target() -> (tempfile::TempDir, PathBuf) {
    let workspace = tempfile::tempdir().unwrap();
    let proj = workspace.path().join("proj");
    fs::create_dir_all(proj.join("build")).unwrap();
    fs::copy(
        common::fixture("nano-button").join("scene.jsonc"),
        proj.join("scene.jsonc"),
    )
    .unwrap();
    fs::copy(common::firmware(), proj.join("build/button.ino.hex")).unwrap();
    (workspace, proj)
}

/// Every `orbe add` command in the hint of a `part_missing` or `part_modified` finding.
fn hint_commands(stdout: &str) -> Vec<String> {
    let lines: Vec<&str> = stdout.lines().collect();
    let mut commands = Vec::new();
    for pair in lines.windows(2) {
        if !(pair[0].contains("error[part_missing]") || pair[0].contains("error[part_modified]")) {
            continue;
        }
        let hint = pair[1]
            .strip_prefix("  hint: ")
            .unwrap_or_else(|| panic!("{stdout}"));
        let at = hint
            .find("`orbe add ")
            .unwrap_or_else(|| panic!("no command in {hint}"));
        let rest = &hint[at + 1..];
        let command = &rest[..rest.find('`').unwrap()];
        if !commands.iter().any(|c| c == command) {
            commands.push(command.to_string());
        }
    }
    commands
}

/// Runs a printed command as a shell would split it. The paths here have no quotes or spaces.
fn run_printed(cwd: &Path, command: &str) -> (Output, String, String) {
    assert!(!command.contains(['\'', '"']), "{command}");
    let words: Vec<&str> = command.split_whitespace().collect();
    assert_eq!(&words[..2], ["orbe", "add"], "{command}");
    orbe_in(cwd, &words[1..])
}

/// Rewrites `id`'s lock line so it matches the folder now on disk.
fn relock(project: &Path, id: &str) {
    let lock = project.join("parts.lock");
    let lines: String = fs::read_to_string(&lock)
        .unwrap()
        .lines()
        .map(|line| {
            if line.split_whitespace().next() == Some(id) {
                common::lock_line(project, id)
            } else {
                line.to_string()
            }
        })
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(&lock, lines).unwrap();
}

/// Writes `title` into one copy and rewrites its lock line so the copy matches that line.
fn retitle(project: &Path, id: &str, title: &str) {
    let path = project.join("parts").join(id).join("part.jsonc");
    let text = fs::read_to_string(&path).unwrap();
    let marker = "\"title\": \"";
    let start = text.find(marker).expect("title") + marker.len();
    let end = start + text[start..].find('"').expect("title end");
    let edited = format!("{}{title}{}", &text[..start], &text[end..]);
    assert_ne!(text, edited, "{id}");
    fs::write(&path, edited).unwrap();
    relock(project, id);
}

/// Writes `version` into one copy and rewrites its lock line so the copy matches that line.
fn hold(project: &Path, id: &str, version: &str) {
    let file = project.join("parts").join(id).join("part.jsonc");
    let text = fs::read_to_string(&file).unwrap();
    let edited = text.replacen(
        "\"version\": \"0.1.0\"",
        &format!("\"version\": \"{version}\""),
        1,
    );
    assert_ne!(text, edited, "{id}");
    fs::write(&file, edited).unwrap();
    relock(project, id);
}

/// The one `orbe add` command in a `part_missing` hint. Every hint names the same command.
fn add_command(stdout: &str) -> String {
    let commands: Vec<String> = stdout
        .lines()
        .filter_map(|line| {
            let at = line.rfind("orbe add ")?;
            Some(line[at..].trim().trim_end_matches('`').to_string())
        })
        .collect();
    assert!(!commands.is_empty(), "{stdout}");
    assert!(
        commands.iter().all(|command| command == &commands[0]),
        "{commands:?}"
    );
    commands[0].clone()
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
fn json_with_a_value_is_still_one_envelope() {
    let (output, stdout) = orbe(&["check", "--json=true"]);
    assert_eq!(output.status.code(), Some(2), "{stdout}");
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "usage");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
}

#[test]
fn a_deleted_copy_prints_one_part_missing() {
    let project = common::project("nano-button");
    fs::remove_dir_all(project.path().join("parts/led-5mm")).unwrap();
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        "scene.jsonc:9:23: error[part_missing]: part `led-5mm` has a parts.lock line but no folder\n  hint: run `orbe add led-5mm`\n1 error in scene.jsonc\n"
    );
}

#[test]
fn help_is_not_a_usage_envelope() {
    let (output, stdout) = orbe(&["check", "--help"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("check"), "{stdout}");
}

#[test]
fn every_bad_fixture_exits_1_and_prints_its_findings() {
    for case in common::cases("bad") {
        let name = case.file_name().unwrap().to_string_lossy().to_string();
        let project = common::overlaid(&case);
        let scene = project.path().join("scene.jsonc");
        let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(1), "{name}: {stdout}");
        let printed: Vec<&str> = stdout.lines().collect();
        for line in fs::read_to_string(case.join("expect")).unwrap().lines() {
            let mut fields = line.splitn(3, " | ");
            let (code, place) = fields.next().unwrap().split_once(' ').unwrap();
            let head = format!("{}/{place}: error[{code}]: ", project.path().display());
            let at = printed
                .iter()
                .position(|l| l.starts_with(&head))
                .unwrap_or_else(|| panic!("{name}: no `{head}` in\n{stdout}"));
            if let Some(message) = fields.next() {
                assert!(printed[at].contains(message), "{name}: {}", printed[at]);
            }
            let hint = printed[at + 1]
                .strip_prefix("  hint: ")
                .unwrap_or_else(|| panic!("{name}: no hint after {}", printed[at]));
            assert!(!hint.is_empty(), "{name}: an empty hint");
            if let Some(want) = fields.next() {
                assert!(hint.contains(want), "{name}: {hint}");
            }
        }
    }
}

#[test]
fn every_good_fixture_exits_0_with_its_part_count() {
    for case in common::cases("good") {
        let name = case.file_name().unwrap().to_string_lossy().to_string();
        let parts = fs::read_to_string(case.join("expect")).unwrap();
        let parts = parts.lines().next().unwrap();
        let project = common::overlaid(&case);
        let scene = project.path().join("scene.jsonc");
        let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(0), "{name}: {stdout}");
        assert!(
            stdout.contains(&format!("ok, {parts}, ")),
            "{name}: {stdout}"
        );
    }
}

#[test]
fn adding_parts_for_the_first_target() {
    let dir = tempfile::tempdir().unwrap();
    fs::copy(
        common::fixture("nano-button").join("scene.jsonc"),
        dir.path().join("scene.jsonc"),
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("build")).unwrap();
    fs::copy(common::firmware(), dir.path().join("build/button.ino.hex")).unwrap();
    let scene = dir.path().join("scene.jsonc");

    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(stdout.matches("error[part_missing]").count(), 4, "{stdout}");
    for (place, id) in [
        ("5:23", "nano-board"),
        ("6:23", "tactile-switch-6mm"),
        ("7:21", "resistor-axial"),
        ("9:23", "led-5mm"),
    ] {
        assert!(
            stdout.contains(&format!(
                "{}:{place}: error[part_missing]: part `{id}`",
                scene.display()
            )),
            "{stdout}"
        );
    }
    assert!(!stdout.contains("did you mean"), "{stdout}");
    let command = add_command(&stdout);
    assert_eq!(
        command,
        format!(
            "orbe add -C {} led-5mm nano-board resistor-axial tactile-switch-6mm",
            dir.path().display()
        )
    );

    let args: Vec<&str> = command.split_whitespace().skip(1).collect();
    let (output, stdout, _) = orbe_in(dir.path(), &args);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("wrote parts/"))
            .count(),
        18,
        "{stdout}"
    );
    assert!(stdout.contains("wrote parts.lock"), "{stdout}");
    let mut copied = Vec::new();
    for entry in fs::read_dir(dir.path().join("parts")).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.path().is_dir(), "{}", entry.path().display());
        copied.push(entry.file_name().into_string().unwrap());
    }
    copied.sort();
    assert_eq!(copied.len(), 18);
    for id in &copied {
        let names = [
            "part.jsonc".to_string(),
            "asset.json".to_string(),
            format!("{id}.glb"),
            format!("{id}.anchors.json"),
        ];
        for name in names {
            let catalog = common::repo().join("parts").join(id).join(&name);
            let copy = dir.path().join("parts").join(id).join(&name);
            if catalog.exists() || name.ends_with(".glb") {
                assert_eq!(fs::read(&copy).ok(), fs::read(&catalog).ok(), "{id}/{name}");
            }
        }
    }
    let fixture_lock: String = copied
        .iter()
        .map(|id| format!("{}\n", common::lock_line(&common::repo(), id)))
        .collect();
    let lock_path = dir.path().join("parts.lock");
    assert_eq!(fs::read_to_string(&lock_path).unwrap(), fixture_lock);
    for id in &copied {
        assert!(
            stdout.contains(&format!("wrote parts/{id} 0.1.0")),
            "{id}: {stdout}"
        );
    }

    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("ok, 18 engine parts, 31 nets, ground nano__GND"),
        "{stdout}"
    );

    let led = dir.path().join("parts/led-5mm/part.jsonc");
    let led_bytes = fs::read(&led).unwrap();
    let led_stamp = fs::metadata(&led).unwrap().modified().unwrap();
    let lock_bytes = fs::read(&lock_path).unwrap();
    let (output, stdout, _) = orbe_in(dir.path(), &args);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("skipped parts/"))
            .count(),
        18,
        "{stdout}"
    );
    assert!(stdout.contains("parts.lock unchanged"), "{stdout}");
    assert!(
        !stdout.lines().any(|line| line.starts_with("wrote ")),
        "{stdout}"
    );
    assert_eq!(fs::read(&led).unwrap(), led_bytes);
    assert_eq!(fs::read(&lock_path).unwrap(), lock_bytes);
    assert_eq!(fs::metadata(&led).unwrap().modified().unwrap(), led_stamp);

    let mut json_args = vec!["add", "--json"];
    json_args.extend(args.iter().copied().skip(1));
    let (output, stdout, stderr) = orbe_in(dir.path(), &json_args);
    assert_eq!(output.status.code(), Some(0), "{stdout} {stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["data"]["written"].as_array().unwrap().len(), 0);
    assert_eq!(reply["data"]["skipped"].as_array().unwrap().len(), 18);
    assert_eq!(reply["data"]["lock_written"], false);
    assert_eq!(fs::metadata(&led).unwrap().modified().unwrap(), led_stamp);

    let text = fs::read_to_string(&led).unwrap();
    fs::write(&led, text.replace("5 mm LED", "5 mm LED edited")).unwrap();
    let (output, stdout, _) = orbe_in(dir.path(), &["add", "led-5mm"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("nothing was written"), "{stdout}");
    assert!(stdout.contains("`part.jsonc`"), "{stdout}");
    assert!(
        fs::read_to_string(&led)
            .unwrap()
            .contains("5 mm LED edited")
    );
    let (output, stdout, stderr) = orbe_in(dir.path(), &["add", "--json", "led-5mm"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "part_differs");

    let (output, stdout, _) = orbe_in(dir.path(), &["add", "led-5mm", "--overwrite"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("wrote parts/led-5mm 0.1.0"), "{stdout}");
    let restored = fs::read_to_string(&led).unwrap();
    assert!(restored.contains("\"title\": \"5 mm LED\""));
    assert!(!restored.contains("edited"));
    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("ok, 18 engine parts, 31 nets, ground nano__GND"),
        "{stdout}"
    );

    let nano = dir.path().join("parts/nano-board/part.jsonc");
    let lock_before = fs::read(&lock_path).unwrap();
    let nano_text = fs::read_to_string(&nano).unwrap();
    fs::write(
        &nano,
        nano_text.replace("Nano (CH340 clone)", "Nano (CH340 clone) edited"),
    )
    .unwrap();
    let (output, stdout, _) = orbe_in(dir.path(), &["add", "nano-board", "--diff"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("stays at 0.1.0"), "{stdout}");
    assert!(stdout.contains("the version is unchanged"), "{stdout}");
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with('-') && line.contains("edited")),
        "{stdout}"
    );
    let plus = stdout
        .lines()
        .find(|line| line.starts_with('+') && line.contains("Nano (CH340 clone)"))
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(!plus.contains("edited"), "{plus}");
    assert!(fs::read_to_string(&nano).unwrap().contains("edited"));
    assert_eq!(fs::read(&lock_path).unwrap(), lock_before);
    assert!(!dir.path().join(".orbe-add-staging").exists());

    let lock_text = fs::read_to_string(&lock_path).unwrap();
    let first = lock_text.lines().next().unwrap();
    let (fields, _) = first.rsplit_once(" sha256:").unwrap();
    let wrong = format!("{fields} sha256:{}", "0".repeat(64));
    fs::write(&lock_path, lock_text.replacen(first, &wrong, 1)).unwrap();
    let (output, stdout, _) = orbe_in(dir.path(), &["add", "--relock"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("wrote parts.lock"), "{stdout}");
    assert!(stdout.contains("`nano-board` is modified"), "{stdout}");
    let lock = fs::read_to_string(&lock_path).unwrap();
    assert!(!lock.contains(&wrong), "{lock}");
    let hash = orbe_scene::folder_hash(nano.parent().unwrap()).unwrap();
    let nano_line = lock
        .lines()
        .find(|line| line.starts_with("nano-board "))
        .unwrap();
    assert!(nano_line.ends_with(&hash), "{nano_line}\n{hash}");
    for line in fixture_lock.lines() {
        if line.starts_with("nano-board ") {
            assert_ne!(line, nano_line);
            continue;
        }
        assert!(
            lock.lines().any(|kept| kept == line),
            "missing {line}\n{lock}"
        );
    }
    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("ok, 18 engine parts, 31 nets, ground nano__GND"),
        "{stdout}"
    );
    assert!(
        stdout.contains("note: `nano-board` differs from its parts.lock line"),
        "{stdout}"
    );

    let lock_bytes = fs::read(&lock_path).unwrap();
    let (output, stdout, _) = orbe_in(dir.path(), &["add", "--relock"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("parts.lock unchanged"), "{stdout}");
    assert_eq!(fs::read(&lock_path).unwrap(), lock_bytes);
}

#[test]
fn an_unknown_id_names_a_close_catalog_id() {
    let dir = scene_only();
    let (output, stdout, _) = orbe_in(dir.path(), &["add", "led-5m"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("did you mean"), "{stdout}");
    assert!(stdout.contains("`led-5mm`"), "{stdout}");
    assert!(!dir.path().join("parts").exists());
    assert!(!dir.path().join("parts.lock").exists());
}

#[test]
fn a_registry_id_says_v1_reads_the_official_catalog() {
    let dir = scene_only();
    let (output, stdout, _) = orbe_in(dir.path(), &["add", "@acme/led-5mm"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("v1 reads only the official catalog"),
        "{stdout}"
    );
    assert!(!dir.path().join("parts.lock").exists());
}

#[test]
fn add_usage_is_exit_2_and_json_stays_on_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let (output, _, stderr) = orbe_in(dir.path(), &["add"]);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("error[usage]"), "{stderr}");
    let (output, _, stderr) = orbe_in(dir.path(), &["add", "led-5mm@1"]);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("three numbers"), "{stderr}");
    let (output, _, stderr) = orbe_in(dir.path(), &["add", "--relock", "led-5mm"]);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("--relock"), "{stderr}");
    let (output, stdout, stderr) = orbe_in(dir.path(), &["add", "--json"]);
    assert_eq!(output.status.code(), Some(2), "{stdout} {stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "usage");
}

#[test]
fn a_folder_with_no_lock_line_is_part_missing() {
    let project = common::project("nano-button");
    let lock = project.path().join("parts.lock");
    let text = fs::read_to_string(&lock).unwrap();
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !line.starts_with("led-5mm "))
        .collect();
    fs::write(&lock, format!("{}\n", kept.join("\n"))).unwrap();
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("part `led-5mm` is not in this project's parts.lock"),
        "{stdout}"
    );
    assert!(stdout.contains("run `orbe add led-5mm`"), "{stdout}");
}

#[test]
fn add_refuses_a_part_id_locked_twice_and_relock_keeps_one() {
    let project = common::project("nano-button");
    let lock = project.path().join("parts.lock");
    let mut text = fs::read_to_string(&lock).unwrap();
    let line = text
        .lines()
        .find(|line| line.starts_with("led-5mm "))
        .unwrap()
        .replace(orbe_scene::REGISTRY, "@acme");
    text.push_str(&format!("{line}\n"));
    fs::write(&lock, &text).unwrap();
    let (output, stdout, _) = orbe_in(project.path(), &["add", "led-5mm"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("second line for `led-5mm`"), "{stdout}");
    assert!(stdout.contains("--relock"), "{stdout}");
    assert_eq!(
        fs::read_to_string(&lock).unwrap(),
        text,
        "nothing was written"
    );
    let (output, stdout, _) = orbe_in(project.path(), &["add", "--relock"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    let relocked = fs::read_to_string(&lock).unwrap();
    assert_eq!(
        relocked
            .lines()
            .filter(|l| l.starts_with("led-5mm "))
            .count(),
        1,
        "{relocked}"
    );
}

#[test]
fn the_first_target_walk_from_the_folder_above() {
    let (workspace, proj) = first_target();
    let (output, stdout, _) = orbe_in(workspace.path(), &["check", "proj/scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(stdout.matches("error[part_missing]").count(), 4, "{stdout}");
    assert!(!stdout.contains("did you mean"), "{stdout}");
    let command = add_command(&stdout);
    assert_eq!(
        command,
        "orbe add -C proj led-5mm nano-board resistor-axial tactile-switch-6mm"
    );
    let (output, stdout, stderr) = run_printed(workspace.path(), &command);
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    assert_eq!(stdout.matches("wrote parts/").count(), 18, "{stdout}");
    assert!(!workspace.path().join("parts").exists());
    assert!(!workspace.path().join("parts.lock").exists());
    let (output, stdout, _) = orbe_in(workspace.path(), &["check", "proj/scene.jsonc"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_eq!(
        stdout,
        "proj/scene.jsonc: ok, 18 engine parts, 31 nets, ground nano__GND\n"
    );
    assert!(proj.join("parts.lock").is_file());
}

#[test]
fn every_part_missing_and_part_modified_hint_makes_the_next_check_pass() {
    let set_version = |project: &Path, to: &str| {
        let path = project.join("parts/led-5mm/part.jsonc");
        let text = fs::read_to_string(&path).unwrap();
        let edited = text.replacen(
            "\"version\": \"0.1.0\"",
            &format!("\"version\": \"{to}\""),
            1,
        );
        assert_ne!(text, edited);
        fs::write(&path, edited).unwrap();
    };
    let drop_lock_line = |project: &Path| {
        let lock = project.join("parts.lock");
        let kept: String = fs::read_to_string(&lock)
            .unwrap()
            .lines()
            .filter(|line| !line.starts_with("led-5mm "))
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(&lock, kept).unwrap();
    };
    let edit_title = |project: &Path| {
        let path = project.join("parts/led-5mm/part.jsonc");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.replace("5 mm LED", "5 mm LED edited")).unwrap();
    };
    type Break = Box<dyn Fn(&Path)>;
    let cases: Vec<(&str, &str, Break)> = vec![
        (
            "no parts at all",
            "is not in this project's parts.lock",
            Box::new(|p: &Path| {
                fs::remove_dir_all(p.join("parts")).unwrap();
                fs::remove_file(p.join("parts.lock")).unwrap();
            }),
        ),
        (
            "a deleted folder",
            "has a parts.lock line but no folder",
            Box::new(|p: &Path| {
                fs::remove_dir_all(p.join("parts/led-5mm")).unwrap();
            }),
        ),
        (
            "no lock line",
            "is not in this project's parts.lock",
            Box::new(move |p: &Path| drop_lock_line(p)),
        ),
        (
            "no lock line and an edit",
            "is not in this project's parts.lock",
            Box::new(move |p: &Path| {
                drop_lock_line(p);
                edit_title(p);
            }),
        ),
        (
            "a folder with no part.jsonc",
            "has a folder, parts/led-5mm, with no part.jsonc",
            Box::new(|p: &Path| {
                fs::remove_file(p.join("parts/led-5mm/part.jsonc")).unwrap();
            }),
        ),
        (
            "a file at the folder's place",
            "is parts/led-5mm, which is a file, not a folder",
            Box::new(|p: &Path| {
                fs::remove_dir_all(p.join("parts/led-5mm")).unwrap();
                fs::write(p.join("parts/led-5mm"), "notes\n").unwrap();
            }),
        ),
        (
            "a sealed edit",
            "differs from `led-5mm@0.1.0` in `title`",
            Box::new(move |p: &Path| edit_title(p)),
        ),
        (
            "a version in another major",
            "differs from `led-5mm@0.1.0` in `version`",
            Box::new(move |p: &Path| set_version(p, "1.0.0")),
        ),
        (
            "a lock line at another version",
            "parts.lock lists `led-5mm` at `0.2.0`",
            Box::new(|p: &Path| {
                let lock = p.join("parts.lock");
                let text = fs::read_to_string(&lock).unwrap();
                fs::write(&lock, text.replacen("led-5mm 0.1.0 ", "led-5mm 0.2.0 ", 1)).unwrap();
            }),
        ),
    ];
    for (name, message, break_it) in cases {
        let project = common::project("nano-button");
        break_it(project.path());
        let above = project.path().parent().unwrap();
        let folder = project.path().file_name().unwrap().to_str().unwrap();
        let scene = format!("{folder}/scene.jsonc");
        let (output, stdout, _) = orbe_in(above, &["check", &scene]);
        assert_eq!(output.status.code(), Some(1), "{name}: {stdout}");
        assert!(stdout.contains(message), "{name}: {stdout}");
        let commands = hint_commands(&stdout);
        assert!(!commands.is_empty(), "{name}: {stdout}");
        for command in &commands {
            assert!(
                command.starts_with(&format!("orbe add -C {folder} ")),
                "{name}: {command}"
            );
            let (output, out, err) = run_printed(above, command);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{name}: {command}\n{out}{err}"
            );
        }
        let (output, stdout, _) = orbe_in(above, &["check", &scene]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {commands:?}\n{stdout}"
        );
        assert!(stdout.contains("ok, 18 engine parts"), "{name}: {stdout}");
        let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
        assert_eq!(output.status.code(), Some(0), "{name}: {stdout}");
    }
}

#[test]
fn an_outdated_copy_names_one_add_and_a_hand_edit_stays_modified() {
    let project = common::project("nano-button");
    hold(project.path(), "led-5mm", "0.0.9");
    hold(project.path(), "resistor-axial", "0.0.8");
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("error[part_outdated]"), "{stdout}");
    assert!(
        stdout.contains("this orbe carries `led-5mm@0.1.0` and the project holds `led-5mm@0.0.9`"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "this orbe carries `resistor-axial@0.1.0` and the project holds `resistor-axial@0.0.8`"
        ),
        "{stdout}"
    );
    assert!(!stdout.contains("part_modified"), "{stdout}");
    let command = add_command(&stdout);
    assert_eq!(command, "orbe add led-5mm resistor-axial --overwrite");
    let (output, out, err) = run_printed(project.path(), &command);
    assert_eq!(output.status.code(), Some(0), "{out}{err}");
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");

    let path = project.path().join("parts/led-5mm/part.jsonc");
    let text = fs::read_to_string(&path).unwrap();
    let edited = text.replace("5 mm LED", "5 mm LED edited");
    assert_ne!(text, edited);
    fs::write(&path, edited).unwrap();
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("error[part_modified]"), "{stdout}");
    assert!(
        stdout.contains("differs from `led-5mm@0.1.0` in `title`"),
        "{stdout}"
    );
    assert!(!stdout.contains("part_outdated"), "{stdout}");
}

#[test]
fn check_json_reports_part_outdated() {
    let project = common::project("nano-button");
    hold(project.path(), "led-5mm", "0.0.9");
    hold(project.path(), "tactile-switch-6mm", "0.0.7");
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "scene_invalid");
    let diagnostics = reply["data"]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2, "{stdout}");
    assert!(
        diagnostics.iter().all(|d| d["code"] == "part_outdated"),
        "{stdout}"
    );
    let hint = diagnostics[0]["hint"].as_str().unwrap();
    assert!(diagnostics.iter().all(|d| d["hint"] == hint), "{stdout}");
    assert!(
        hint.contains("orbe add led-5mm tactile-switch-6mm --overwrite"),
        "{hint}"
    );
}

#[test]
fn a_newer_major_is_not_pinned_into_an_overwrite() {
    let project = common::project("nano-button");
    hold(project.path(), "led-5mm", "1.0.0");
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("error[part_newer]"), "{stdout}");
    assert!(
        stdout.contains("this project holds `led-5mm@1.0.0` and this orbe carries `led-5mm@0.1.0`"),
        "{stdout}"
    );
    assert_eq!(
        stdout
            .lines()
            .find(|line| line.contains("hint:"))
            .unwrap()
            .trim(),
        "hint: this project needs a newer orbe"
    );
    assert!(!stdout.contains("orbe add"), "{stdout}");
    assert!(!stdout.contains("led-5mm@0.1.0 --overwrite"), "{stdout}");
    let text = fs::read_to_string(project.path().join("parts/led-5mm/part.jsonc")).unwrap();
    assert!(text.contains("\"version\": \"1.0.0\""), "{text}");
}

#[test]
fn a_newer_copy_is_not_an_overwrite() {
    let project = common::project("nano-button");
    hold(project.path(), "led-5mm", "0.2.0");
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    let diagnostics = reply["data"]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{stdout}");
    assert_eq!(diagnostics[0]["code"], "part_newer");
    assert_eq!(
        diagnostics[0]["message"],
        "this project holds `led-5mm@0.2.0` and this orbe carries `led-5mm@0.1.0`"
    );
    assert_eq!(diagnostics[0]["hint"], "this project needs a newer orbe");
    assert!(!stdout.contains("orbe add"), "{stdout}");
    let text = fs::read_to_string(project.path().join("parts/led-5mm/part.jsonc")).unwrap();
    assert!(text.contains("\"version\": \"0.2.0\""), "{text}");
}

#[test]
fn a_mix_of_older_and_newer_overwrites_only_the_older() {
    let project = common::project("nano-button");
    hold(project.path(), "led-5mm", "1.0.0");
    hold(project.path(), "resistor-axial", "0.0.8");
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("error[part_outdated]"), "{stdout}");
    assert!(stdout.contains("error[part_newer]"), "{stdout}");
    let command = add_command(&stdout);
    assert_eq!(command, "orbe add resistor-axial --overwrite");
    let (output, out, err) = run_printed(project.path(), &command);
    assert_eq!(output.status.code(), Some(0), "{out}{err}");
    assert!(!out.contains("led-5mm"), "the newer id was written: {out}");
    let led = fs::read_to_string(project.path().join("parts/led-5mm/part.jsonc")).unwrap();
    assert!(led.contains("\"version\": \"1.0.0\""), "{led}");
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("error[part_newer]"), "{stdout}");
    assert!(!stdout.contains("part_outdated"), "{stdout}");
    assert!(!stdout.contains("orbe add"), "{stdout}");
}

#[test]
fn an_overwrite_of_an_older_board_leaves_a_newer_child() {
    for (version, title) in [("0.2.0", "NEWER CHIP"), ("1.0.0", "MAJOR CHIP")] {
        let project = common::project("nano-button");
        let root = project.path();
        hold(root, "nano-board", "0.0.9");
        hold(root, "atmega328p-tqfp32", version);
        retitle(root, "atmega328p-tqfp32", title);
        let chip_before = fs::read(root.join("parts/atmega328p-tqfp32/part.jsonc")).unwrap();
        let lock_before = fs::read_to_string(root.join("parts.lock")).unwrap();
        let chip_line = lock_before
            .lines()
            .find(|line| line.starts_with("atmega328p-tqfp32 "))
            .unwrap()
            .to_string();

        let (output, stdout, _) = orbe_in(root, &["check", "scene.jsonc"]);
        assert_eq!(output.status.code(), Some(1), "{version}: {stdout}");
        assert!(stdout.contains("error[part_newer]"), "{version}: {stdout}");
        assert!(
            stdout.contains("error[part_outdated]"),
            "{version}: {stdout}"
        );
        let command = add_command(&stdout);
        assert_eq!(
            command, "orbe add nano-board --overwrite",
            "{version}: {stdout}"
        );
        assert!(!command.contains("atmega328p-tqfp32"), "{command}");

        let (output, out, err) = run_printed(root, &command);
        assert_eq!(output.status.code(), Some(0), "{version}: {out}{err}");
        assert!(
            out.contains("wrote parts/nano-board 0.1.0, replacing 0.0.9"),
            "{version}: {out}"
        );
        assert!(
            !out.contains("wrote parts/atmega328p-tqfp32")
                && !out.contains("skipped parts/atmega328p-tqfp32"),
            "{version}: {out}"
        );
        assert!(
            out.contains(&format!(
                "left `atmega328p-tqfp32` at {version}. this project needs a newer orbe for it"
            )),
            "{version}: {out}"
        );
        assert!(!out.contains("atmega328p-tqfp32@"), "{version}: {out}");
        assert_eq!(
            fs::read(root.join("parts/atmega328p-tqfp32/part.jsonc")).unwrap(),
            chip_before,
            "{version}"
        );
        let lock = fs::read_to_string(root.join("parts.lock")).unwrap();
        assert!(
            lock.lines().any(|line| line == chip_line),
            "{version}: {lock}"
        );

        let (output, stdout, _) = orbe_in(root, &["check", "scene.jsonc"]);
        assert_eq!(output.status.code(), Some(1), "{version}: {stdout}");
        assert!(stdout.contains("error[part_newer]"), "{version}: {stdout}");
        assert!(
            stdout.contains(&format!(
                "this project holds `atmega328p-tqfp32@{version}` and this orbe carries `atmega328p-tqfp32@0.1.0`"
            )),
            "{version}: {stdout}"
        );
        assert!(!stdout.contains("part_outdated"), "{version}: {stdout}");
        assert!(!stdout.contains("orbe add"), "{version}: {stdout}");
    }
}

#[test]
fn a_named_overwrite_leaves_a_newer_locked_copy() {
    let chip = "atmega328p-tqfp32";
    let named = ["add", chip, "--overwrite"];
    let pinned = ["add", "atmega328p-tqfp32@0.1.0", "--overwrite"];
    let board = ["add", "nano-board", "--overwrite"];
    for args in [named.as_slice(), pinned.as_slice(), board.as_slice()] {
        let project = common::project("nano-button");
        let root = project.path();
        hold(root, chip, "0.2.0");
        retitle(root, chip, "NEWER CHIP");
        let before = fs::read(root.join("parts").join(chip).join("part.jsonc")).unwrap();
        let (output, out, err) = orbe_in(root, args);
        assert_eq!(output.status.code(), Some(0), "{args:?}: {out}{err}");
        assert!(
            out.contains(
                "left `atmega328p-tqfp32` at 0.2.0. this project needs a newer orbe for it"
            ),
            "{args:?}: {out}"
        );
        assert!(
            !out.contains("wrote parts/atmega328p-tqfp32"),
            "{args:?}: {out}"
        );
        assert!(!out.contains("atmega328p-tqfp32@"), "{args:?}: {out}");
        assert_eq!(
            fs::read(root.join("parts").join(chip).join("part.jsonc")).unwrap(),
            before,
            "{args:?}"
        );
        let (output, stdout, _) = orbe_in(root, &["check", "scene.jsonc"]);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {stdout}");
        assert!(stdout.contains("error[part_newer]"), "{args:?}: {stdout}");
        assert!(!stdout.contains("part_outdated"), "{args:?}: {stdout}");
        assert!(!stdout.contains("orbe add"), "{args:?}: {stdout}");
    }

    let project = common::project("nano-button");
    let root = project.path();
    hold(root, chip, "0.2.0");
    retitle(root, chip, "NEWER CHIP");
    let before = fs::read(root.join("parts").join(chip).join("part.jsonc")).unwrap();
    let (output, stdout, stderr) = orbe_in(root, &["add", "nano-button-led", "--overwrite"]);
    assert_eq!(output.status.code(), Some(2), "{stdout}{stderr}");
    assert!(
        stderr.starts_with("error[usage]: `nano-button-led` is a Block"),
        "{stderr}"
    );
    assert_eq!(
        fs::read(root.join("parts").join(chip).join("part.jsonc")).unwrap(),
        before
    );
}

#[test]
fn an_outdated_add_does_not_call_a_lock_match_a_local_edit() {
    let project = common::project("nano-button");
    hold(project.path(), "led-5mm", "0.0.9");
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let command = add_command(&stdout);
    assert_eq!(command, "orbe add led-5mm --overwrite");
    let (output, out, err) = run_printed(project.path(), &command);
    assert_eq!(output.status.code(), Some(0), "{out}{err}");
    assert!(
        out.contains("wrote parts/led-5mm 0.1.0, replacing 0.0.9\n"),
        "{out}"
    );
    assert!(!out.contains("local edits"), "{out}");

    let edited = common::project("nano-button");
    let path = edited.path().join("parts/led-5mm/part.jsonc");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("5 mm LED", "5 mm LED edited")).unwrap();
    let (output, stdout, _) = orbe_in(edited.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let command = stdout
        .lines()
        .find_map(|line| {
            let start = line.find("`orbe add ")? + 1;
            let end = line[start..].find('`')? + start;
            Some(line[start..end].to_string())
        })
        .unwrap_or_else(|| panic!("{stdout}"));
    let (output, out, err) = run_printed(edited.path(), &command);
    assert_eq!(output.status.code(), Some(0), "{out}{err}");
    assert!(out.contains("replacing 0.1.0 with local edits"), "{out}");
}

#[test]
fn a_foreign_registry_at_another_version_names_that_registry() {
    let project = common::project("nano-button");
    hold(project.path(), "led-5mm", "0.0.9");
    let lock = project.path().join("parts.lock");
    let text = fs::read_to_string(&lock).unwrap().replace(
        &format!("led-5mm 0.0.9 {}", orbe_scene::REGISTRY),
        "led-5mm 0.0.9 @acme",
    );
    fs::write(&lock, text).unwrap();
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("from `@acme`"), "{stdout}");
    assert!(!stdout.contains("part_outdated"), "{stdout}");
    assert!(!stdout.contains("part_newer"), "{stdout}");
}

#[test]
fn a_scene_id_the_catalog_lacks_gets_a_suggestion_and_stays_out_of_the_command() {
    let (_workspace, proj) = first_target();
    let scene = proj.join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(&scene, text.replace("\"led-5mm\"", "\"led-5mn\"")).unwrap();
    let (output, stdout, _) = orbe_in(&proj, &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("part `led-5mn` is not in this project's parts.lock\n  hint: did you mean `led-5mm`? the catalog has no part named `led-5mn`\n"),
        "{stdout}"
    );
    assert_eq!(stdout.matches("did you mean").count(), 1, "{stdout}");
    assert!(
        stdout.contains("run `orbe add nano-board resistor-axial tactile-switch-6mm`"),
        "{stdout}"
    );
}

#[test]
fn add_refuses_a_folder_that_is_not_a_project() {
    let (_workspace, proj) = first_target();
    let build = proj.join("build");
    let (output, stdout, _) = orbe_in(&build, &["add", "led-5mm"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        "error[not_a_project]: the current folder is not an orbe project. it has no scene.jsonc and no parts.lock\n  hint: run `orbe add -C .. led-5mm`\n"
    );
    assert_eq!(fs::read_dir(&build).unwrap().count(), 1);
    let (output, stdout, _) = run_printed(&build, "orbe add -C .. led-5mm");
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(proj.join("parts/led-5mm/part.jsonc").is_file());
    assert!(!build.join("parts").exists());

    let lonely = tempfile::tempdir().unwrap();
    let (output, stdout, _) = orbe_in(lonely.path(), &["add", "--json", "led-5mm"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["error"]["code"], "not_a_project");
    assert!(
        reply["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("orbe add -C <folder>"),
        "{stdout}"
    );
    assert_eq!(fs::read_dir(lonely.path()).unwrap().count(), 0);
}

#[test]
fn a_failed_overwrite_puts_every_folder_back_and_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let project = common::project("nano-button");
    let root = project.path();
    for id in ["led-5mm", "resistor-axial"] {
        let path = root.join("parts").join(id).join("part.jsonc");
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"// my edit\n");
        fs::write(&path, bytes).unwrap();
    }
    let lock = fs::read(root.join("parts.lock")).unwrap();
    let blocked = root.join("parts/resistor-axial");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o555)).unwrap();
    let (output, stdout, stderr) =
        orbe_in(root, &["add", "led-5mm", "resistor-axial", "--overwrite"]);
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(output.status.code(), Some(3), "{stdout}{stderr}");
    assert!(stderr.starts_with("error[io_error]: cannot move ./parts/resistor-axial to ./parts/.orbe-add/old/resistor-axial: Permission denied"), "{stderr}");
    assert!(
        stderr.ends_with("rolled back, so the project is as it was: put back parts/led-5mm\n"),
        "{stderr}"
    );
    for id in ["led-5mm", "resistor-axial"] {
        let text = fs::read_to_string(root.join("parts").join(id).join("part.jsonc")).unwrap();
        assert!(text.ends_with("// my edit\n"), "{id}");
    }
    assert_eq!(fs::read(root.join("parts.lock")).unwrap(), lock);
    assert!(!root.join("parts/.orbe-add").exists());
}

/// A scene of another name: check names the folder with `-C .`, and that command runs.
#[test]
fn a_scene_not_named_scene_jsonc_gets_a_hint_that_runs() {
    let (_workspace, proj) = first_target();
    fs::rename(proj.join("scene.jsonc"), proj.join("blinky.jsonc")).unwrap();
    let (output, stdout, _) = orbe_in(&proj, &["check", "blinky.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let command = add_command(&stdout);
    assert_eq!(
        command,
        "orbe add -C . led-5mm nano-board resistor-axial tactile-switch-6mm"
    );
    let (output, stdout, stderr) = run_printed(&proj, &command);
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    let (output, stdout, _) = orbe_in(&proj, &["check", "blinky.jsonc"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_eq!(
        stdout,
        "blinky.jsonc: ok, 18 engine parts, 31 nets, ground nano__GND\n"
    );
}

/// A scene file that is a link: `-C` names the folder check reads parts from, the link's
/// target's, relative to where check ran.
#[test]
fn a_linked_scene_file_gets_the_folder_check_reads_from() {
    let (workspace, proj) = first_target();
    let view = workspace.path().join("view");
    fs::create_dir(&view).unwrap();
    std::os::unix::fs::symlink("../proj/scene.jsonc", view.join("scene.jsonc")).unwrap();
    for (cwd, scene, command) in [
        (
            workspace.path().to_path_buf(),
            "view/scene.jsonc",
            "orbe add -C proj led-5mm nano-board resistor-axial tactile-switch-6mm",
        ),
        (
            view.clone(),
            "scene.jsonc",
            "orbe add -C ../proj led-5mm nano-board resistor-axial tactile-switch-6mm",
        ),
    ] {
        let _ = fs::remove_dir_all(proj.join("parts"));
        let _ = fs::remove_file(proj.join("parts.lock"));
        let (output, stdout, _) = orbe_in(&cwd, &["check", scene]);
        assert_eq!(output.status.code(), Some(1), "{stdout}");
        assert_eq!(add_command(&stdout), command);
        let (output, stdout, stderr) = run_printed(&cwd, command);
        assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
        assert!(!view.join("parts").exists());
        let (output, stdout, _) = orbe_in(&cwd, &["check", scene]);
        assert_eq!(output.status.code(), Some(0), "{stdout}");
    }
}

/// A project folder whose name starts with `-` is printed as `./-x`, which the argument parser
/// reads as a folder, for both a missing part and a modified one.
#[test]
fn a_folder_starting_with_a_dash_gets_hints_that_run() {
    let (workspace, proj) = first_target();
    let dashed = workspace.path().join("-backup");
    fs::rename(&proj, &dashed).unwrap();
    let scene = "./-backup/scene.jsonc";
    let (output, stdout, _) = orbe_in(workspace.path(), &["check", "--", "-backup/scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let command = add_command(&stdout);
    assert_eq!(
        command,
        "orbe add -C ./-backup led-5mm nano-board resistor-axial tactile-switch-6mm"
    );
    let (output, stdout, stderr) = run_printed(workspace.path(), &command);
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    let led = dashed.join("parts/led-5mm/part.jsonc");
    let text = fs::read_to_string(&led).unwrap();
    fs::write(&led, text.replace("5 mm LED", "5 mm LED edited")).unwrap();
    let (output, stdout, _) = orbe_in(workspace.path(), &["check", scene]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        hint_commands(&stdout),
        ["orbe add -C ./-backup led-5mm --overwrite"]
    );
    let (output, stdout, stderr) = run_printed(
        workspace.path(),
        "orbe add -C ./-backup led-5mm --overwrite",
    );
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    let (output, stdout, _) = orbe_in(workspace.path(), &["check", scene]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
}

/// A file where `parts/` goes: check says so and prints no command that cannot run.
#[test]
fn a_file_named_parts_is_named_and_gets_no_command() {
    let project = common::project("nano-button");
    fs::remove_dir_all(project.path().join("parts")).unwrap();
    fs::write(project.path().join("parts"), "x\n").unwrap();
    let (output, stdout, _) = orbe_in(project.path(), &["check", "scene.jsonc"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("error[part_missing]: part `nano-board` cannot be read, because `parts` is a file, not a folder\n  hint: move `parts` out of the way, since part folders go in a folder named `parts`. then check again for the command that adds them\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("orbe add"), "{stdout}");
}

/// While one run holds the project's lock, another refuses at once with `busy`, `--diff`
/// included, and writes nothing.
#[test]
fn a_second_add_is_busy_while_one_holds_the_lock() {
    let project = scene_only();
    let lock = fs::File::open(project.path()).unwrap();
    lock.lock().unwrap();
    for args in [
        &["add", "led-5mm"][..],
        &["add", "led-5mm", "--diff"],
        &["add", "--relock"],
    ] {
        let (output, stdout, _) = orbe_in(project.path(), args);
        assert_eq!(output.status.code(), Some(1), "{stdout}");
        assert!(
            stdout.starts_with("error[busy]: another `orbe add` is at work in this project, and holds its lock\n  hint: wait for it to finish, then run `orbe "),
            "{stdout}"
        );
    }
    let (output, stdout, _) = orbe_in(project.path(), &["add", "--json", "led-5mm"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["error"]["code"], "busy");
    assert!(!project.path().join("parts").exists());
    assert!(!project.path().join("parts.lock").exists());
    lock.unlock().unwrap();
    drop(lock);
    let (output, stdout, _) = orbe_in(project.path(), &["add", "led-5mm"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(!project.path().join(".orbe-add.lock").exists());
}

/// `--diff` writes nothing, so it previews a project it cannot write.
#[test]
fn diff_previews_a_project_it_cannot_write() {
    use std::os::unix::fs::PermissionsExt;
    let project = scene_only();
    let (output, stdout, _) = orbe_in(project.path(), &["add", "led-5mm"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    let part = project.path().join("parts/led-5mm/part.jsonc");
    let text = fs::read_to_string(&part).unwrap();
    fs::write(&part, text.replace("5 mm LED", "5 mm LED edited")).unwrap();
    let folders = [
        project.path().to_path_buf(),
        project.path().join("parts"),
        project.path().join("parts/led-5mm"),
    ];
    let mode = |path: &Path, mode| fs::set_permissions(path, fs::Permissions::from_mode(mode));
    for folder in &folders {
        mode(folder, 0o555).unwrap();
    }
    let (output, stdout, stderr) = orbe_in(project.path(), &["add", "led-5mm", "--diff"]);
    let left: Vec<String> = fs::read_dir(project.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    for folder in &folders {
        mode(folder, 0o755).unwrap();
    }
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    assert!(
        stdout.contains("parts/led-5mm/part.jsonc\n-") && stdout.contains("5 mm LED edited"),
        "{stdout}"
    );
    let mut left = left;
    left.sort();
    assert_eq!(left, ["parts", "parts.lock", "scene.jsonc"]);
}

/// A folder at the name an earlier orbe used for its trash is the user's, so an add leaves it.
#[test]
fn a_user_folder_at_the_old_add_trash_is_left_in_place() {
    let project = scene_only();
    let note = project.path().join("parts/.orbe-add.trash/mine/note.txt");
    fs::create_dir_all(note.parent().unwrap()).unwrap();
    fs::write(&note, "KEEP-ME\n").unwrap();
    fs::write(
        project.path().join("parts/.orbe-add.trash/log"),
        "orbe add log 1\nplan existed -\n",
    )
    .unwrap();
    let (output, stdout, stderr) = orbe_in(project.path(), &["add", "led-5mm"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    assert_eq!(stdout, "wrote parts/led-5mm 0.1.0\nwrote parts.lock\n");
    assert_eq!(fs::read(&note).unwrap(), b"KEEP-ME\n");
}
