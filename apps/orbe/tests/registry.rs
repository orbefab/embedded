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
fn previous_carries_a_build_forward_and_refuses_a_changed_version() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let part = src.join("parts/lamp");
    std::fs::create_dir_all(&part).unwrap();
    std::fs::write(
        src.join("registry.jsonc"),
        r#"{ "format": 1, "name": "@acme" }"#,
    )
    .unwrap();
    let lamp = |version: &str, title: &str| {
        std::fs::write(
            part.join("part.jsonc"),
            format!(
                r#"{{ "format": 1, "part": "lamp", "version": "{version}", "title": "{title}", "license": "MIT", "pins": {{ "a": [1], "b": [2] }}, "params": {{ "resistance": {{ "unit": "ohm", "default": "220", "min_exclusive": 0, "sets": "/resistance" }} }}, "model": {{ "kind": "resistor", "ratings": {{ "power": null }} }} }}"#
            ),
        )
        .unwrap();
    };
    let path = |p: &Path| p.to_str().unwrap().to_string();
    let first = dir.path().join("first");
    lamp("1.0.0", "Lamp");
    let (code, _, stderr) = orbe(&["registry", "build", &path(&src), &path(&first)]);
    assert_eq!(code, Some(0), "{stderr}");

    lamp("1.1.0", "Lamp");
    let second = dir.path().join("second");
    let (code, stdout, stderr) = orbe(&[
        "registry",
        "build",
        &path(&src),
        &path(&second),
        "--previous",
        &path(&first),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("and 1 records are carried forward"),
        "{stdout}"
    );
    assert!(second.join("lamp@1.0.0.json").is_file());
    assert!(second.join("lamp@1.1.0.json").is_file());

    lamp("1.1.0", "A brighter lamp");
    let third = dir.path().join("third");
    let (code, _, stderr) = orbe(&[
        "registry",
        "build",
        &path(&src),
        &path(&third),
        "--previous",
        &path(&second),
    ]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("`lamp@1.1.0` builds to a record other than the one served"),
        "{stderr}"
    );
    assert!(!third.exists(), "a refused build writes nothing");
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
        format!(
            "{{ \"format\": 1, \"parts\": {{ \"chip\": {{ \"type\": \"./parts/{id}/part.jsonc\" }} }} }}\n"
        ),
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

const PROBE_GROUND: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "model": {
    "kind": "voltage_source",
    "voltage": { "value": 0, "unit": "V", "source": "user" },
    "pins": { "p": "pos", "n": "neg" }
  }
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

fn lock_line(folder: &Path, id: &str, version: &str, registry: &str) -> String {
    format!(
        "{id} {version} {registry} {}",
        orbe_scene::folder_hash(folder).unwrap()
    )
}

fn grounded(id: &str) -> String {
    format!(
        "{{ \"format\": 1, \"parts\": {{ \"r1\": {{ \"type\": \"{id}\" }}, \"g\": {{ \"type\": \"./probe-ground.jsonc\" }} }}, \"ground\": \"g.neg\" }}\n"
    )
}

/// The project an install of these folders would check: the part folders, a lock line per
/// folder, and `scene.jsonc` read from the project directory. No probe file is written. A
/// scene that names `./probe-ground.jsonc` uses [`project_of`].
fn project_locked(project: &Path, scene: &str, parts: &[(&str, &Path, &str, &str)]) {
    std::fs::create_dir_all(project).unwrap();
    let mut lines = Vec::new();
    for (id, folder, version, registry) in parts {
        copy_tree(folder, &project.join("parts").join(id));
        lines.push(lock_line(folder, id, version, registry));
    }
    lines.sort();
    std::fs::write(project.join("scene.jsonc"), scene).unwrap();
    std::fs::write(
        project.join("parts.lock"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();
}

fn project_of(project: &Path, scene: &str, parts: &[(&str, &Path, &str, &str)]) {
    project_locked(project, scene, parts);
    std::fs::write(project.join("probe-ground.jsonc"), PROBE_GROUND).unwrap();
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
    assert_eq!(
        stdout.trim(),
        format!("scene.jsonc: ok, {parts} engine parts, {nets} nets, ground {ground}")
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

fn check_at(project: &Path, snippet: &str) -> Found {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(["check", "--json", "scene.jsonc"])
        .current_dir(project)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stdout}{stderr}");
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
    project_of(
        &project,
        &grounded("lamp"),
        &[("lamp", &folder, "1.2.0", "@acme")],
    );
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
    project_of(
        &project,
        &grounded("lamp"),
        &[("lamp", &folder, "1.2.0", "@acme")],
    );
    refuse_installed(&src, &project, "parts/lamp/part.jsonc", "invalid type");
}

#[test]
fn a_param_default_outside_its_range_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "lamp", &LAMP.replace("\"220\"", "\"-1\""));
    let project = dir.path().join("project");
    project_of(
        &project,
        &grounded("lamp"),
        &[("lamp", &folder, "1.2.0", "@acme")],
    );
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
    project_of(&project, scene, &[("lamp", &folder, "1.2.0", "@acme")]);
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
    project_of(
        &project,
        &grounded("lamp"),
        &[("lamp", &folder, "1.2.0", "@acme")],
    );
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
    project_of(
        &project,
        &grounded("lamp"),
        &[("lamp", &folder, "1.2.0", "@acme")],
    );
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
    project_of(
        &project,
        &grounded("wrap"),
        &[
            ("lamp", &lamp, "1.2.0", "@acme"),
            ("wrap", &folder, "0.1.0", "@acme"),
        ],
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
    project_of(&project, scene, &[("lamp", &folder, "1.2.0", "@acme")]);
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
    project_of(&project, scene, &[("lamp", &folder, "1.2.0", "@acme")]);
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
    project_of(
        &project,
        scene,
        &[
            ("breadboard-400", &board, "0.1.0", "@orbefab"),
            ("lamp", &lamp, "1.2.0", "@acme"),
        ],
    );
    refuse_installed(&src, &project, "blocks/bad-hole/scene.jsonc", "zz99");
}

#[test]
fn a_changed_sealed_catalog_part_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = src.join("parts/resistor-axial");
    copy_tree(&repo().join("parts/resistor-axial"), &folder);
    let path = folder.join("part.jsonc");
    let text = std::fs::read_to_string(&path).unwrap();
    let edited = text.replacen("Resistor, 1/4 W axial", "Resistor, edited", 1);
    assert_ne!(text, edited);
    std::fs::write(&path, edited).unwrap();
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"resistor-axial\", \"resistance\": \"220\" } }, \"ground\": \"r1.b\" }\n";
    let project = dir.path().join("project");
    project_of(
        &project,
        scene,
        &[("resistor-axial", &folder, "0.1.0", "@orbefab")],
    );
    refuse_installed(
        &src,
        &project,
        "parts/resistor-axial/part.jsonc",
        "in `title`",
    );
}

#[test]
fn a_block_that_names_the_probe_firmware_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = src.join("parts/atmega328p-dip28");
    copy_tree(&repo().join("parts/atmega328p-dip28"), &folder);
    let scene = "{ \"format\": 1, \"parts\": { \"mcu\": { \"type\": \"atmega328p-dip28\", \"firmware\": \"firmware.hex\" } }, \"ground\": \"mcu.GND\" }\n";
    put_block(&src, "blink", "atmega328p-dip28", scene);
    let project = dir.path().join("project");
    project_locked(
        &project,
        scene,
        &[("atmega328p-dip28", &folder, "0.1.0", "@orbefab")],
    );
    refuse_installed(&src, &project, "blocks/blink/scene.jsonc", "firmware.hex");
}

#[test]
fn a_block_that_names_the_probe_ground_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    manifest(&src);
    let folder = put_part(&src, "lamp", LAMP);
    let scene = "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"lamp\" }, \"g\": { \"type\": \"./probe-ground.jsonc\" } }, \"ground\": \"g.neg\" }\n";
    put_block(&src, "blink", "lamp", scene);
    let project = dir.path().join("project");
    project_locked(&project, scene, &[("lamp", &folder, "1.2.0", "@acme")]);
    refuse_installed(
        &src,
        &project,
        "blocks/blink/scene.jsonc",
        "probe-ground.jsonc",
    );
}

