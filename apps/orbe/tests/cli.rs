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
/// A Scratch, since a test that locks this project's folders still leaves nothing behind.
fn scene_only() -> orbe_scratch::Scratch {
    let dir = orbe_scratch::Scratch::new();
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

/// Runs a printed command as a shell would split it. The paths here have no quotes or spaces.
fn run_printed(cwd: &Path, command: &str) -> (Output, String, String) {
    assert!(!command.contains(['\'', '"']), "{command}");
    let words: Vec<&str> = command.split_whitespace().collect();
    assert_eq!(&words[..2], ["orbe", "add"], "{command}");
    orbe_in(cwd, &words[1..])
}
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
        stdout.contains("ok, 5 scene parts, expanding to 19 engine parts, 34 nets, ground GND"),
        "{stdout}"
    );

    let (output, stdout) = orbe(&["check", "--json", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    let reply: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["data"]["parts"], 19);
    assert_eq!(reply["data"]["nets"], 34);
    assert_eq!(reply["data"]["ground"], "GND");
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
fn check_and_lint_read_scene_jsonc_when_no_scene_is_named() {
    let (_workspace, proj) = first_target();
    for command in ["check", "lint"] {
        let (named, named_out, named_err) = orbe_in(&proj, &[command, "scene.jsonc"]);
        let (bare, bare_out, bare_err) = orbe_in(&proj, &[command]);
        assert_eq!(
            (named.status.code(), named_out, named_err),
            (bare.status.code(), bare_out, bare_err),
            "{command}"
        );
        assert_ne!(
            bare.status.code(),
            Some(2),
            "{command} treated the default as usage"
        );
    }
    let empty = tempfile::tempdir().unwrap();
    let (output, _, stderr) = orbe_in(empty.path(), &["lint"]);
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("no scene file at scene.jsonc"), "{stderr}");
    let (output, stdout) = orbe(&["check", "--help"]);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("scene.jsonc when omitted"), "{stdout}");
    let (output, stdout) = orbe(&["lint", "--help"]);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("scene.jsonc when omitted"), "{stdout}");
}

#[test]
fn json_usage_is_one_envelope_on_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let (output, stdout, stderr) = orbe_in(dir.path(), &["check", "--json"]);
    assert_eq!(output.status.code(), Some(3), "{stdout}{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "io_error");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("scene.jsonc"),
        "{reply}"
    );
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
        "scene.jsonc:9:23: error[part_missing]: part `led-5mm` is not in parts/\n  hint: run `orbe add led-5mm`\n1 error in scene.jsonc\n"
    );
}

