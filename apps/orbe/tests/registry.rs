//! `orbe registry build` on this repo's `parts/` and `blocks/`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn orbe(args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn build(out: &Path, json: bool) -> (Option<i32>, String, String) {
    let src = repo();
    let mut args = vec![
        "registry",
        "build",
        src.to_str().unwrap(),
        out.to_str().unwrap(),
    ];
    if json {
        args.push("--json");
    }
    orbe(&args)
}

#[test]
fn build_writes_a_registry_and_reports_its_files_and_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("registry");
    let (code, stdout, stderr) = build(&out, false);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.starts_with(&format!("wrote {}: ", out.display())),
        "{stdout}"
    );
    assert!(stdout.contains(" parts and 4 Blocks: "), "{stdout}");
    assert!(stderr.starts_with("warning: "), "{stderr}");
    assert!(stderr.contains("lack buy"), "{stderr}");
    assert!(out.join("index.json").is_file());

    let (code, stdout, stderr) = build(&out, true);
    assert_eq!(code, Some(1), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["error"]["code"], "registry_out_not_empty");

    let fresh = dir.path().join("again");
    let (code, stdout, _) = build(&fresh, true);
    assert_eq!(code, Some(0), "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], true);
    let report = &reply["data"]["report"];
    assert_eq!(report["blocks"], 4);
    assert!(report["stored"].as_u64() < report["listed"].as_u64());
    assert!(!reply["data"]["warnings"].as_array().unwrap().is_empty());
}

#[test]
fn a_folder_with_no_parts_and_no_blocks_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("registry.jsonc"),
        r#"{ "format": 1, "name": "@acme" }"#,
    )
    .unwrap();
    let out = dir.path().join("out");
    let (code, _, stderr) = orbe(&[
        "registry",
        "build",
        dir.path().to_str().unwrap(),
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.starts_with("error[registry_invalid]: "), "{stderr}");
    assert!(
        stderr.contains("holds no part in parts/ and no Block in blocks/"),
        "{stderr}"
    );
    assert!(stderr.contains("hint: "), "{stderr}");
    assert!(!out.exists());
}

#[test]
fn a_missing_manifest_or_source_folder_names_what_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    let out = dir.path().join("out");
    let (code, stdout, stderr) = orbe(&[
        "registry",
        "build",
        src.to_str().unwrap(),
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            "error[registry_invalid]: {} has no registry.jsonc, which names the registry: {{ \"format\": 1, \"name\": \"@<id>\" }}\n  hint: write registry.jsonc in that folder\n",
            src.display()
        )
    );
    assert!(!out.exists());

    let file = dir.path().join("a-file");
    std::fs::write(&file, "").unwrap();
    for not_a_folder in [dir.path().join("missing-src"), file] {
        let (code, stdout, stderr) = orbe(&[
            "registry",
            "build",
            not_a_folder.to_str().unwrap(),
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, Some(1), "{stdout}{stderr}");
        assert_eq!(stdout, "");
        assert_eq!(
            stderr,
            format!(
                "error[registry_invalid]: no folder at {}\n  hint: name the folder that holds registry.jsonc, parts/ and blocks/\n",
                not_a_folder.display()
            )
        );
        assert!(!out.exists());
    }
}

/// A part file, or a Block scene, that `orbe check` refuses is not published. The build exits
/// with the same finding and writes nothing.
#[test]
fn a_part_check_would_refuse_is_not_published() {
    let parts = [
        (
            "blob-chip",
            r#"{
  "format": 1,
  "part": "blob-chip",
  "version": "0.1.0",
  "title": "Blob",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "params": { "image": { "file": "bin", "as": "blob", "default": "image.bin", "sets": "/note" } },
  "model": { "kind": "resistor" }
}
"#,
            "blob",
        ),
        (
            "odd-key",
            r#"{
  "format": 1,
  "part": "odd-key",
  "version": "0.1.0",
  "title": "Odd",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "extra": 1,
  "model": { "kind": "resistor" }
}
"#,
            "extra",
        ),
        (
            "bad-pin",
            r#"{
  "format": 1,
  "part": "bad-pin",
  "version": "0.1.0",
  "title": "Pin",
  "license": "MIT",
  "pins": { "a": true },
  "model": { "kind": "resistor" }
}
"#,
            "found a boolean",
        ),
    ];
    for (id, part, snippet) in parts {
        refused_part(id, part, snippet);
    }
    refused_block();
}

fn refused_part(id: &str, part: &str, snippet: &str) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let folder = src.join("parts").join(id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        src.join("registry.jsonc"),
        r#"{ "format": 1, "name": "@acme" }"#,
    )
    .unwrap();
    std::fs::write(folder.join("part.jsonc"), part).unwrap();

    let project = dir.path().join("project");
    let copy = project.join("parts").join(id);
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::write(copy.join("part.jsonc"), part).unwrap();
    std::fs::write(
        project.join("scene.jsonc"),
        format!("{{ \"format\": 1, \"parts\": {{ \"chip\": {{ \"type\": \"{id}\" }} }} }}\n"),
    )
    .unwrap();
    let found = check_finding(&project.join("scene.jsonc"), snippet);
    let out = dir.path().join("out");
    assert_refused(&src, &out, &format!("parts/{id}/part.jsonc"), &found);
}

fn refused_block() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("parts/lamp")).unwrap();
    std::fs::create_dir_all(src.join("blocks/bad-block")).unwrap();
    std::fs::write(
        src.join("registry.jsonc"),
        r#"{ "format": 1, "name": "@acme" }"#,
    )
    .unwrap();
    std::fs::write(
        src.join("parts/lamp/part.jsonc"),
        r#"{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "model": { "kind": "resistor" }
}
"#,
    )
    .unwrap();
    let scene = r#"{
  "format": 1,
  "parts": { "l1": { "type": "lamp" } },
  "titel": "no"
}
"#;
    std::fs::write(src.join("blocks/bad-block/scene.jsonc"), scene).unwrap();
    std::fs::write(
        src.join("blocks/bad-block/block.jsonc"),
        r#"{ "format": 1, "block": "bad-block", "version": "0.1.0", "title": "A lamp", "board": "lamp", "license": "MIT" }"#,
    )
    .unwrap();

    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("scene.jsonc"), scene).unwrap();
    let found = check_finding(&project.join("scene.jsonc"), "titel");
    let out = dir.path().join("out");
    assert_refused(&src, &out, "blocks/bad-block/scene.jsonc", &found);
}

