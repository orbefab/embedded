//! Embeds the repo's `parts/` folder, the catalog this binary was built with, and its `blocks/`
//! folder, the Blocks `orbe add` installs. The reader compares a project's sealed copies against
//! the catalog to name the field an edit changed, and reads a Block's parts from it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    embed(&manifest.join("../../parts"), &out.join("catalog.rs"));
    embed(&manifest.join("../../blocks"), &out.join("blocks.rs"));
}

/// Writes `FILES`, each file below `dir` by its `/`-separated path from `dir`, sorted.
fn embed(dir: &Path, out: &Path) {
    let dir = dir.canonicalize().unwrap();
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();
    let mut text = String::from("pub(crate) static FILES: &[(&str, &[u8])] = &[\n");
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let rel = file.strip_prefix(&dir).unwrap();
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>();
        writeln!(
            text,
            "    ({:?}, include_bytes!({:?})),",
            rel.join("/"),
            file.display().to_string()
        )
        .unwrap();
    }
    text.push_str("];\n");
    fs::write(out, text).unwrap();
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            walk(&path, files);
        } else {
            files.push(path);
        }
    }
}
