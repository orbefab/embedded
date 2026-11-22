//! The Blocks built into this binary, from the repo's `blocks/`. A wasm32 build embeds none.

use std::collections::BTreeMap;

include!(concat!(env!("OUT_DIR"), "/blocks.rs"));

/// Block ids, in path order.
pub(crate) fn ids() -> impl Iterator<Item = &'static str> {
    FILES.iter().filter_map(|(path, _)| {
        let (id, name) = path.split_once('/')?;
        (name == crate::project::BLOCK).then_some(id)
    })
}

/// Every published file of the Block `id`, by its path inside the Block.
pub(crate) fn folder(id: &str) -> Option<BTreeMap<String, Vec<u8>>> {
    let prefix = format!("{id}/");
    let mut files = BTreeMap::new();
    for (path, bytes) in FILES {
        let Some(rel) = path.strip_prefix(prefix.as_str()) else {
            continue;
        };
        if !rel.is_empty() {
            files.insert((*rel).to_string(), bytes.to_vec());
        }
    }
    (!files.is_empty()).then_some(files)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// The annotation names each file the Block holds. A file outside that list, including one
    /// added later, is not covered.
    #[test]
    fn an_annotation_covers_the_block_files_and_not_a_later_file() {
        for id in ids() {
            let files = folder(id).unwrap();
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