struct Found {
    code: String,
    line: u64,
    column: u64,
    message: String,
    hint: String,
}

fn check_finding(scene: &Path, snippet: &str) -> Found {
    let (code, stdout, stderr) = orbe(&["check", "--json", scene.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    let found = reply["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["message"].as_str().unwrap().contains(snippet))
        .unwrap_or_else(|| panic!("no finding containing {snippet} in {stdout}"));
    Found {
        code: found["code"].as_str().unwrap().to_string(),
        line: found["line"].as_u64().unwrap(),
        column: found["column"].as_u64().unwrap(),
        message: found["message"].as_str().unwrap().to_string(),
        hint: found["hint"].as_str().unwrap().to_string(),
    }
}

fn assert_refused(src: &Path, out: &Path, file: &str, found: &Found) {
    let path = |p: &Path| p.to_str().unwrap().to_string();
    let (code, stdout, stderr) = orbe(&["registry", "build", &path(src), &path(out)]);
    let got = format!("{stdout}{stderr}");
    assert_eq!(code, Some(1), "{got}");
    assert!(
        got.contains(&format!(
            "{file}:{}:{}: error[{}]: {}",
            found.line, found.column, found.code, found.message
        )),
        "{got}"
    );
    assert!(got.contains(&format!("hint: {}", found.hint)), "{got}");
    assert!(!out.exists(), "a refused build writes nothing: {got}");

    let (code, stdout, stderr) = orbe(&["registry", "build", "--json", &path(src), &path(out)]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "scene_invalid");
    let diagnostics = reply["data"]["diagnostics"].as_array().unwrap();
    let same = diagnostics.iter().find(|d| {
        d["code"] == found.code
            && d["message"] == found.message
            && d["hint"] == found.hint
            && d["line"] == found.line
            && d["column"] == found.column
            && d["file"] == file
    });
    assert!(same.is_some(), "{stdout}");
    assert!(!out.exists(), "a refused build writes nothing");
}

/// A lamp `orbe check` accepts. Each refusal below changes one thing on this part, or on a
/// Block that places it.
const LAMP: &str = r#"{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "summary": "A lamp",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "params": { "resistance": { "unit": "ohm", "default": "220", "min_exclusive": 0, "sets": "/resistance" } },
  "model": { "kind": "resistor", "ratings": { "power": null } }
}
"#;

fn manifest(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("registry.jsonc"),
        "{ \"format\": 1, \"name\": \"@acme\" }\n",
    )
    .unwrap();
}

fn put_part(root: &Path, id: &str, text: &str) -> PathBuf {
    let folder = root.join("parts").join(id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("part.jsonc"), text).unwrap();
    folder
}

fn put_block(root: &Path, id: &str, board: &str, scene: &str) {
    let folder = root.join("blocks").join(id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("scene.jsonc"), scene).unwrap();
    std::fs::write(
        folder.join("block.jsonc"),
        format!(
            "{{ \"format\": 1, \"block\": \"{id}\", \"version\": \"0.1.0\", \"title\": \"Block\", \"board\": \"{board}\", \"license\": \"MIT\" }}\n"
        ),
    )
    .unwrap();
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

/// A scene that places the part `id` as `r1` and grounds its pin `a`.
fn grounded(id: &str) -> String {
    format!(
        "{{ \"format\": 1, \"parts\": {{ \"r1\": {{ \"type\": \"{id}\" }} }}, \"ground\": \"r1.a\" }}\n"
    )
}

/// Copies these part folders into the project and writes `scene.jsonc`.
fn installed_project(project: &Path, scene: &str, parts: &[(&str, &Path)]) {
    std::fs::create_dir_all(project).unwrap();
    for (id, folder) in parts {
        copy_tree(folder, &project.join("parts").join(id));
    }
    std::fs::write(project.join("scene.jsonc"), scene).unwrap();
}

struct Checked {
    parts: u64,
    nets: u64,
    ground: String,
}

fn check_clean(project: &Path) -> Checked {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(["check", "scene.jsonc"])
        .current_dir(project)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    let json = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(["check", "--json", "scene.jsonc"])
        .current_dir(project)
        .output()
        .unwrap();
    let body = String::from_utf8(json.stdout).unwrap();
    let err = String::from_utf8(json.stderr).unwrap();
    assert_eq!(json.status.code(), Some(0), "{body}{err}");
    let reply: Value = serde_json::from_str(body.trim()).unwrap();
    assert_eq!(reply["ok"], true, "{body}");
    let data = &reply["data"];
    let parts = data["parts"].as_u64().unwrap();
    let nets = data["nets"].as_u64().unwrap();
    let ground = data["ground"].as_str().unwrap().to_string();
    assert_eq!(data["scene"], "scene.jsonc");
    let line = stdout.trim();
    assert!(
        line.starts_with("scene.jsonc: ok, ")
            && line.contains("scene part")
            && line.contains(&format!("expanding to {parts} engine part"))
            && line.ends_with(&format!(", {nets} nets, ground {ground}")),
        "{stdout}"
    );
    Checked {
        parts,
        nets,
        ground,
    }
}

fn assert_published(src: &Path, project: &Path, expect: Checked) {
    let got = check_clean(project);
    assert_eq!(got.parts, expect.parts);
    assert_eq!(got.nets, expect.nets);
    assert_eq!(got.ground, expect.ground);
    let out = src.join("out");
    let path = |p: &Path| p.to_str().unwrap().to_string();
    let (code, stdout, stderr) = orbe(&["registry", "build", &path(src), &path(&out)]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(
        stdout.starts_with(&format!("wrote {}: ", out.display())),
        "{stdout}"
    );
    assert!(out.join("index.json").is_file(), "{stdout}");
}

fn run_here(dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// `orbe lint` prints the same errors as `orbe check` when the scene does not check.
fn assert_lint_matches_check(project: &Path) {
    for json in [false, true] {
        let mut check_args = vec!["check", "scene.jsonc"];
        let mut lint_args = vec!["lint", "scene.jsonc"];
        if json {
            check_args.insert(1, "--json");
            lint_args.insert(1, "--json");
        }
        let check = run_here(project, &check_args);
        let lint = run_here(project, &lint_args);
        assert_eq!(
            (lint.0, lint.1, lint.2),
            (check.0, check.1, check.2),
            "lint differs from check"
        );
    }
}

fn check_at(project: &Path, snippet: &str) -> Found {
    let (code, stdout, stderr) = run_here(project, &["check", "--json", "scene.jsonc"]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    let found = reply["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["message"].as_str().unwrap_or("").contains(snippet))
        .unwrap_or_else(|| panic!("no finding containing {snippet} in {stdout}"));
    Found {
        code: found["code"].as_str().unwrap().to_string(),
        line: found["line"].as_u64().unwrap(),
        column: found["column"].as_u64().unwrap(),
        message: found["message"].as_str().unwrap().to_string(),
        hint: found["hint"].as_str().unwrap_or("").to_string(),
    }
}

fn refuse_installed(src: &Path, project: &Path, file: &str, snippet: &str) {
    let found = check_at(project, snippet);
    assert_refused(src, &src.join("out"), file, &found);
}

#[test]
fn a_model_that_omits_a_field_the_engine_needs_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let part = r#"{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "summary": "A lamp",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "model": { "kind": "resistor" }
}
"#;
    let folder = put_part(&src, "lamp", part);
    let project = dir.path().join("project");
    installed_project(&project, &grounded("lamp"), &[("lamp", &folder)]);
    refuse_installed(&src, &project, "parts/lamp/part.jsonc", "nothing writes it");
}

#[test]
fn a_model_field_of_the_wrong_type_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let part = r#"{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "summary": "A lamp",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "model": { "kind": "resistor", "resistance": "220" }
}
"#;
    let folder = put_part(&src, "lamp", part);
    let project = dir.path().join("project");
    installed_project(&project, &grounded("lamp"), &[("lamp", &folder)]);
    refuse_installed(&src, &project, "parts/lamp/part.jsonc", "invalid type");
}

#[test]
fn a_param_default_outside_its_range_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "lamp", &LAMP.replace("\"220\"", "\"-1\""));
    let project = dir.path().join("project");
    installed_project(&project, &grounded("lamp"), &[("lamp", &folder)]);
    refuse_installed(&src, &project, "parts/lamp/part.jsonc", "out of range");
}

