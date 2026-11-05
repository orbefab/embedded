//! The catalog this binary was built with: the repo's `parts/`, embedded by `build.rs`. It is
//! the registry's index for the versions it holds, so a sealed copy can be compared with it.

use std::fs;
use std::path::Path;

use crate::diagnostic::Findings;
use crate::project::hash_files;
use crate::source::Sources;
use crate::syntax::{self, Node};

include!(concat!(env!("OUT_DIR"), "/catalog.rs"));

pub(crate) struct Original {
    pub(crate) version: String,
    pub(crate) hash: String,
    pub(crate) tree: Node,
    files: Vec<(&'static str, &'static [u8])>,
}

pub(crate) fn ids() -> impl Iterator<Item = String> {
    FILES
        .iter()
        .filter_map(|(path, _)| path.strip_suffix("/part.jsonc"))
        .map(str::to_string)
}

pub(crate) fn find(id: &str) -> Option<Original> {
    let prefix = format!("{id}/");
    let files: Vec<(&'static str, &'static [u8])> = FILES
        .iter()
        .filter_map(|(path, bytes)| path.strip_prefix(prefix.as_str()).map(|rel| (rel, *bytes)))
        .collect();
    let text = std::str::from_utf8(files.iter().find(|(rel, _)| *rel == "part.jsonc")?.1).ok()?;
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new("part.jsonc"), text.to_string());
    let tree = syntax::parse(&sources, file, &mut Findings::default())?;
    let version = tree.get("version")?.str()?.to_string();
    let hash = hash_files(files.iter().copied());
    Some(Original {
        version,
        hash,
        tree,
        files,
    })
}

impl Original {
    /// The files of `folder` that differ from this version's, for a message. `None` when
    /// every file matches.
    pub(crate) fn files_differing(&self, folder: &Path) -> Option<String> {
        let mut names: Vec<String> = Vec::new();
        for (rel, bytes) in &self.files {
            match fs::read(folder.join(rel)) {
                Ok(copy) if copy == *bytes => {}
                Ok(_) if *rel == "part.jsonc" => {
                    names.push("the comments or spacing of `part.jsonc`".into())
                }
                Ok(_) => names.push(format!("`{rel}`")),
                Err(_) => names.push(format!("`{rel}`, which is missing")),
            }
        }
        let mut extra = Vec::new();
        list(folder, folder, &mut extra);
        for rel in extra
            .iter()
            .filter(|rel| !self.files.iter().any(|(r, _)| r == rel))
        {
            names.push(format!("`{rel}`, which the catalog's copy does not have"));
        }
        (!names.is_empty()).then(|| names.join(", "))
    }
}

fn list(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for path in fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
    {
        if path.is_dir() {
            list(root, &path, out);
        } else {
            out.push(crate::source::display(root, &path));
        }
    }
}
