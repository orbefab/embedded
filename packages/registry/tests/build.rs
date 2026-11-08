//! The registry this repo's `parts/` and `blocks/` build into, and registries of other folders.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use orbe_registry::{Error, INDEX_SCHEMA, ITEM_SCHEMA, Item, build};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn record(output: &BTreeMap<String, Vec<u8>>, address: &str) -> Value {
    let bytes = output
        .get(&format!("{address}.json"))
        .unwrap_or_else(|| panic!("no record {address}"));
    serde_json::from_slice(bytes).unwrap()
}

/// A record's text with the values that change whenever a file's bytes do replaced by names.
fn without_hashes(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let indent = &line[..line.len() - line.trim_start().len()];
        let comma = if line.ends_with(',') { "," } else { "" };
        let field = line.trim_start().split_once(": ").map(|(key, _)| key);
        let value = match field {
            Some("\"hash\"") => Some("\"sha256:<folder hash>\""),
            Some("\"sha256\"") => Some("\"<sha256>\""),
            Some("\"url\"") if line.contains("\"files/") => Some("\"files/<sha256>\""),
            Some("\"bytes\"") => Some("0"),
            _ => None,
        };
        match (field, value) {
            (Some(key), Some(value)) => out.push_str(&format!("{indent}{key}: {value}{comma}")),
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

#[test]
fn records_of_three_parts_and_a_block_match_their_snapshots() {
    let registry = build(&repo()).unwrap();
    let bless = std::env::var_os("ORBE_BLESS").is_some();
    for address in [
        "led-5mm@0.1.0",
        "atmega328p-dip28@0.1.0",
        "nano-board@0.1.0",
        "nano-button-led",
    ] {
        let text = std::str::from_utf8(&registry.output[&format!("{address}.json")]).unwrap();
        let got = without_hashes(text);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/records")
            .join(format!("{address}.json"));
        if bless {
            fs::write(&path, &got).unwrap();
            continue;
        }
        let want = fs::read_to_string(&path).unwrap();
        assert!(
            got == want,
            "{address} differs from tests/records. ORBE_BLESS=1 rewrites the snapshots\ngot:\n{got}"
        );
    }
}

fn written(dir: &Path) -> BTreeMap<String, Vec<u8>> {
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

#[test]
fn two_builds_write_the_same_bytes() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    build(&repo()).unwrap().write(first.path()).unwrap();
    build(&repo()).unwrap().write(second.path()).unwrap();
    let a = written(first.path());
    let b = written(second.path());
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    for (path, bytes) in &a {
        assert!(&b[path] == bytes, "{path} differs between two builds");
    }
    assert_eq!(a.len(), build(&repo()).unwrap().report.files);
}

/// Every row's record and every record's file is in the output, under the URL the row or
/// record gives, with the SHA-256 and size it gives.
#[test]
fn every_url_holds_the_bytes_its_hash_names() {
    let registry = build(&repo()).unwrap();
    let index: Value = serde_json::from_slice(&registry.output["index.json"]).unwrap();
    assert_eq!(index["$schema"], INDEX_SCHEMA);
    let rows = index["items"].as_array().unwrap();
    assert_eq!(rows.len(), registry.items.len());
    for row in rows {
        let url = row["url"].as_str().unwrap();
        let bytes = &registry.output[url];
        assert_eq!(row["sha256"], sha256(bytes), "{url}");
        assert_eq!(row["bytes"], bytes.len(), "{url}");
        let record: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(record["name"], row["name"], "{url}");
        assert_eq!(record["type"], row["type"], "{url}");
        for file in record["files"].as_array().unwrap() {
            let sha = file["sha256"].as_str().unwrap();
            assert_eq!(file["url"], format!("files/{sha}"));
            let stored = &registry.output[&format!("files/{sha}")];
            assert_eq!(sha256(stored), sha, "{url} {}", file["path"]);
            assert_eq!(file["bytes"], stored.len(), "{url} {}", file["path"]);
            assert!(!file["license"].as_str().unwrap().is_empty());
        }
    }
    let block = record(&registry.output, "nano-button-led");
    for file in block["files"].as_array().unwrap() {
        assert_eq!(file["license"], "(MIT OR Apache-2.0) AND LGPL-2.1-or-later");
    }
}

#[test]
fn a_block_depends_on_every_part_it_runs_at_the_version_it_was_tested_with() {
    let registry = build(&repo()).unwrap();
    let block = record(&registry.output, "nano-button-led");
    let deps: Vec<&str> = block["registryDependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|dep| dep.as_str().unwrap())
        .collect();
    for direct in [
        "nano-board@0.1.0",
        "tactile-switch-6mm@0.1.0",
        "resistor-axial@0.1.0",
        "led-5mm@0.1.0",
    ] {
        assert!(deps.contains(&direct), "{direct} in {deps:?}");
    }
    let board = record(&registry.output, "nano-board@0.1.0");
    for through_the_board in board["registryDependencies"].as_array().unwrap() {
        assert!(deps.contains(&through_the_board.as_str().unwrap()));
    }
    assert_eq!(block["board"], "nano-board");
    assert!(block.get("version").is_none());
}

#[test]
fn the_item_schema_has_the_part_block_and_model_types() {
    let registry = build(&repo()).unwrap();
    let schema: Value = serde_json::from_slice(&registry.output[ITEM_SCHEMA]).unwrap();
    let types: Vec<&Value> = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variant| &variant["properties"]["type"]["const"])
        .collect();
    assert_eq!(types, ["part", "block", "model"]);
    assert_eq!(registry.report.models, 0);
}

const LAMP: &str = r#"{
  "format": 1,
  "part": "lamp",
  "version": "1.2.0",
  "title": "Lamp",
  "pins": { "a": [1], "b": [2] },
  "model": { "kind": "resistor" },
}
"#;

fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn third_party() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "parts/lamp/part.jsonc", LAMP);
    put(
        dir.path(),
        "blocks/lamp-demo/block.jsonc",
        r#"{ "format": 1, "block": "lamp-demo", "title": "A lamp", "board": "lamp", "license": "MIT" }"#,
    );
    put(
        dir.path(),
        "blocks/lamp-demo/scene.jsonc",
        r#"{ "format": 1, "parts": { "l1": { "type": "lamp" } } }"#,
    );
    dir
}