#[test]
fn a_block_scene_param_outside_its_range_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "lamp", LAMP);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"lamp\", \"resistance\": \"-1\" } }, \"ground\": \"r1.a\" }\n";
    put_block(&src, "bad-range", "lamp", scene);
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("lamp", &folder)]);
    refuse_installed(
        &src,
        &project,
        "blocks/bad-range/scene.jsonc",
        "out of range",
    );
}

#[test]
fn a_file_param_that_names_nothing_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let part = r#"{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "summary": "A lamp",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "params": {
    "resistance": { "unit": "ohm", "default": "220", "min_exclusive": 0, "sets": "/resistance" },
    "note": { "file": "txt", "as": "text", "default": "missing.txt", "sets": "/note" }
  },
  "model": { "kind": "resistor", "ratings": { "power": null }, "note": "" }
}
"#;
    let folder = put_part(&src, "lamp", part);
    let project = dir.path().join("project");
    installed_project(&project, &grounded("lamp"), &[("lamp", &folder)]);
    refuse_installed(&src, &project, "parts/lamp/part.jsonc", "missing.txt");
}

#[test]
fn a_part_that_names_a_missing_child_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let part = r#"{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "license": "MIT",
  "pins": { "a": "r1.a", "b": "r1.b" },
  "parts": { "r1": { "type": "missing-child" } },
  "params": { "resistance": { "unit": "ohm", "default": "220", "min_exclusive": 0, "sets": "/resistance" } },
  "model": { "kind": "resistor", "ratings": { "power": null } }
}
"#;
    let folder = put_part(&src, "lamp", part);
    let project = dir.path().join("project");
    installed_project(&project, &grounded("lamp"), &[("lamp", &folder)]);
    refuse_installed(&src, &project, "parts/lamp/part.jsonc", "missing-child");
}

#[test]
fn a_part_wire_to_an_unknown_pin_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let lamp = put_part(&src, "lamp", LAMP);
    let wrap = r#"{
  "format": 1,
  "part": "wrap",
  "version": "0.1.0",
  "title": "Wrap",
  "license": "MIT",
  "pins": { "a": "r1.a", "b": "r1.b" },
  "parts": { "r1": { "type": "lamp" } },
  "wires": [["r1.nope", "r1.a"]]
}
"#;
    let folder = put_part(&src, "wrap", wrap);
    let project = dir.path().join("project");
    installed_project(
        &project,
        &grounded("wrap"),
        &[("lamp", &lamp), ("wrap", &folder)],
    );
    refuse_installed(&src, &project, "parts/wrap/part.jsonc", "nope");
}

#[test]
fn a_block_scene_wire_to_an_unknown_pin_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "lamp", LAMP);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"lamp\" } }, \"wires\": [[\"r1.nope\", \"r1.a\"]], \"ground\": \"r1.a\" }\n";
    put_block(&src, "bad-pin", "lamp", scene);
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("lamp", &folder)]);
    refuse_installed(&src, &project, "blocks/bad-pin/scene.jsonc", "nope");
}

#[test]
fn a_block_scene_with_no_ground_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "lamp", LAMP);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"lamp\" } } }\n";
    put_block(&src, "no-gnd", "lamp", scene);
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("lamp", &folder)]);
    refuse_installed(
        &src,
        &project,
        "blocks/no-gnd/scene.jsonc",
        "nothing in the scene names a ground",
    );
}

