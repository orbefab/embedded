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

    /// `firmware/NOTICE` states which file carries which licence. The Block ships no
    /// `REUSE.toml`.
    #[test]
    fn the_notice_states_each_file_licence() {
        let dir = blocks_dir();
        for id in ids(&dir) {
            let files = folder(&dir, &id).unwrap();
            assert!(
                !files.contains_key("REUSE.toml"),
                "{id} still ships REUSE.toml"
            );
            let notice = std::str::from_utf8(
                files
                    .get("firmware/NOTICE")
                    .unwrap_or_else(|| panic!("{id} has no firmware/NOTICE")),
            )
            .unwrap();
            for fact in [
                "MIT OR Apache-2.0",
                "LGPL-2.1-or-later",
                "BSD-3-Clause",
                "LICENSE-runtime",
                "LICENSE-libc",
            ] {
                assert!(notice.contains(fact), "{id}: NOTICE does not state {fact}");
            }
        }
    }
}