#[test]
fn another_folder_of_parts_and_blocks_builds_and_counts_what_it_lacks() {
    let src = third_party();
    let registry = build(src.path()).unwrap();
    let names: Vec<String> = registry.items.iter().map(|item| item.address()).collect();
    assert_eq!(names, ["lamp@1.2.0", "lamp-demo"]);
    let block = record(&registry.output, "lamp-demo");
    assert_eq!(
        block["registryDependencies"],
        serde_json::json!(["lamp@1.2.0"])
    );
    let lamp = record(&registry.output, "lamp@1.2.0");
    for absent in ["description", "categories", "docs", "buy"] {
        assert!(lamp.get(absent).is_none(), "{absent}");
    }
    assert_eq!(lamp["license"], "MIT OR Apache-2.0");

    let report = &registry.report;
    assert_eq!((report.parts, report.blocks, report.models), (1, 1, 0));
    let parts = &report.missing.parts;
    assert_eq!(
        [
            parts.parts,
            parts.description,
            parts.categories,
            parts.docs,
            parts.datasheet_url,
            parts.buy
        ],
        [1; 6]
    );
    let blocks = &report.missing.blocks;
    assert_eq!(
        [blocks.blocks, blocks.description, blocks.categories],
        [1; 3]
    );
    assert_eq!(
        report.missing.warnings(),
        [
            "1 parts: 1 lack a description, 1 lack categories, 1 lack a datasheet, 1 lack a datasheet url, 1 lack buy",
            "1 Blocks: 1 lack a description, 1 lack categories",
        ]
    );
    let out = src.path().join("out");
    registry.write(&out).unwrap();
    assert_eq!(written(&out).len(), report.files);
    assert_eq!(
        registry.write(&out),
        Err(Error::NotEmpty(out.clone())),
        "a second write into the same folder"
    );
}