#[test]
fn a_block_scene_that_names_an_unknown_hole_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let lamp = put_part(&src, "lamp", LAMP);
    let board = src.join("parts/breadboard-400");
    copy_tree(&repo().join("parts/breadboard-400"), &board);
    let scene = "{ \"format\": 1, \"parts\": { \"bb\": { \"type\": \"breadboard-400\" }, \"r1\": { \"type\": \"lamp\" } }, \"layout\": { \"bb\": { \"at\": [0, 0, 0] }, \"r1\": { \"on\": { \"a\": \"bb.zz99\" } } }, \"ground\": \"r1.b\" }\n";
    put_block(&src, "bad-hole", "breadboard-400", scene);
    let project = dir.path().join("project");
    installed_project(
        &project,
        scene,
        &[("breadboard-400", &board), ("lamp", &lamp)],
    );
    refuse_installed(&src, &project, "blocks/bad-hole/scene.jsonc", "zz99");
}

#[test]
fn a_block_that_names_firmware_it_does_not_ship_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = src.join("parts/atmega328p-dip28");
    copy_tree(&repo().join("parts/atmega328p-dip28"), &folder);
    let scene = "{ \"format\": 1, \"parts\": { \"mcu\": { \"type\": \"atmega328p-dip28\", \"firmware\": \"firmware.hex\" } }, \"ground\": \"mcu.GND\" }\n";
    put_block(&src, "blink", "atmega328p-dip28", scene);
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("atmega328p-dip28", &folder)]);
    refuse_installed(&src, &project, "blocks/blink/scene.jsonc", "firmware.hex");
}

#[test]
fn a_block_that_types_a_part_file_it_does_not_ship_is_not_published() {
    for ty in ["./ground.jsonc", "././ground.jsonc", "../ground.jsonc"] {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        manifest(&src);
        let folder = put_part(&src, "lamp", LAMP);
        let scene = format!(
            "{{ \"format\": 1, \"parts\": {{ \"r1\": {{ \"type\": \"lamp\" }}, \"g\": {{ \"type\": \"{ty}\" }} }}, \"ground\": \"r1.a\" }}\n"
        );
        put_block(&src, "blink", "lamp", &scene);
        let project = dir.path().join("project");
        installed_project(&project, &scene, &[("lamp", &folder)]);
        refuse_installed(&src, &project, "blocks/blink/scene.jsonc", ty);
    }
}

/// A lamp whose `note` file param defaults to `note`.
fn noted_lamp(note: &str) -> String {
    format!(
        r#"{{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "summary": "A lamp",
  "license": "MIT",
  "pins": {{ "a": [1], "b": [2] }},
  "params": {{
    "resistance": {{ "unit": "ohm", "default": "220", "min_exclusive": 0, "sets": "/resistance" }},
    "note": {{ "file": "txt", "as": "text", "default": "{note}", "sets": "/note" }}
  }},
  "model": {{ "kind": "resistor", "ratings": {{ "power": null }}, "note": "" }}
}}
"#
    )
}

#[test]
fn a_part_default_in_its_folder_that_names_nothing_is_not_published() {
    for note in ["firmware.hex", "./firmware.hex"] {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        manifest(&src);
        let folder = put_part(&src, "lamp", &noted_lamp(note));
        let project = dir.path().join("project");
        installed_project(&project, &grounded("lamp"), &[("lamp", &folder)]);
        refuse_installed(&src, &project, "parts/lamp/part.jsonc", "firmware.hex");
    }
}

/// The build refuses `src` as `registry_invalid`, naming `place` and `path`, and writes nothing.
fn refuse_outside(src: &Path, place: &str, path: &str, folder: &str) {
    let out = src.join("out");
    let source = src.to_str().unwrap();
    let (code, stdout, stderr) = orbe(&["registry", "build", source, out.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.starts_with("error[registry_invalid]: "), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "{place}: `{}` names `{path}`, outside the folder an install copies; {folder}",
            if place.starts_with("parts/") {
                "note"
            } else {
                "mcu.firmware"
            }
        )),
        "{stderr}"
    );
    assert!(!out.exists(), "a refused build writes nothing");
    let (code, stdout, _) = orbe(&["registry", "build", "--json", source, out.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{stdout}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(reply["error"]["code"], "registry_invalid", "{stdout}");
    assert!(!out.exists(), "a refused build writes nothing");
}

/// A Block's firmware, by its absolute path.
fn button_hex() -> String {
    let path = repo().join("blocks/nano-button-led/build/button.ino.hex");
    path.canonicalize().unwrap().to_str().unwrap().to_string()
}

/// `to`, written relative to the folder `from` through enough `..` to reach the root.
fn walked_up(from: &Path, to: &str) -> String {
    let depth = from.canonicalize().unwrap().components().count() - 1;
    format!("{}{}", "../".repeat(depth), to.trim_start_matches('/'))
}

#[test]
fn a_part_default_that_leaves_its_folder_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "lamp", &noted_lamp("x"));
    let hex = button_hex();
    let paths = [
        "../../firmware.hex".to_string(),
        "./../../firmware.hex".to_string(),
        "../.././firmware.hex".to_string(),
        "../../firmware.hex/../firmware.hex".to_string(),
        "../../ground.jsonc".to_string(),
        "../other/firmware.hex".to_string(),
        hex.clone(),
        walked_up(&folder, &hex),
    ];
    for path in paths {
        std::fs::write(folder.join("part.jsonc"), noted_lamp(&path)).unwrap();
        refuse_outside(
            &src,
            "parts/lamp/part.jsonc:11:55",
            &path,
            "a part's files must ship in its folder",
        );
    }
}

