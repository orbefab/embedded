//! A temporary project built from a fixture, as `orbe add` would leave it: the fixture's own
//! files, a copy of every catalog part, and the firmware build output. `parts.lock` is written
//! from those copies, so a part change does not rewrite a committed lock.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

/// The `button.ino.hex` the first target builds: the Block's own build of the same sketch.
pub fn firmware() -> PathBuf {
    repo().join("blocks/nano-button-led/firmware/button.hex")
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

/// The catalog's `parts.lock` text. A process hashes each part once.
fn catalog_lock() -> &'static str {
    static LOCK: OnceLock<String> = OnceLock::new();
    LOCK.get_or_init(|| compute_catalog_lock(&repo().join("parts")))
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