#[test]
fn two_parts_with_one_files_bytes_store_it_once() {
    let src = tempfile::tempdir().unwrap();
    let glb = "binary model bytes";
    for id in ["lamp", "lamp-red"] {
        put(
            src.path(),
            &format!("parts/{id}/part.jsonc"),
            &LAMP.replace("\"lamp\"", &format!("\"{id}\"")),
        );
        put(src.path(), &format!("parts/{id}/lamp.glb"), glb);
    }
    let registry = build(src.path()).unwrap();
    assert_eq!(registry.report.listed, 4);
    assert_eq!(registry.report.stored, 3);
    let stored = format!("files/{}", sha256(glb.as_bytes()));
    assert_eq!(registry.output[&stored], glb.as_bytes());
    for address in ["lamp@1.2.0", "lamp-red@1.2.0"] {
        let lamp = record(&registry.output, address);
        let urls: Vec<&Value> = lamp["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| &file["url"])
            .collect();
        assert!(urls.contains(&&Value::from(stored.as_str())), "{address}");
    }
}

fn invalid(src: &Path) -> String {
    match build(src) {
        Err(Error::Invalid(message)) => message,
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("{} built", src.display()),
    }
}

#[test]
fn a_block_whose_parts_are_not_in_the_registry_does_not_build() {
    let src = third_party();
    put(
        src.path(),
        "blocks/lamp-demo/scene.jsonc",
        r#"{ "format": 1, "parts": { "l1": { "type": "lamp" }, "r1": { "type": "resistor-axial" } } }"#,
    );
    assert!(
        invalid(src.path()).contains("`resistor-axial`, which this registry lacks"),
        "{}",
        invalid(src.path())
    );
    put(
        src.path(),
        "blocks/lamp-demo/scene.jsonc",
        r#"{ "format": 1, "parts": { "l1": { "type": "@acme/lamp" } } }"#,
    );
    assert!(invalid(src.path()).contains("`@acme/lamp`"));
}

#[test]
fn one_name_names_one_item() {
    let src = third_party();
    put(src.path(), "blocks/lamp/block.jsonc", "{}");
    assert!(invalid(src.path()).contains("both a part and a Block"));

    let src = third_party();
    put(
        src.path(),
        "parts/index/part.jsonc",
        &LAMP.replace("\"lamp\"", "\"index\""),
    );
    assert!(invalid(src.path()).contains("the index's address"));

    let src = third_party();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &LAMP.replace("\"lamp\"", "\"bulb\""),
    );
    assert!(invalid(src.path()).contains("names `bulb`"));
}

#[test]
fn a_part_that_uses_a_version_the_registry_lacks_does_not_build() {
    let src = third_party();
    put(
        src.path(),
        "parts/lamp-pair/part.jsonc",
        r#"{ "format": 1, "part": "lamp-pair", "version": "0.1.0", "title": "Two lamps", "uses": { "lamp": "1.0.0" }, "parts": { "a": { "type": "lamp" } } }"#,
    );
    assert!(invalid(src.path()).contains("uses `lamp@1.0.0`, which these parts lack"));
}

#[test]
fn every_item_is_a_part_or_a_block_of_the_repo() {
    let registry = build(&repo()).unwrap();
    let parts = fs::read_dir(repo().join("parts")).unwrap().count();
    let blocks = fs::read_dir(repo().join("blocks")).unwrap().count();
    let (mut p, mut b) = (0, 0);
    for item in &registry.items {
        match item {
            Item::Part(_) => p += 1,
            Item::Block(_) => b += 1,
            Item::Model(_) => panic!("no folder holds a model"),
        }
    }
    assert_eq!((p, b), (parts, blocks));
}