#[test]
fn a_block_file_that_leaves_its_folder_is_not_published() {
    let hex = button_hex();
    let through = hex.replacen("/build/", "/build/../build/", 1);
    assert_ne!(through, hex);
    for path in [hex.clone(), through, "../firmware.hex".to_string()] {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        manifest(&src);
        let folder = src.join("parts/atmega328p-dip28");
        copy_tree(&repo().join("parts/atmega328p-dip28"), &folder);
        let scene = format!(
            "{{ \"format\": 1, \"parts\": {{ \"mcu\": {{ \"type\": \"atmega328p-dip28\", \"firmware\": \"{path}\" }} }}, \"ground\": \"mcu.GND\" }}\n"
        );
        put_block(&src, "blink", "atmega328p-dip28", &scene);
        refuse_outside(
            &src,
            "blocks/blink/scene.jsonc:1:76",
            &path,
            "a Block's files must ship in its folder",
        );
        if path == hex {
            // `orbe check` in the author's own tree finds the file, and an install would not.
            let project = dir.path().join("project");
            installed_project(&project, &scene, &[("atmega328p-dip28", &folder)]);
            check_clean(&project);
        }
    }
}

#[test]
fn a_file_that_ships_in_its_folder_is_published() {
    let hex = std::fs::read(button_hex()).unwrap();
    for path in ["sketch.hex", "./sketch.hex"] {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        manifest(&src);
        let folder = src.join("parts/blink-mcu");
        copy_tree(&repo().join("parts/atmega328p-dip28"), &folder);
        let text = std::fs::read_to_string(folder.join("part.jsonc")).unwrap();
        let text = text
            .replacen(
                "\"part\": \"atmega328p-dip28\"",
                "\"part\": \"blink-mcu\"",
                1,
            )
            .replacen("\"required\": true", &format!("\"default\": \"{path}\""), 1);
        std::fs::write(folder.join("part.jsonc"), text).unwrap();
        std::fs::write(folder.join("sketch.hex"), &hex).unwrap();
        let scene = "{ \"format\": 1, \"parts\": { \"mcu\": { \"type\": \"blink-mcu\" } }, \"ground\": \"mcu.GND\" }\n";
        let project = dir.path().join("project");
        installed_project(&project, scene, &[("blink-mcu", &folder)]);
        assert_published(
            &src,
            &project,
            Checked {
                parts: 1,
                nets: 27,
                ground: "GND".into(),
            },
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = src.join("parts/atmega328p-dip28");
    copy_tree(&repo().join("parts/atmega328p-dip28"), &folder);
    let scene = "{ \"format\": 1, \"parts\": { \"mcu\": { \"type\": \"atmega328p-dip28\", \"firmware\": \"sketch.hex\" } }, \"ground\": \"mcu.GND\" }\n";
    put_block(&src, "blink", "atmega328p-dip28", scene);
    std::fs::write(src.join("blocks/blink/sketch.hex"), &hex).unwrap();
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("atmega328p-dip28", &folder)]);
    std::fs::write(project.join("sketch.hex"), &hex).unwrap();
    assert_published(
        &src,
        &project,
        Checked {
            parts: 1,
            nets: 27,
            ground: "GND".into(),
        },
    );
}

/// A part with one required param, `name`, declared by `decl`, and this model.
fn required_part(id: &str, pins: &str, name: &str, decl: &str, model: &str) -> String {
    format!(
        r#"{{
  "format": 1,
  "part": "{id}",
  "version": "0.1.0",
  "title": "Part",
  "summary": "A part",
  "license": "MIT",
  "pins": {pins},
  "params": {{
    "{name}": {{ {decl} }}
  }},
  "model": {model}
}}
"#
    )
}

const SOURCE_MODEL: &str = r#"{ "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } }"#;
const SOURCE_PINS: &str = r#"{ "pos": [1], "neg": [2] }"#;
const SWITCH_PINS: &str = r#"{ "a": [1], "b": [2] }"#;

/// A switch whose bounce object holds a window and a count, so a param can replace either.
fn switch_model(window_ps: u64, max_bounces: u64) -> String {
    format!(
        r#"{{ "kind": "switch", "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }}, "off_resistance": {{ "value": 100000000.0, "unit": "ohm", "source": "user" }}, "initial": "open", "events": [], "bounce": {{ "window": {{ "value": {window_ps}, "unit": "ps", "source": "user" }}, "max_bounces": {{ "value": {max_bounces}, "unit": "1", "source": "user" }} }} }}"#
    )
}

