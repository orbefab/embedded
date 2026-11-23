//! A registry built from this commit, as a file URL. Tests set `ORBE_REGISTRY` to it so
//! `orbe add`, `orbe part`, `orbe search` and `orbe blocks` never call orbefab.com.
//! Not every test binary calls every helper.
#![allow(dead_code)]

use std::cell::RefCell;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

thread_local! {
    static CACHE_DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
}

pub fn dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let dir = tmp.join("registry");
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let stamp = source_stamp(&repo);
        let lock = File::create(tmp.join("registry.lock")).unwrap();
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
        let stamp_path = tmp.join("registry.stamp");
        let ready = dir.join("index.json").is_file()
            && fs::read_to_string(&stamp_path).ok().as_deref() == Some(stamp.as_str());
        if !ready {
            if dir.exists() {
                fs::remove_dir_all(&dir).unwrap();
            }
            fs::create_dir_all(&dir).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
                .args([
                    "registry",
                    "build",
                    repo.to_str().unwrap(),
                    dir.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "registry build: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            fs::write(&stamp_path, &stamp).unwrap();
        }
        if let Ok(entries) = fs::read_dir(&tmp) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if name.starts_with("registry-") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        dir
    })
}

/// The parts, Blocks and registry manifest this commit would build. One cargo run shares
/// the registry when this does not change.
fn source_stamp(repo: &Path) -> String {
    let mut files = Vec::new();
    files.push(repo.join("registry.jsonc"));
    for name in ["parts", "blocks"] {
        walk(&repo.join(name), &mut files);
    }
    files.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for path in files {
        let rel = path.strip_prefix(repo).unwrap();
        rel.hash(&mut hasher);
        fs::read(&path).unwrap().hash(&mut hasher);
    }
    hasher.finish().to_string()
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

pub fn template() -> String {
    format!("file://{}/{{name}}.json", dir().display())
}

pub fn apply(command: &mut Command) {
    command.env("ORBE_REGISTRY", template());
    let cache = tempfile::tempdir().unwrap();
    command.env("XDG_CACHE_HOME", cache.path());
    CACHE_DIRS.with(|dirs| dirs.borrow_mut().push(cache));
}

/// Sets `ORBE_REGISTRY` when `args` is a command that fetches.
pub fn apply_if(command: &mut Command, args: &[&str]) {
    let fetches = args
        .iter()
        .any(|arg| matches!(*arg, "add" | "part" | "search" | "blocks"));
    if fetches {
        apply(command);
    }
}