#[test]
fn help_is_not_a_usage_envelope() {
    let (output, stdout) = orbe(&["check", "--help"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("check"), "{stdout}");
}

#[test]
fn add_help_names_a_version_placeholder() {
    let (output, stdout) = orbe(&["add", "--help"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("id@<version>"), "{stdout}");
    assert!(!stdout.contains("@0.1.0"), "{stdout}");
}

#[test]
fn a_version_that_is_not_three_numbers_shows_the_shape() {
    let dir = tempfile::tempdir().unwrap();
    let (output, stdout, stderr) = orbe_in(dir.path(), &["add", "led-5mm@1"]);
    assert_eq!(output.status.code(), Some(2), "{stdout}{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        stderr,
        "error[usage]: `led-5mm@1` is not a version. a version is three numbers, such as `id@1.2.3`\n"
    );
}

#[test]
fn open_prints_a_missing_part_once() {
    let project = tempfile::tempdir().unwrap();
    let (output, stdout, stderr) = orbe_in(project.path(), &["add", "nano-button-led"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    fs::remove_dir_all(project.path().join("parts/nano-board")).unwrap();
    let (output, stdout, stderr) = orbe_in(project.path(), &["open"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(
        stdout,
        "scene.jsonc:8:23: error[part_missing]: part `nano-board` is not in parts/\n  hint: run `orbe add nano-board`\n"
    );
    assert_eq!(stdout.matches("error[part_missing]").count(), 1, "{stdout}");

    let (output, stdout, stderr) =
        orbe_in(project.path(), &["open", "--json", "--session", "once"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "part_missing");
    assert_eq!(
        reply["error"]["message"],
        "part `nano-board` is not in parts/"
    );
    assert_eq!(reply["error"]["hint"], "run `orbe add nano-board`");
    let found = reply["data"]["diagnostics"].as_array().unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["code"], "part_missing");
    assert_eq!(found[0]["line"], 8);
    assert_eq!(found[0]["column"], 23);
    assert_eq!(found[0]["message"], reply["error"]["message"]);
    assert_eq!(found[0]["hint"], reply["error"]["hint"]);
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
        let engine = parts
            .strip_suffix(" engine parts")
            .unwrap_or_else(|| panic!("{name}: the first line is `<n> engine parts`"));
        let project = common::overlaid(&case);
        let scene = project.path().join("scene.jsonc");
        let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(0), "{name}: {stdout}");
        assert!(
            stdout.contains("scene part")
                && stdout.contains(&format!("expanding to {engine} engine part")),
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
    assert!(!stdout.contains("parts.lock"), "{stdout}");
    assert!(!dir.path().join("parts.lock").exists());
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
    for id in &copied {
        assert!(
            stdout.contains(&format!(
                "wrote parts/{id} {}",
                orbe_scene::card(id).unwrap().version
            )),
            "{id}: {stdout}"
        );
    }

    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("ok, 5 scene parts, expanding to 19 engine parts, 34 nets, ground GND"),
        "{stdout}"
    );

    let led = dir.path().join("parts/led-5mm/part.jsonc");
    let led_bytes = fs::read(&led).unwrap();
    let (output, stdout, _) = orbe_in(dir.path(), &args);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("error[part_exists]"), "{stdout}");
    assert!(stdout.contains("is already led-5mm "), "{stdout}");
    assert!(
        !stdout.lines().any(|line| line.starts_with("wrote ")),
        "{stdout}"
    );
    assert_eq!(fs::read(&led).unwrap(), led_bytes);

    let mut json_args = vec!["add", "--json"];
    json_args.extend(args.iter().copied().skip(1));
    let (output, stdout, stderr) = orbe_in(dir.path(), &json_args);
    assert_eq!(output.status.code(), Some(1), "{stdout} {stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "part_exists");
    assert_eq!(fs::read(&led).unwrap(), led_bytes);

    let text = fs::read_to_string(&led).unwrap();
    fs::write(&led, text.replace("\"value\": 2.115", "\"value\": 1000")).unwrap();
    let (output, stdout) = orbe(&["check", scene.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(!stdout.contains("part_modified"), "{stdout}");
    assert!(!stdout.contains("note:"), "{stdout}");
    assert!(
        stdout.contains("ok, 5 scene parts, expanding to 19 engine parts, 34 nets, ground GND"),
        "{stdout}"
    );
    assert!(
        fs::read_to_string(&led)
            .unwrap()
            .contains("\"value\": 1000")
    );
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
        "proj/scene.jsonc: ok, 5 scene parts, expanding to 19 engine parts, 34 nets, ground GND\n"
    );
    assert!(proj.join("parts/led-5mm/part.jsonc").is_file());
    assert!(!proj.join("parts.lock").exists());
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
        stdout.contains("part `led-5mn` is not in parts/\n  hint: did you mean `led-5mm`? the catalog has no part named `led-5mn`\n"),
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
        "error[not_a_project]: the current folder is not an orbe project. it has no scene.jsonc and no parts folder\n  hint: run `orbe add -C .. led-5mm`\n"
    );
    assert_eq!(fs::read_dir(&build).unwrap().count(), 1);
    let (output, stdout, _) = run_printed(&build, "orbe add -C .. led-5mm");
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(proj.join("parts/led-5mm/part.jsonc").is_file());
    assert!(!build.join("parts").exists());

    let empty = tempfile::tempdir().unwrap();
    let (output, stdout, _) = orbe_in(empty.path(), &["add", "no-such-part"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        "error[unknown_part]: no Block or part named `no-such-part`\n  hint: the catalog has no Block or part with that id\n"
    );
    assert_eq!(fs::read_dir(empty.path()).unwrap().count(), 0);
    let build_typo = build.join("deeper");
    fs::create_dir(&build_typo).unwrap();
    let (output, stdout, _) = orbe_in(&build_typo, &["add", "nano-button-lde"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        "error[unknown_part]: no Block or part named `nano-button-lde`\n  hint: did you mean `nano-button-led`?\n"
    );
    let (output, stdout, _) = orbe_in(&build_typo, &["add", "nano-buton-led"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        "error[unknown_part]: no Block or part named `nano-buton-led`\n  hint: did you mean `nano-button-led`?\n"
    );
    assert_eq!(fs::read_dir(&build_typo).unwrap().count(), 0);

    let lonely = tempfile::tempdir().unwrap();
    let (output, stdout, _) = orbe_in(lonely.path(), &["add", "--json", "led-5m"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["error"]["code"], "unknown_part");
    assert_eq!(reply["error"]["message"], "no Block or part named `led-5m`");
    assert_eq!(reply["error"]["hint"], "did you mean `led-5mm`?");
    assert_eq!(fs::read_dir(lonely.path()).unwrap().count(), 0);

    let (output, stdout, stderr) = orbe_in(lonely.path(), &["add", "led-5mm"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    assert!(stdout.contains("wrote parts/led-5mm 0.1.1\n"), "{stdout}");
    assert!(!stdout.contains("parts.lock"), "{stdout}");
    assert!(!lonely.path().join("parts.lock").exists());
    assert!(lonely.path().join("parts/led-5mm/part.jsonc").is_file());
    assert!(!lonely.path().join("scene.jsonc").exists());

    let demo = tempfile::tempdir().unwrap();
    let (output, stdout, stderr) = orbe_in(demo.path(), &["add", "nano-button-led"]);
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    assert!(stdout.contains("wrote scene.jsonc\n"), "{stdout}");
    assert!(stdout.contains("wrote tests/button.test\n"), "{stdout}");
    assert!(!stdout.contains("parts.lock"), "{stdout}");
    assert!(!demo.path().join("parts.lock").exists());
    assert!(
        stdout.ends_with(
            "installed the Block `nano-button-led`. run `orbe check scene.jsonc` and `orbe test`\n"
        ),
        "{stdout}"
    );
    assert!(demo.path().join("scene.jsonc").is_file());
    assert!(demo.path().join("parts/led-5mm/part.jsonc").is_file());
    assert!(demo.path().join("firmware/button.hex").is_file());

    let relock = tempfile::tempdir().unwrap();
    let (output, stdout, stderr) = orbe_in(relock.path(), &["add", "--relock"]);
    assert_eq!(output.status.code(), Some(2), "{stdout}{stderr}");
    assert!(stderr.contains("--relock"), "{stderr}");
    assert_eq!(fs::read_dir(relock.path()).unwrap().count(), 0);
}
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
        "blinky.jsonc: ok, 5 scene parts, expanding to 19 engine parts, 34 nets, ground GND\n"
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
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(!stdout.contains("part_modified"), "{stdout}");
    assert!(!stdout.contains("note:"), "{stdout}");
    assert!(
        fs::read_to_string(&led)
            .unwrap()
            .contains("5 mm LED edited")
    );
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

/// While one run holds the project's lock, another refuses at once with `busy` and writes
/// nothing.
#[test]
fn a_second_add_is_busy_while_one_holds_the_lock() {
    let project = scene_only();
    let lock = fs::File::open(project.path()).unwrap();
    lock.lock().unwrap();
    let (output, stdout, _) = orbe_in(project.path(), &["add", "led-5mm"]);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.starts_with("error[busy]: another `orbe add` is at work in this project, and holds its lock\n  hint: wait for it to finish, then run `orbe "),
        "{stdout}"
    );
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
    assert_eq!(stdout, "wrote parts/led-5mm 0.1.1\n");
    assert_eq!(fs::read(&note).unwrap(), b"KEEP-ME\n");
}