/// A required param whose `min_exclusive` and `max_exclusive` hold no value is refused at its
/// key in the part file. `orbe check` refuses every value a scene sets it to.
#[test]
fn a_param_whose_bounds_hold_no_value_is_not_published() {
    let switch = switch_model(1_000_000_000, 1);
    let cases = [
        (
            SOURCE_PINS,
            SOURCE_MODEL,
            "voltage",
            r#""unit": "V", "required": true, "min_exclusive": 1, "max_exclusive": 1.0000000000000002, "sets": "/voltage""#,
            "no number is greater than 1 V and less than 1.0000000000000002 V",
            "1",
        ),
        (
            SOURCE_PINS,
            SOURCE_MODEL,
            "voltage",
            r#""unit": "V", "required": true, "min_exclusive": 5, "max_exclusive": 5, "sets": "/voltage""#,
            "no number is greater than 5 V and less than 5 V",
            "5",
        ),
        (
            SWITCH_PINS,
            switch.as_str(),
            "window",
            r#""unit": "s", "required": true, "min_exclusive": 0, "max_exclusive": 1e-13, "sets": "/bounce/window""#,
            "no whole number of picoseconds, zero or more, is greater than 0 s and less than 0.0000000000001 s",
            "0.05ps",
        ),
        (
            SWITCH_PINS,
            switch.as_str(),
            "window",
            r#""unit": "s", "required": true, "max_exclusive": 0, "sets": "/bounce/window""#,
            "no whole number of picoseconds, zero or more, is less than 0 s",
            "0",
        ),
    ];
    for (pins, model, name, decl, why, value) in cases {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        manifest(&src);
        let text = required_part("src", pins, name, decl, model);
        let folder = put_part(&src, "src", &text);
        let message = format!("param `{name}` takes no value: {why}");
        let out = src.join("out");
        let (code, stdout, stderr) = orbe(&[
            "registry",
            "build",
            src.to_str().unwrap(),
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, Some(1), "{stdout}{stderr}");
        assert!(
            stdout.contains(&format!(
                "parts/src/part.jsonc:10:5: error[scene.param]: {message}"
            )),
            "{stdout}"
        );
        assert!(!out.exists(), "a refused build writes nothing");

        let pin = pins.split('"').nth(1).unwrap();
        let scene = format!(
            "{{ \"format\": 1, \"parts\": {{ \"r1\": {{ \"type\": \"src\", \"{name}\": \"{value}\" }} }}, \"ground\": \"r1.{pin}\" }}\n"
        );
        let project = dir.path().join("project");
        installed_project(&project, &scene, &[("src", &folder)]);
        check_at(&project, "");
    }
}

/// The engine takes a whole count, and a bounce window of at least 4 ns with one bounce. Bounds
/// that hold such a value publish, and a scene that sets one checks.
#[test]
fn a_required_count_or_duration_the_engine_takes_is_published() {
    let cases = [
        (
            switch_model(1_000_000_000, 1),
            "max_bounces",
            r#""unit": "1", "required": true, "min_exclusive": 0, "max_exclusive": 5, "sets": "/bounce/max_bounces""#,
            "2",
        ),
        (
            switch_model(1_000_000_000, 1),
            "max_bounces",
            r#""unit": "1", "required": true, "max_exclusive": 5, "sets": "/bounce/max_bounces""#,
            "2",
        ),
        (
            switch_model(1_000_000_000, 1),
            "window",
            r#""unit": "s", "required": true, "min_exclusive": 0, "max_exclusive": 5e-9, "sets": "/bounce/window""#,
            "4.5ns",
        ),
    ];
    for (model, name, decl, value) in cases {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        manifest(&src);
        let folder = put_part(
            &src,
            "sw",
            &required_part("sw", SWITCH_PINS, name, decl, &model),
        );
        let scene = format!(
            "{{ \"format\": 1, \"parts\": {{ \"r1\": {{ \"type\": \"sw\", \"{name}\": \"{value}\" }} }}, \"ground\": \"r1.a\" }}\n"
        );
        let project = dir.path().join("project");
        installed_project(&project, &scene, &[("sw", &folder)]);
        assert_published(
            &src,
            &project,
            Checked {
                parts: 1,
                nets: 2,
                ground: "GND".into(),
            },
        );
    }
}

/// A board whose one child waits for a value the board passes it still has its other children
/// checked by the engine. Here the switch's window breaks the engine's 4 ns floor.
#[test]
fn a_board_child_that_has_its_values_is_checked_while_another_waits() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let source = put_part(
        &src,
        "src",
        &required_part(
            "src",
            SOURCE_PINS,
            "voltage",
            r#""unit": "V", "required": true, "sets": "/voltage""#,
            SOURCE_MODEL,
        ),
    );
    let switch = put_part(
        &src,
        "sw",
        &required_part(
            "sw",
            SWITCH_PINS,
            "window",
            r#""unit": "s", "default": "1ms", "sets": "/bounce/window""#,
            &switch_model(1_000_000_000, 1),
        ),
    );
    let board = r#"{
  "format": 1,
  "part": "board",
  "version": "0.1.0",
  "title": "Board",
  "license": "MIT",
  "pins": { "a": "s.pos", "b": "s.neg" },
  "params": { "voltage": {} },
  "parts": {
    "s": { "type": "src", "voltage": { "param": "voltage" } },
    "k": { "type": "sw", "window": "1ns" }
  },
  "wires": [["s.pos", "k.a"], ["s.neg", "k.b"]]
}
"#;
    let folder = put_part(&src, "board", board);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"board\", \"voltage\": \"5\" } }, \"ground\": \"r1.b\" }\n";
    let project = dir.path().join("project");
    installed_project(
        &project,
        scene,
        &[("board", &folder), ("src", &source), ("sw", &switch)],
    );
    refuse_installed(&src, &project, "parts/board/part.jsonc", "at least 4ns");
}

/// A part whose required param has no value is still checked for what does not need one.
#[test]
fn a_part_with_a_required_param_is_checked_without_its_value() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let part = noted_lamp("missing.txt").replacen(r#""default": "220""#, r#""required": true"#, 1);
    let folder = put_part(&src, "lamp", &part);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"lamp\", \"resistance\": \"220\" } }, \"ground\": \"r1.a\" }\n";
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("lamp", &folder)]);
    refuse_installed(&src, &project, "parts/lamp/part.jsonc", "missing.txt");
}

#[test]
fn a_required_voltage_below_zero_is_published() {
    let cases = [
        (r#""max_exclusive": 0"#, "-2"),
        (r#""max_exclusive": -5"#, "-6"),
        (r#""min_exclusive": -10, "max_exclusive": -1"#, "-5"),
        (r#""min_exclusive": 0, "max_exclusive": 5"#, "2"),
    ];
    for (bounds, voltage) in cases {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        manifest(&src);
        let part = format!(
            r#"{{
  "format": 1,
  "part": "src",
  "version": "0.1.0",
  "title": "Source",
  "summary": "A source",
  "license": "MIT",
  "pins": {{ "pos": [1], "neg": [2] }},
  "params": {{ "voltage": {{ "unit": "V", "required": true, {bounds}, "sets": "/voltage" }} }},
  "model": {{ "kind": "voltage_source", "pins": {{ "p": "pos", "n": "neg" }} }}
}}
"#
        );
        let folder = put_part(&src, "src", &part);
        let scene = format!(
            "{{ \"format\": 1, \"parts\": {{ \"r1\": {{ \"type\": \"src\", \"voltage\": \"{voltage}\" }} }}, \"ground\": \"r1.neg\" }}\n"
        );
        let project = dir.path().join("project");
        installed_project(&project, &scene, &[("src", &folder)]);
        assert_published(
            &src,
            &project,
            Checked {
                parts: 1,
                nets: 2,
                ground: "GND".into(),
            },
        );
    }
}

#[test]
fn disagreeing_child_grounds_are_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let grounded_lamp = |id: &str| {
        LAMP.replacen("\"part\": \"lamp\"", &format!("\"part\": \"{id}\""), 1)
            .replacen("\"model\":", "\"ground\": \"a\",\n  \"model\":", 1)
    };
    let left = put_part(&src, "left", &grounded_lamp("left"));
    let right = put_part(&src, "right", &grounded_lamp("right"));
    let twin = r#"{
  "format": 1,
  "part": "twin",
  "version": "0.1.0",
  "title": "Twin",
  "license": "MIT",
  "pins": { "a": "l.a", "b": "r.b" },
  "parts": { "l": { "type": "left" }, "r": { "type": "right" } }
}
"#;
    let folder = put_part(&src, "twin", twin);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"twin\" } } }\n";
    let project = dir.path().join("project");
    installed_project(
        &project,
        scene,
        &[("left", &left), ("right", &right), ("twin", &folder)],
    );
    refuse_installed(&src, &project, "scene.jsonc", "different nets");
}

