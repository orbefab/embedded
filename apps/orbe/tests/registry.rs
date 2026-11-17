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
                r#"{{ "format": 1, "part": "lamp", "version": "{version}", "title": "{title}", "license": "MIT", "pins": {{ "a": [1], "b": [2] }}, "model": {{ "kind": "resistor" }} }}"#
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
