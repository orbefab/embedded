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
    assert!(
        stdout.contains(" parts, 3 Blocks and 0 models: "),
        "{stdout}"
    );
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
    assert_eq!(report["blocks"], 3);
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
    assert!(stdout.contains("1 records carried forward"), "{stdout}");
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