/// A source and a resistor in one model. `voltage` writes only the source.
fn combo_part() -> &'static str {
    r#"{
  "format": 1,
  "part": "combo",
  "version": "0.1.0",
  "title": "Combo",
  "summary": "A source and a resistor",
  "license": "MIT",
  "pins": { "pos": [1], "neg": [2], "a": [3], "b": [4] },
  "params": { "voltage": { "unit": "V", "required": true, "sets": "/src/voltage" } },
  "model": {
    "src": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } },
    "r": { "kind": "resistor", "resistance": "220", "pins": { "a": "a", "b": "b" } }
  }
}
"#
}

/// The resistor beside a waiting voltage is the same error `orbe check` prints once a scene
/// sets that voltage. The source itself is not whole, so a missing voltage is not a finding.
#[test]
fn a_sibling_body_a_waiting_param_does_not_write_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "combo", combo_part());
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"combo\", \"voltage\": \"5\" } }, \"ground\": \"r1.neg\" }\n";
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("combo", &folder)]);
    refuse_installed(&src, &project, "parts/combo/part.jsonc", "string \"220\"");
}

/// The waiting param writes the only body, so the build has nothing else to refuse and the
/// part is published. A scene that sets the value checks.
#[test]
fn a_part_whose_waiting_param_writes_its_only_body_is_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(
        &src,
        "src",
        &required_part(
            "src",
            SOURCE_PINS,
            "voltage",
            r#""unit": "V", "required": true, "sets": "/voltage""#,
            SOURCE_MODEL,
        ),
    );
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"src\", \"voltage\": \"5\" } }, \"ground\": \"r1.neg\" }\n";
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("src", &folder)]);
    assert_published(
        &src,
        &project,
        Checked {
            parts: 1,
            nets: 2,
            ground: "GND".into(),
        },
    );
}

const FIT_CHOICE: &str = r#"{ "value": 5, "unit": "V", "source": "user" }"#;
const OHM_CHOICE: &str = r#"{ "value": 220, "unit": "ohm", "source": "user" }"#;

/// One choice writes the source and the first resistor. The third body is not one of those
/// pointers, so its string resistance is still refused.
#[test]
fn a_param_that_sets_several_bodies_still_checks_one_it_does_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let part = format!(
        r#"{{
  "format": 1,
  "part": "combo",
  "version": "0.1.0",
  "title": "Combo",
  "summary": "A source and two resistors",
  "license": "MIT",
  "pins": {{ "pos": [1], "neg": [2], "a": [3], "b": [4], "c": [5], "d": [6] }},
  "params": {{
    "fit": {{
      "required": true,
      "choices": {{
        "five": {{
          "sets": {{
            "/src/voltage": {FIT_CHOICE},
            "/r/resistance": {OHM_CHOICE}
          }}
        }}
      }}
    }}
  }},
  "model": {{
    "src": {{ "kind": "voltage_source", "pins": {{ "p": "pos", "n": "neg" }} }},
    "r": {{ "kind": "resistor", "resistance": "220", "pins": {{ "a": "a", "b": "b" }} }},
    "k": {{ "kind": "resistor", "resistance": "220", "pins": {{ "a": "c", "b": "d" }} }}
  }}
}}
"#
    );
    let folder = put_part(&src, "combo", &part);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"combo\", \"fit\": \"five\" } }, \"ground\": \"r1.neg\" }\n";
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("combo", &folder)]);
    refuse_installed(&src, &project, "parts/combo/part.jsonc", "string \"220\"");
}

/// The same choice writes both bodies, and the resistor's string is what the choice replaces.
/// The build does not pick the choice, so it does not treat that string as the value.
#[test]
fn a_body_a_waiting_choice_writes_is_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let part = format!(
        r#"{{
  "format": 1,
  "part": "combo",
  "version": "0.1.0",
  "title": "Combo",
  "summary": "A source and a resistor",
  "license": "MIT",
  "pins": {{ "pos": [1], "neg": [2], "a": [3], "b": [4] }},
  "params": {{
    "fit": {{
      "required": true,
      "choices": {{
        "five": {{
          "sets": {{
            "/src/voltage": {FIT_CHOICE},
            "/r/resistance": {OHM_CHOICE}
          }}
        }}
      }}
    }}
  }},
  "model": {{
    "src": {{ "kind": "voltage_source", "pins": {{ "p": "pos", "n": "neg" }} }},
    "r": {{ "kind": "resistor", "resistance": "220", "ratings": {{ "power": null }}, "pins": {{ "a": "a", "b": "b" }} }}
  }}
}}
"#
    );
    let folder = put_part(&src, "combo", &part);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"combo\", \"fit\": \"five\" } }, \"ground\": \"r1.neg\" }\n";
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("combo", &folder)]);
    assert_published(
        &src,
        &project,
        Checked {
            parts: 2,
            nets: 4,
            ground: "GND".into(),
        },
    );
}

