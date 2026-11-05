//! `orbe check` and `orbe add` on a project built from the reader's fixture.

#[path = "../../orbe-scene/tests/common/mod.rs"]
mod common;

use std::fs;
use std::path::Path;
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
    let scene = project.path().join("scene.jsonc");
    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let expected = format!(
        "{}:9:23: error[part_missing]: part `led-5mm` has a parts.lock line but no folder\n  hint: run `orbe add led-5mm`\n1 error in {}\n",
        scene.display(),
        scene.display()
    );
    assert_eq!(stdout, expected);
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
    let command = add_command(&stdout);
    assert_eq!(
        command,
        "orbe add led-5mm nano-board resistor-axial tactile-switch-6mm"
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
    let fixture_lock =
        fs::read_to_string(common::fixture("nano-button").join("parts.lock")).unwrap();
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

    let mangled = fs::read_to_string(&lock_path)
        .unwrap()
        .replacen("sha256:0", "sha256:1", 1);
    fs::write(&lock_path, &mangled).unwrap();
    let (output, stdout, _) = orbe_in(dir.path(), &["add", "--relock"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("wrote parts.lock"), "{stdout}");
    assert!(stdout.contains("`nano-board` is modified"), "{stdout}");
    let lock = fs::read_to_string(&lock_path).unwrap();
    assert!(!lock.contains("sha256:1d3d50"), "{lock}");
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
    let dir = tempfile::tempdir().unwrap();
    let (output, stdout, _) = orbe_in(dir.path(), &["add", "led-5m"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("did you mean"), "{stdout}");
    assert!(stdout.contains("`led-5mm`"), "{stdout}");
    assert!(!dir.path().join("parts").exists());
    assert!(!dir.path().join("parts.lock").exists());
}

#[test]
fn a_registry_id_says_v1_reads_the_official_catalog() {
    let dir = tempfile::tempdir().unwrap();
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
    let scene = project.path().join("scene.jsonc");
    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("part `led-5mm` is not in this project's parts.lock"),
        "{stdout}"
    );
    assert!(stdout.contains("run `orbe add led-5mm`"), "{stdout}");
}
