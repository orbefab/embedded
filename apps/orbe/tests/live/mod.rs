//! Running `orbe` on a temporary first-target project, shared by the `orbe test` tests.

#![allow(dead_code)]

#[path = "../../../../packages/scene/tests/common/mod.rs"]
pub mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}: {}", self.stdout))
    }
}

/// Runs `orbe` in `dir`.
pub fn orbe(dir: &Path, args: &[&str]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orbe"));
    crate::registry_env::apply(&mut command);
    let output = command.args(args).current_dir(dir).output().unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// The first-target project with one test file, `tests/a.test`.
pub fn project(test: &str) -> TempDir {
    let dir = common::project("nano-button");
    fs::create_dir_all(dir.path().join("tests")).unwrap();
    fs::write(dir.path().join("tests/a.test"), test).unwrap();
    dir
}

/// `project` with the Nano running `tests/firmware/<name>/<name>.hex`.
pub fn with_firmware(name: &str, test: &str) -> TempDir {
    let dir = project(test);
    fs::copy(firmware(name), dir.path().join("build/button.ino.hex")).unwrap();
    dir
}

pub fn firmware(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/firmware")
        .join(name)
        .join(format!("{name}.hex"))
}

pub fn edit_scene(dir: &TempDir, from: &str, to: &str) {
    let scene = dir.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    assert!(text.contains(from), "`{from}` is not in the scene");
    fs::write(&scene, text.replace(from, to)).unwrap();
}

/// Each reply of a `--json -v` run of the project's one test.
pub fn replies_in(dir: &TempDir) -> Vec<Value> {
    let run = orbe(dir.path(), &["test", "--json", "-v"]);
    run.json()["data"]["tests"][0]["replies"]
        .as_array()
        .unwrap_or_else(|| panic!("{}", run.stdout))
        .iter()
        .map(|r| r["reply"].clone())
        .collect()
}

pub fn replies(test: &str) -> Vec<Value> {
    replies_in(&project(test))
}

/// The error a one-test run stopped on, from `--json`, and its test entry. The exit code is
/// `orbe check`'s for that error.
pub fn failed(test: &str) -> (Value, Value) {
    let dir = project(test);
    let run = orbe(dir.path(), &["test", "--json"]);
    let entry = run.json()["data"]["tests"][0].clone();
    let exit = match entry["error"]["code"].as_str() {
        Some("usage") => 2,
        Some("io_error") => 3,
        _ => 1,
    };
    assert_eq!(run.code, exit, "{}", run.stdout);
    (entry["error"].clone(), entry)
}

pub fn change<'a>(reply: &'a Value, kind: &str, reference: &str) -> &'a Value {
    reply["data"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == kind && c["ref"] == reference)
        .unwrap_or_else(|| panic!("no {kind} change on {reference}: {reply}"))
}