#[test]
fn a_part_path_that_walks_up_to_a_probe_file_is_not_published() {
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
    "note": { "file": "txt", "as": "text", "default": "../../firmware.hex", "sets": "/note" }
  },
  "model": { "kind": "resistor", "ratings": { "power": null }, "note": "" }
}
"#;
    let folder = put_part(&src, "lamp", part);
    let scene =
        "{ \"format\": 1, \"parts\": { \"r1\": { \"type\": \"lamp\" } }, \"ground\": \"r1.a\" }\n";
    let project = dir.path().join("project");
    project_locked(&project, scene, &[("lamp", &folder, "1.2.0", "@acme")]);
    refuse_installed(&src, &project, "parts/lamp/part.jsonc", "firmware.hex");
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
        project_locked(&project, &scene, &[("src", &folder, "0.1.0", "@acme")]);
        assert_published(
            &src,
            &project,
            Checked {
                parts: 1,
                nets: 2,
                ground: "r1__neg".into(),
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
    project_locked(
        &project,
        scene,
        &[
            ("left", &left, "1.2.0", "@acme"),
            ("right", &right, "1.2.0", "@acme"),
            ("twin", &folder, "0.1.0", "@acme"),
        ],
    );
    refuse_installed(&src, &project, "scene.jsonc", "different nets");
}
