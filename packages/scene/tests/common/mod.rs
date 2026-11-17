//! A temporary project built from a fixture, as `orbe add` would leave it: the fixture's own
//! files, a copy of every catalog part, and the firmware build output. `parts.lock` is written
//! from those copies, so a part change does not rewrite a committed lock.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use orbe_scene::{REGISTRY, folder_hash};
use tempfile::TempDir;

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn fixture(name: &str) -> PathBuf {
    repo().join("packages/scene/tests/fixtures").join(name)
}

/// The `button.ino.hex` the first target builds: the engine's own build of the same sketch.
pub fn firmware() -> PathBuf {
    repo().join("packages/engine/tests/firmware/sketch/button.hex")
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

/// The ids a project's `parts.lock` lists.
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
    line_for(&project.join("parts").join(id), id)
}

fn line_for(folder: &Path, id: &str) -> String {
    let text = fs::read_to_string(folder.join("part.jsonc")).unwrap();
    let version = text
        .split("\"version\": \"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap();
    format!("{id} {version} {REGISTRY} {}", folder_hash(folder).unwrap())
}

/// The catalog's `parts.lock` text. A process hashes each part once. Nextest runs each test
/// in its own process, so the text is also stored under the test temp dir and reused while
/// every catalog file's size and mtime still match.
fn catalog_lock() -> &'static str {
    static LOCK: OnceLock<String> = OnceLock::new();
    LOCK.get_or_init(load_catalog_lock)
}

fn load_catalog_lock() -> String {
    let catalog = repo().join("parts");
    let stamp = catalog_stamp(&catalog);
    let cache =
        option_env!("CARGO_TARGET_TMPDIR").map(|dir| Path::new(dir).join("catalog-parts.lock"));
    if let Some(path) = &cache
        && let Some(text) = read_cached(path, &stamp)
    {
        return text;
    }
    let text = compute_catalog_lock(&catalog);
    if let Some(path) = &cache {
        let _ = write_cached(path, &stamp, &text);
    }
    text
}

fn catalog_stamp(catalog: &Path) -> String {
    let mut rows = Vec::new();
    fn walk(root: &Path, dir: &Path, rows: &mut Vec<String>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(root, &path, rows);
            } else {
                let meta = fs::metadata(&path).unwrap();
                let modified = meta
                    .modified()
                    .unwrap()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let rel = path.strip_prefix(root).unwrap().to_string_lossy();
                rows.push(format!("{rel} {} {modified}\n", meta.len()));
            }
        }
    }
    walk(catalog, catalog, &mut rows);
    rows.concat()
}

fn read_cached(path: &Path, stamp: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let body = text.strip_prefix(stamp)?.strip_prefix("---\n")?;
    Some(body.to_string())
}

fn write_cached(path: &Path, stamp: &str, body: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("lock.tmp");
    fs::write(&tmp, format!("{stamp}---\n{body}"))?;
    fs::rename(&tmp, path)
}

fn compute_catalog_lock(catalog: &Path) -> String {
    let mut ids: Vec<String> = fs::read_dir(catalog)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| catalog.join(id).join("part.jsonc").is_file())
        .collect();
    ids.sort();
    let mut lines = String::new();
    for id in &ids {
        lines.push_str(&line_for(&catalog.join(id), id));
        lines.push('\n');
    }
    lines
}

pub fn project(name: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture(name), dir.path());
    let catalog = repo().join("parts");
    let lock = catalog_lock();
    for id in lock
        .lines()
        .filter_map(|line| line.split_whitespace().next())
    {
        copy_dir(&catalog.join(id), &dir.path().join("parts").join(id));
    }
    fs::write(dir.path().join("parts.lock"), lock).unwrap();
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
    let mut cases: Vec<PathBuf> = fs::read_dir(repo().join("packages/scene/tests").join(kind))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    cases
}
