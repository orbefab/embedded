//! Builds the viewer page into `orbe` when `apps/viewer/dist` holds one, so `orbe show` serves
//! it with nothing else installed. Without it, `orbe` builds with no page, and `show` says how
//! to build one. Building the page needs Node, which `cargo build` never runs.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let app = manifest.join("../viewer");
    let dist = app.join("dist");
    let mut files = Vec::new();
    if dist.join("index.html").is_file() {
        walk(&dist, &mut files);
        println!("cargo:rerun-if-changed={}", dist.display());
    } else if app.is_dir() {
        println!("cargo:rerun-if-changed={}", app.display());
    }
    files.sort();
    let mut table = String::from("pub(crate) static PAGE: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let name = file
            .strip_prefix(&dist)
            .unwrap()
            .components()
            .map(|c| {
                c.as_os_str()
                    .to_str()
                    .expect("a UTF-8 file name in the page")
            })
            .collect::<Vec<_>>()
            .join("/");
        let path = file.to_str().expect("a UTF-8 path to the page");
        writeln!(table, "    ({name:?}, include_bytes!({path:?})),").unwrap();
    }
    table.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("page.rs");
    std::fs::write(out, table).unwrap();
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, files);
        } else {
            files.push(path);
        }
    }
}
