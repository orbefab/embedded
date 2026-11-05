//! Embeds the repo's `parts/` folder, the catalog this binary was built with. The reader compares
//! a project's sealed copies against it to name the field an edit changed.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let parts = manifest.join("../../parts").canonicalize().unwrap();
    println!("cargo:rerun-if-changed={}", parts.display());
    let mut files = Vec::new();
    walk(&parts, &mut files);
    files.sort();
    let mut out = String::from("pub(crate) static FILES: &[(&str, &[u8])] = &[\n");
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let rel = file.strip_prefix(&parts).unwrap();
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>();
        writeln!(
            out,
            "    ({:?}, include_bytes!({:?})),",
            rel.join("/"),
            file.display().to_string()
        )
        .unwrap();
    }
    out.push_str("];\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("catalog.rs"),
        out,
    )
    .unwrap();
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
