//! A temporary project built from a fixture, as `orbe add` would leave it: the fixture's own
//! files, a copy of each catalog part its `parts.lock` lists, and the firmware build output.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use orbe_scene::{REGISTRY, folder_hash};
use tempfile::TempDir;

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn fixture(name: &str) -> PathBuf {
    repo().join("crates/orbe-scene/tests/fixtures").join(name)
}

/// The `button.ino.hex` the first target builds: the engine's own build of the same sketch.
pub fn firmware() -> PathBuf {
    repo().join("engine/firmware/sketch/button.hex")
}

pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// The ids a fixture's `parts.lock` lists.
pub fn locked_ids(project: &Path) -> Vec<String> {
    fs::read_to_string(project.join("parts.lock"))
        .unwrap()
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// The `parts.lock` line `orbe add` will write for the catalog copy at `parts/<id>/`.
pub fn lock_line(project: &Path, id: &str) -> String {
    let folder = project.join("parts").join(id);
    let text = fs::read_to_string(folder.join("part.jsonc")).unwrap();
    let version = text
        .split("\"version\": \"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap();
    format!(
        "{id} {version} {REGISTRY} {}",
        folder_hash(&folder).unwrap()
    )
}

pub fn project(name: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture(name), dir.path());
    for id in locked_ids(dir.path()) {
        copy_dir(
            &repo().join("parts").join(&id),
            &dir.path().join("parts").join(&id),
        );
    }
    fs::create_dir_all(dir.path().join("build")).unwrap();
    fs::copy(firmware(), dir.path().join("build/button.ino.hex")).unwrap();
    dir
}

/// The `nano-button` project with a case folder from `tests/bad/` or `tests/good/` laid over
/// it. An `edit` file changes one project file: its path, the text, and the new text.
pub fn overlaid(case: &Path) -> TempDir {
    let project = project("nano-button");
    for entry in fs::read_dir(case).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap();
        if name == "expect" || name == "edit" {
            continue;
        }
        let target = project.path().join(name);
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
    if let Ok(edit) = fs::read_to_string(case.join("edit")) {
        let lines: Vec<&str> = edit.lines().collect();
        let target = project.path().join(lines[0]);
        let text = fs::read_to_string(&target).unwrap();
        assert!(
            text.contains(lines[1]),
            "{}: `{}` is not in {}",
            case.display(),
            lines[1],
            lines[0]
        );
        fs::write(&target, text.replacen(lines[1], lines[2], 1)).unwrap();
    }
    project
}

/// The case folders in `tests/<kind>/`, sorted.
pub fn cases(kind: &str) -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(repo().join("crates/orbe-scene/tests").join(kind))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    cases
}
