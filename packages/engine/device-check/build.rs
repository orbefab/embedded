//! Writes one module per `packages/engine/src/device/<kind>/model.rs`, for `src/lib.rs` to
//! include.

use std::path::Path;
use std::{env, fs};

fn main() {
    let devices = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/device");
    println!("cargo::rerun-if-changed={}", devices.display());
    let mut kinds: Vec<_> = fs::read_dir(&devices)
        .expect("the engine's device folder")
        .map(|entry| entry.expect("a device folder entry").path())
        .filter(|path| path.join("model.rs").is_file())
        .collect();
    kinds.sort();
    assert!(!kinds.is_empty(), "no device has a model.rs");
    let mut out = String::new();
    for kind in kinds {
        let model = kind
            .join("model.rs")
            .canonicalize()
            .expect("a model.rs path");
        println!("cargo::rerun-if-changed={}", kind.display());
        let name = kind
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a UTF-8 kind");
        out += &format!("#[path = {:?}]\nmod {name};\n", model.display().to_string());
    }
    let path = Path::new(&env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("devices.rs");
    fs::write(path, out).expect("the generated module list");
}
