//! The files a part or Block folder publishes.
//!
//! [`is_published`] is that set. The folder hash, the catalog this binary embeds, the registry
//! build and `orbe add` each keep a file when it says so, and leave it out otherwise. Build
//! output is left out, and so are `model.py` and `asset.json`, the inputs a part's 3D model is
//! built from. The GLB still ships. The names are fixed, because an installed project has no
//! copy of the repo's `.gitignore` to read. Any other file is published, including one an
//! author added beside `part.jsonc`.

use std::io;
use std::path::{Path, PathBuf};

/// Directory names the repo's `.gitignore` lists that are Python build output and can sit in a
/// part folder. Running `model.py` writes `__pycache__`. A test run can write the other two.
const BUILD_DIRS: &[&str] = &["__pycache__", ".pytest_cache", ".venv"];

/// Whether `rel` is a file the folder publishes. `rel` is a `/`-separated path inside the
/// folder, or below `parts/` or `blocks/` with the id as its first segment.
pub fn is_published(rel: &str) -> bool {
    rel.split('/')
        .all(|segment| !is_build_dir(segment) && !is_bytecode(segment) && !is_model_input(segment))
}

/// `model.py` builds the part's body and `asset.json` records that build. A project needs the
/// GLB, not these two.
fn is_model_input(name: &str) -> bool {
    name == "model.py" || name == "asset.json"
}

fn is_build_dir(name: &str) -> bool {
    BUILD_DIRS.contains(&name)
}

/// A `.pyc` beside the script. Python writes that when it does not use `__pycache__`.
fn is_bytecode(name: &str) -> bool {
    name.ends_with(".pyc")
}

/// The published files under `dir`, as absolute paths, sorted. `build.rs` embeds this set.
pub(crate) fn published_paths(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if !is_published(&rel_path(root, &path)) {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::is_published;

    #[test]
    fn build_output_is_unpublished_and_an_author_file_is_published() {
        for rel in [
            "a.pyc",
            "__pycache__/part.cpython-312.pyc",
            "lamp/__pycache__/part.cpython-312.pyc",
            "lamp/a.pyc",
            ".pytest_cache/v",
            ".venv/pyvenv.cfg",
            "model.py",
            "asset.json",
            "lamp/model.py",
            "lamp/asset.json",
        ] {
            assert!(!is_published(rel), "{rel}");
        }
        for rel in [
            "part.jsonc",
            "notes.txt",
            "lamp/notes.txt",
            "lamp/lamp.glb",
            ".env",
            ".gitignore",
        ] {
            assert!(is_published(rel), "{rel}");
        }
    }
}