/// `voltage` is passed to the child source. The wrapper's own resistor is not that pointer,
/// so the build refuses it on the same line `orbe check` does once the scene sets the value.
#[test]
fn a_body_stays_checked_when_the_waiting_param_is_passed_to_a_child() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let source = put_part(
        &src,
        "src",
        &required_part(
            "src",
            SOURCE_PINS,
            "voltage",
            r#""unit": "V", "required": true, "sets": "/voltage""#,
            SOURCE_MODEL,
        ),
    );
    let wrap = r#"{
  "format": 1,
  "part": "wrap",
  "version": "0.1.0",
  "title": "Wrap",
  "summary": "A source and a resistor",
  "license": "MIT",
  "pins": { "pos": "s.pos", "neg": "s.neg", "a": [1], "b": [2] },
  "params": { "voltage": {} },
  "parts": { "s": { "type": "src", "voltage": { "param": "voltage" } } },
  "model": { "kind": "resistor", "resistance": "220", "pins": { "a": "a", "b": "b" } }
}
"#;
    let folder = put_part(&src, "wrap", wrap);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"wrap\", \"voltage\": \"5\" } }, \"ground\": \"r1.neg\" }\n";
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("src", &source), ("wrap", &folder)]);
    refuse_installed(&src, &project, "parts/wrap/part.jsonc", "string \"220\"");
}

const BROKEN_COMBO: &str = r#"{
  "src": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } },
  "r": { "kind": "resistor", "resistance": "220", "pins": { "a": "a", "b": "b" } }
}"#;

const VALID_COMBO: &str = r#"{
  "src": {
    "kind": "voltage_source",
    "voltage": { "value": 5, "unit": "V", "source": "user" },
    "pins": { "p": "pos", "n": "neg" }
  },
  "r": {
    "kind": "resistor",
    "resistance": { "value": 220, "unit": "ohm", "source": "user" },
    "ratings": { "power": null },
    "pins": { "a": "a", "b": "b" }
  }
}"#;

fn combo_shell(params: &str, model: &str) -> String {
    format!(
        r#"{{
  "format": 1,
  "part": "combo",
  "version": "0.1.0",
  "title": "Combo",
  "summary": "A source and a resistor",
  "license": "MIT",
  "pins": {{ "pos": [1], "neg": [2], "a": [3], "b": [4] }},
  "params": {params},
  "model": {model}
}}
"#
    )
}

/// The empty-name finding, alone, at the same line and column in the build and in check.
/// `absent` must not appear in check's JSON. Lint prints check's text.
fn refuse_empty_sets(part: &str, scene: &str, pointer: &str, absent: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "combo", part);
    let project = dir.path().join("project");
    installed_project(&project, scene, &[("combo", &folder)]);
    let found = check_at(&project, "this name is empty");
    assert_eq!(found.code, "scene.param");
    assert!(
        found.message.contains(&format!(
            "`{pointer}` is not a pointer into `model`; a pointer names a model field, and this name is empty"
        )),
        "{}",
        found.message
    );
    assert!(
        found
            .hint
            .contains("point `sets` at a field of an object in `model`"),
        "{}",
        found.hint
    );
    let (code, stdout, stderr) = run_here(&project, &["check", "--json", "scene.jsonc"]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap();
    let diagnostics = reply["data"]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{stdout}");
    for snippet in absent {
        assert!(!stdout.contains(snippet), "{snippet} in {stdout}");
    }
    refuse_installed(
        &src,
        &project,
        "parts/combo/part.jsonc",
        "this name is empty",
    );
    assert_lint_matches_check(&project);
}

/// A required number `sets` `/`. The name is empty, so check and the build refuse it at the
/// param and the build writes nothing. `set` writes no field, so the value cannot replace the
/// model; the part's own finding is the one both sides print.
#[test]
fn a_required_param_that_sets_slash_is_refused() {
    let part = combo_shell(
        r#"{ "voltage": { "unit": "V", "required": true, "sets": "/" } }"#,
        BROKEN_COMBO,
    );
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"combo\", \"voltage\": \"5\" } }, \"ground\": \"r1.neg\" }\n";
    refuse_empty_sets(&part, scene, "/", &["string \"220\"", "nothing writes it"]);
}

/// A choice arm `sets` `/` to a whole model. The name is empty, so the arm is refused and
/// does not replace the broken model.
#[test]
fn a_choice_that_sets_slash_is_refused() {
    let params = r#"{
  "fit": {
    "required": true,
    "choices": {
      "good": {
        "sets": {
          "/": {
            "src": {
              "kind": "voltage_source",
              "voltage": { "value": 5, "unit": "V", "source": "user" },
              "pins": { "p": "pos", "n": "neg" }
            },
            "r": {
              "kind": "resistor",
              "resistance": { "value": 220, "unit": "ohm", "source": "user" },
              "ratings": { "power": null },
              "pins": { "a": "a", "b": "b" }
            }
          }
        }
      }
    }
  }
}"#;
    let part = combo_shell(params, BROKEN_COMBO);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"combo\", \"fit\": \"good\" } }, \"ground\": \"r1.neg\" }\n";
    refuse_empty_sets(&part, scene, "/", &["string \"220\"", "nothing writes it"]);
}

/// The model is already valid, and a choice `sets` `/` to a broken one. The broken value is
/// not what the build checks, and the empty name is still refused, so nothing is published.
#[test]
fn a_choice_that_sets_slash_on_a_valid_model_is_refused() {
    let params = r#"{ "fit": { "required": true, "choices": { "bad": { "sets": { "/": { "src": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } }, "r": { "kind": "resistor", "resistance": "220", "pins": { "a": "a", "b": "b" } } } } } } } }"#;
    let part = combo_shell(params, VALID_COMBO);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"combo\", \"fit\": \"bad\" } }, \"ground\": \"r1.neg\" }\n";
    refuse_empty_sets(&part, scene, "/", &["invalid type", "nothing writes it"]);
}

/// `//x` and `/a/` are empty names, refused the same way `/` is.
#[test]
fn a_sets_pointer_with_an_empty_name_is_refused() {
    for pointer in ["//x", "/a/"] {
        let params =
            format!(r#"{{ "note": {{ "unit": "1", "required": true, "sets": "{pointer}" }} }}"#);
        let part = combo_shell(&params, VALID_COMBO);
        let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"combo\", \"note\": \"1\" } }, \"ground\": \"r1.neg\" }\n";
        refuse_empty_sets(
            &part,
            scene,
            pointer,
            &["invalid type", "nothing writes it"],
        );
    }
}
