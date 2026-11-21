#![allow(dead_code, reason = "each test file uses part of this")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use orbe_registry::{Error, build};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn record(output: &BTreeMap<String, Vec<u8>>, address: &str) -> Value {
    let bytes = output
        .get(&format!("{address}.json"))
        .unwrap_or_else(|| panic!("no record {address}"));
    serde_json::from_slice(bytes).unwrap()
}

pub fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub const MANIFEST: &str = r#"{ "format": 1, "name": "@acme" }"#;

pub const LAMP: &str = r#"{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "params": { "resistance": { "unit": "ohm", "default": "220", "min_exclusive": 0, "sets": "/resistance" } },
  "model": { "kind": "resistor", "ratings": { "power": null } }
}
"#;

/// A registry source with one part and one Block, as a third party would write it.
pub fn third_party() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "registry.jsonc", MANIFEST);
    put(dir.path(), "parts/lamp/part.jsonc", LAMP);
    put(
        dir.path(),
        "blocks/lamp-demo/block.jsonc",
        r#"{ "format": 1, "block": "lamp-demo", "version": "0.1.0", "title": "A lamp", "board": "lamp", "license": "MIT" }"#,
    );
    put(
        dir.path(),
        "blocks/lamp-demo/scene.jsonc",
        r#"{ "format": 1, "parts": { "l1": { "type": "lamp" } }, "ground": "l1.a" }"#,
    );
    dir
}

pub fn written(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy();
                out.insert(rel.replace('\\', "/"), fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

pub fn invalid(src: &Path) -> String {
    match build(src) {
        Err(Error::Invalid(message)) => message,
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("{} built", src.display()),
    }
}
