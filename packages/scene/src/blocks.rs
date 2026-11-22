//! A Block is a folder that holds `block.jsonc`. The repo keeps them in `blocks/`, beside
//! `parts/`. Nothing here is built into the binary.

use std::collections::BTreeMap;
use std::path::Path;

use crate::project::BLOCK;

/// Block ids in `dir`, in path order. A Block is a folder that holds `block.jsonc`.
pub(crate) fn ids(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join(BLOCK).is_file() {
            found.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    found.sort();
    found
}

/// Every published file of the Block `id`, by its path inside the Block.
pub(crate) fn folder(dir: &Path, id: &str) -> Option<BTreeMap<String, Vec<u8>>> {
    let root = dir.join(id);
    if !root.join(BLOCK).is_file() {
        return None;
    }
    let paths = crate::published::published_paths(&root).ok()?;
    let mut files = BTreeMap::new();
    for path in paths {
        let rel = path
            .strip_prefix(&root)
            .ok()?
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if !rel.is_empty() {
            files.insert(rel, std::fs::read(&path).ok()?);
        }
    }
    (!files.is_empty()).then_some(files)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn blocks_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../blocks")
    }

    /// The annotation names each file the Block holds. A file outside that list, including one
    /// added later, is not covered.
    #[test]
    fn an_annotation_covers_the_block_files_and_not_a_later_file() {
        let dir = blocks_dir();
        for id in ids(&dir) {
            let files = folder(&dir, &id).unwrap();
            let text = std::str::from_utf8(&files["REUSE.toml"]).unwrap();
            let reuse = crate::reuse::Reuse::parse(text).unwrap();
            for path in files.keys() {
                assert!(
                    reuse.license(path, None).is_some(),
                    "{id}: {path} has no annotation"
                );
            }
            assert_eq!(reuse.license("parts.lock", None), None, "{id}: parts.lock");
            for extra in [
                "mine.txt",
                "src/mine.ino",
                "firmware/extra.ino",
                "notes.md",
                "parts/led-5mm/part.jsonc",
            ] {
                assert_eq!(reuse.license(extra, None), None, "{id}: {extra} is covered");
            }
        }
    }
}
