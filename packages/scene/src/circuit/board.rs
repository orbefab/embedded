//! One catalog part as a single-part Block, read from a `parts/` directory the caller names.
//! Nothing here is built into the binary.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::diagnostic::Findings;
use crate::files::Files;
use crate::project::BLOCK;
use crate::source::Sources;
use crate::syntax::{self, Node};

/// Files of a Block whose scene holds one catalog part `id`. The scene file is `scene.jsonc`.
/// `parts` is the directory of part folders. The Block reads `id` and the parts it uses.
pub fn one_part(parts: &Path, id: &str) -> Files {
    let name = id.replace('-', "_");
    let block = format!(
        r#"{{ "format": 1, "block": "{id}", "version": "0.0.0", "title": "{id}", "board": "{id}", "license": "MIT" }}"#
    );
    let scene = format!(r#"{{ "format": 1, "parts": {{ "{name}": {{ "type": "{id}" }} }} }}"#);
    Files::memory([
        (BLOCK, block.into_bytes()),
        ("scene.jsonc", scene.into_bytes()),
    ])
    .with_catalog(catalog_of(parts, id))
}

/// The part `id` and every part its `uses` names, flat, as a Block's catalog.
fn catalog_of(parts: &Path, root_id: &str) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pending = vec![root_id.to_string()];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let dir = parts.join(&id);
        let Ok(paths) = crate::published::published_paths(&dir) else {
            continue;
        };
        let mut files = BTreeMap::new();
        for path in paths {
            let Ok(rel) = path.strip_prefix(&dir) else {
                continue;
            };
            let rel = rel
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if rel.is_empty() {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            files.insert(rel, bytes);
        }
        if let Some(text) = files
            .get("part.jsonc")
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        {
            pending.extend(use_ids(text));
        }
        for (rel, bytes) in files {
            out.push((format!("{id}/{rel}"), bytes));
        }
    }
    out
}

fn use_ids(text: &str) -> Vec<String> {
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new("part.jsonc"), text.to_string());
    let mut findings = Findings::default();
    let Some(tree) = syntax::parse(&sources, file, &mut findings) else {
        return Vec::new();
    };
    tree.get("uses")
        .and_then(Node::props)
        .into_iter()
        .flatten()
        .map(|prop| prop.key.clone())
        .collect()
}
