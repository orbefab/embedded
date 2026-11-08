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
    let out = dir.path().join("out");
    let (code, _, stderr) = orbe(&[
        "registry",
        "build",
        dir.path().to_str().unwrap(),
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.starts_with("error[registry_invalid]: "), "{stderr}");
    assert!(stderr.contains("hint: "), "{stderr}");
    assert!(!out.exists());
}
