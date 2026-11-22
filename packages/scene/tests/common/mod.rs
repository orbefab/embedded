//! A temporary project built from a fixture, as `orbe add` would leave it: the fixture's own
//! files, a copy of every catalog part, and the firmware build output.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

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

/// Copies each part named in `dest`'s `uses` to `dest/parts/<id>/`, then the same for those.
/// A cycle is skipped. This is the layout `orbe add` writes.
pub fn nest_uses(catalog: &Path, dest: &Path, stack: &mut Vec<String>) {
    let Ok(text) = fs::read_to_string(dest.join("part.jsonc")) else {
        return;
    };
    for dep in use_ids(&text) {
        if stack.iter().any(|seen| seen == &dep) {
            continue;
        }
        let from = catalog.join(&dep);
        if !from.join("part.jsonc").is_file() {
            continue;
        }
        let child = dest.join("parts").join(&dep);
        if !child.join("part.jsonc").is_file() {
            copy_dir(&from, &child);
        }
        stack.push(dep);
        nest_uses(catalog, &child, stack);
        stack.pop();
    }
}

/// The ids in a part file's `uses` object. A string is a key when `:` follows it.
fn use_ids(text: &str) -> Vec<String> {
    let Some(at) = text.find("\"uses\"") else {
        return Vec::new();
    };
    let rest = &text[at + "\"uses\"".len()..];
    let Some(start) = rest.find('{') else {
        return Vec::new();
    };
    let body = &rest[start + 1..];
    let mut ids = Vec::new();
    let mut depth = 1usize;
    let mut i = 0usize;
    let bytes = body.as_bytes();
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'"' if depth == 1 => {
                let Some(end) = body[i + 1..].find('"') else {
                    break;
                };
                let key = &body[i + 1..i + 1 + end];
                let after = body[i + 1 + end + 1..].trim_start();
                if after.starts_with(':') {
                    ids.push(key.to_string());
                }
                i += end + 2;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    ids
}

/// The `button.ino.hex` the first target builds: the Block's own build of the same sketch.
pub fn firmware() -> PathBuf {
    repo().join("blocks/nano-button-led/build/button.ino.hex")
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

/// True when this process can read a file with mode 000. Root can, so a chmod test
/// would not be the repair.
pub fn mode_000_reads() -> bool {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hidden");
    fs::write(&path, b"x").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    fs::read(&path).is_ok()
}

pub fn project(name: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture(name), dir.path());
    let catalog = repo().join("parts");
    for id in fs::read_dir(&catalog).unwrap() {
        let id = id.unwrap().file_name();
        let from = catalog.join(&id);
        if from.join("part.jsonc").is_file() {
            copy_dir(&from, &dir.path().join("parts").join(&id));
        }
    }
    let parts = dir.path().join("parts");
    let ids: Vec<_> = fs::read_dir(&parts)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    for id in ids {
        nest_uses(&catalog, &parts.join(id), &mut Vec::new());
    }
    fs::create_dir_all(dir.path().join("build")).unwrap();
    fs::copy(firmware(), dir.path().join("build/button.ino.hex")).unwrap();
    dir
}

/// The `nano-button` project, or the fixture a `base` file names, with a case folder from
/// `tests/bad/` or `tests/good/` laid over it. An `edit` file changes one project file: its
/// path, the text, and the new text. A `blobs` file lists `<path> <sha256>` lines, each a blob
/// from [`blob_cache`] copied to that project path, for a file no fixture commits, such as a
/// ROM.
pub fn overlaid(case: &Path) -> TempDir {
    let base = fs::read_to_string(case.join("base"));
    let project = project(base.as_deref().map_or("nano-button", str::trim));
    for entry in fs::read_dir(case).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap();
        if ["expect", "edit", "base", "blobs"]
            .iter()
            .any(|n| name == *n)
        {
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
    if let Ok(blobs) = fs::read_to_string(case.join("blobs")) {
        for line in blobs.lines() {
            let (path, sha256) = line.split_once(' ').expect("`<path> <sha256>`");
            let blob = blob_cache().join(sha256);
            fs::copy(&blob, project.path().join(path)).unwrap_or_else(|_| {
                panic!(
                    "{}: the blob is not at {}; run packages/engine/scripts/blobs.sh",
                    case.display(),
                    blob.display()
                )
            });
        }
    }
    project
}

/// The folder `packages/engine/scripts/blobs.sh` fills: `$ORBE_BLOBS`, else
/// `$XDG_CACHE_HOME/orbe/blobs`, else `~/.cache/orbe/blobs`.
pub fn blob_cache() -> PathBuf {
    let set = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    set("ORBE_BLOBS").map(PathBuf::from).unwrap_or_else(|| {
        set("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".cache")
            })
            .join("orbe/blobs")
    })
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
