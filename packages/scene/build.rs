//! Embeds the repo's `parts/` folder, the catalog this binary was built with, and its `blocks/`
//! folder, the Blocks `orbe add` installs. The reader compares a project's sealed copies against
//! the catalog to name the field an edit changed, and reads a Block's parts from it. A wasm32
//! build is a browser's, which is handed the parts it runs from a registry, so it embeds neither.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

#[path = "src/published.rs"]
mod published;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let browser = env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    embed(
        &manifest.join("../../parts"),
        &out.join("catalog.rs"),
        browser,
    );
    embed(
        &manifest.join("../../blocks"),
        &out.join("blocks.rs"),
        browser,
    );
}

/// Writes `FILES`, each published file below `dir` by its `/`-separated path from `dir`, sorted,
/// or none. Build output is left out.
fn embed(dir: &Path, out: &Path, none: bool) {
    let dir = dir.canonicalize().unwrap();
    println!("cargo:rerun-if-changed={}", dir.display());
    let files = if none {
        Vec::new()
    } else {
        watch(&dir);
        published::published_paths(&dir).unwrap()
    };
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

/// Watches every path under `dir`, build output included, so a new file rebuilds the catalog.
/// [`published::published_paths`] decides which of them are embedded.
fn watch(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            watch(&path);
        }
    }
}
