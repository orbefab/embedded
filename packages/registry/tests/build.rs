//! The registry this repo's `parts/` and `blocks/` build into, and registries of other folders.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{LAMP, invalid, put, record, repo, sha256, third_party, written};
use orbe_registry::{Error, INDEX_SCHEMA, ITEM_SCHEMA, Item, LARGE, Why, build};
use serde_json::Value;

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
    let registry = build(&repo(), None).unwrap();
    let bless = std::env::var_os("ORBE_BLESS").is_some();
    for address in [
        "led-5mm@0.1.0",
        "atmega328p-dip28@0.1.0",
        "nano-board@0.1.0",
        "nano-button-led@0.1.0",
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

#[test]
fn two_builds_write_the_same_bytes() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    build(&repo(), None).unwrap().write(first.path()).unwrap();
    build(&repo(), None).unwrap().write(second.path()).unwrap();
    let a = written(first.path());
    let b = written(second.path());
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    for (path, bytes) in &a {
        assert!(&b[path] == bytes, "{path} differs between two builds");
    }
    assert_eq!(a.len(), build(&repo(), None).unwrap().report.files);
}

/// Every row's record and every record's file is in the output, under the URL the row or
/// record gives, with the SHA-256 and size it gives.
#[test]
fn every_url_holds_the_bytes_its_hash_names() {
    let registry = build(&repo(), None).unwrap();
    let index: Value = serde_json::from_slice(&registry.output["index.json"]).unwrap();
    assert_eq!(index["$schema"], INDEX_SCHEMA);
    assert_eq!(index["format"], 1);
    assert_eq!(index["name"], orbe_scene::REGISTRY);
    assert_eq!(index["homepage"], "https://orbefab.com/embedded");
    let rows = index["items"].as_array().unwrap();
    assert_eq!(rows.len(), registry.items.len());
    for row in rows {
        let url = row["url"].as_str().unwrap();
        assert_eq!(
            url,
            format!(
                "{}@{}.json",
                row["name"].as_str().unwrap(),
                row["version"].as_str().unwrap()
            )
        );
        let bytes = &registry.output[url];
        assert_eq!(row["sha256"], sha256(bytes), "{url}");
        assert_eq!(row["bytes"], bytes.len(), "{url}");
        let record: Value = serde_json::from_slice(bytes).unwrap();
        for key in ["name", "type", "version"] {
            assert_eq!(record[key], row[key], "{url} {key}");
        }
        assert_eq!(record["$schema"], ITEM_SCHEMA, "{url}");
        assert_eq!(record["format"], 1, "{url}");
        let mut listed = String::new();
        for file in record["files"].as_array().unwrap() {
            let sha = file["sha256"].as_str().unwrap();
            assert_eq!(file["url"], format!("files/{sha}"));
            let stored = &registry.output[&format!("files/{sha}")];
            assert_eq!(sha256(stored), sha, "{url} {}", file["path"]);
            assert_eq!(file["bytes"], stored.len(), "{url} {}", file["path"]);
            assert!(!file["license"].as_str().unwrap().is_empty());
            listed.push_str(&format!("{sha}  {}\n", file["path"].as_str().unwrap()));
        }
        assert_eq!(
            record["hash"],
            format!("sha256:{}", sha256(listed.as_bytes())),
            "{url}: the folder hash is over the files it lists"
        );
    }
    let block = record(&registry.output, "nano-button-led@0.1.0");
    assert_eq!(
        block["license"],
        "(MIT OR Apache-2.0) AND LGPL-2.1-or-later"
    );
    let licence = |path: &str| {
        block["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == path)
            .map(|file| file["license"].clone())
    };
    assert_eq!(licence("scene.jsonc"), Some("MIT OR Apache-2.0".into()));
    assert_eq!(
        licence("firmware/button.hex"),
        Some("LGPL-2.1-or-later".into())
    );
    assert_eq!(
        licence("firmware/LICENSE-libc"),
        Some("BSD-3-Clause".into())
    );
}

#[test]
fn the_repo_names_the_official_registry() {
    let registry = build(&repo(), None).unwrap();
    let index: Value = serde_json::from_slice(&registry.output["index.json"]).unwrap();
    assert_eq!(index["name"], "@orbefab");
    assert_eq!(index["name"], orbe_scene::REGISTRY);
}

#[test]
fn a_block_depends_on_every_part_it_runs_at_the_versions_in_the_same_build() {
    let registry = build(&repo(), None).unwrap();
    let block = record(&registry.output, "nano-button-led@0.1.0");
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
    assert_eq!(block["version"], "0.1.0");
}

#[test]
fn the_item_schema_has_the_part_and_block_types() {
    let registry = build(&repo(), None).unwrap();
    let schema: Value = serde_json::from_slice(&registry.output[ITEM_SCHEMA]).unwrap();
    let types: Vec<&Value> = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variant| &variant["properties"]["type"]["const"])
        .collect();
    assert_eq!(types, ["part", "block"]);
}

#[test]
fn another_folder_of_parts_and_blocks_builds_and_counts_what_it_lacks() {
    let src = third_party();
    let registry = build(src.path(), None).unwrap();
    let names: Vec<String> = registry.items.iter().map(Item::address).collect();
    assert_eq!(names, ["lamp@1.2.0", "lamp-demo@0.1.0"]);
    let index: Value = serde_json::from_slice(&registry.output["index.json"]).unwrap();
    assert_eq!(index["name"], "@acme");
    assert!(index.get("homepage").is_none());
    let block = record(&registry.output, "lamp-demo@0.1.0");
    assert_eq!(
        block["registryDependencies"],
        serde_json::json!(["lamp@1.2.0"])
    );
    let lamp = record(&registry.output, "lamp@1.2.0");
    for absent in ["description", "categories", "docs", "buy", "usedBy"] {
        assert!(lamp.get(absent).is_none(), "{absent}");
    }
    assert_eq!(lamp["license"], "MIT");

    let report = &registry.report;
    assert_eq!((report.parts, report.blocks), (1, 1));
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
        report.warnings(),
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
    let siblings: Vec<String> = fs::read_dir(src.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        siblings.iter().all(|name| !name.contains("partial")),
        "the staging folder is gone: {siblings:?}"
    );
    let empty = src.path().join("empty");
    fs::create_dir(&empty).unwrap();
    registry.write(&empty).unwrap();
    assert_eq!(
        written(&empty),
        written(&out),
        "an empty folder takes a registry"
    );
}

#[test]
fn build_output_in_a_part_folder_is_not_in_the_registry() {
    let src = third_party();
    let bytecode = "bytecode";
    let cached = "cached";
    put(src.path(), "parts/lamp/a.pyc", bytecode);
    put(
        src.path(),
        "parts/lamp/__pycache__/lamp.cpython-312.pyc",
        cached,
    );
    put(src.path(), "parts/lamp/notes.txt", "an author file");
    let registry = build(src.path(), None).unwrap();
    let lamp = record(&registry.output, "lamp@1.2.0");
    let paths: Vec<&str> = lamp["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["notes.txt", "part.jsonc"]);
    assert!(
        !registry
            .output
            .contains_key(&format!("files/{}", sha256(bytecode.as_bytes())))
    );
    assert!(
        !registry
            .output
            .contains_key(&format!("files/{}", sha256(cached.as_bytes())))
    );
}

#[test]
fn a_hidden_or_large_file_is_named_in_the_warnings() {
    let src = third_party();
    put(src.path(), "parts/lamp/.env", "TOKEN=secret");
    put(
        src.path(),
        "parts/lamp/model.glb",
        &"x".repeat(LARGE as usize + 1),
    );
    let registry = build(src.path(), None).unwrap();
    let notices: Vec<(&str, &str, Why)> = registry
        .report
        .notices
        .iter()
        .map(|n| (n.item.as_str(), n.path.as_str(), n.why))
        .collect();
    assert_eq!(
        notices,
        [
            ("lamp@1.2.0", ".env", Why::Hidden),
            ("lamp@1.2.0", "model.glb", Why::Large)
        ]
    );
    let warnings = registry.report.warnings();
    assert!(
        warnings.contains(&"lamp@1.2.0 publishes the hidden file `.env`, since a file in an item's folder is published unless it is build output".to_string()),
        "{warnings:?}"
    );
    assert!(
        warnings
            .contains(&"lamp@1.2.0 publishes `model.glb`, 1000001 bytes, over 1 MB".to_string()),
        "{warnings:?}"
    );
}

#[test]
fn two_parts_with_one_files_bytes_store_it_once() {
    let src = tempfile::tempdir().unwrap();
    put(src.path(), "registry.jsonc", common::MANIFEST);
    let glb = "binary model bytes";
    for id in ["lamp", "lamp-red"] {
        put(
            src.path(),
            &format!("parts/{id}/part.jsonc"),
            &LAMP.replace("\"lamp\"", &format!("\"{id}\"")),
        );
        put(src.path(), &format!("parts/{id}/lamp.glb"), glb);
    }
    let registry = build(src.path(), None).unwrap();
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

#[test]
fn a_block_whose_parts_are_not_in_the_registry_does_not_build() {
    let src = third_party();
    put(
        src.path(),
        "blocks/lamp-demo/scene.jsonc",
        r#"{ "format": 1, "parts": { "l1": { "type": "lamp" }, "r1": { "type": "resistor-axial" } } }"#,
    );
    let message = invalid(src.path());
    assert!(
        message.contains("`resistor-axial`, which this registry lacks"),
        "{message}"
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
        r#"{ "format": 1, "part": "lamp-pair", "version": "0.1.0", "title": "Two lamps", "license": "MIT", "uses": { "lamp": "1.0.0" }, "parts": { "a": { "type": "lamp" } } }"#,
    );
    assert!(invalid(src.path()).contains("uses `lamp@1.0.0`, which these parts lack"));
}

/// Each probe is a source a registry must refuse, with what its message names.
#[test]
fn the_build_refuses_what_a_client_could_not_trust() {
    type Change = dyn Fn(&std::path::Path);
    let probes: [(&str, &Change, &str); 14] = [
        (
            "a version with a sign and a leading zero",
            &|src| {
                put(
                    src,
                    "parts/lamp/part.jsonc",
                    &LAMP.replace("1.2.0", "+01.2.3"),
                )
            },
            "no version of the form `0.1.0`",
        ),
        (
            "a Block version with a leading zero",
            &|src| {
                put(
                    src,
                    "blocks/lamp-demo/block.jsonc",
                    r#"{ "format": 1, "block": "lamp-demo", "version": "0.01.0", "title": "A lamp", "board": "lamp", "license": "MIT" }"#,
                )
            },
            "no version of the form `0.1.0`",
        ),
        (
            "a part with no licence",
            &|src| {
                put(
                    src,
                    "parts/lamp/part.jsonc",
                    &LAMP.replace("  \"license\": \"MIT\",\n", ""),
                )
            },
            "`lamp` part.jsonc has no `license`",
        ),
        (
            "a part that names a path",
            &|src| {
                put(
                    src,
                    "parts/lamp/part.jsonc",
                    &LAMP.replace("\"part\": \"lamp\"", "\"part\": \"../../evil\""),
                )
            },
            "names `../../evil`",
        ),
        (
            "a part named index",
            &|src| {
                put(
                    src,
                    "parts/index/part.jsonc",
                    &LAMP.replace("\"lamp\"", "\"index\""),
                )
            },
            "`index` is reserved",
        ),
        (
            "a Block named con",
            &|src| put(src, "blocks/con/block.jsonc", "{}"),
            "`con` is reserved",
        ),
        (
            "a folder that is not an id",
            &|src| put(src, "parts/Lamp/part.jsonc", LAMP),
            "parts/Lamp is not an id",
        ),
        (
            "a file name with a colon",
            &|src| put(src, "parts/lamp/a:b.txt", ""),
            "parts/lamp/a:b.txt has a backslash",
        ),
        (
            "a file name with a backslash",
            &|src| put(src, "parts/lamp/a\\b.txt", ""),
            "has a backslash",
        ),
        (
            "a file named for a Windows device",
            &|src| put(src, "parts/lamp/firmware/aux.c", ""),
            "parts/lamp/firmware/aux.c has a name Windows keeps for a device",
        ),
        (
            "a file name ending in a dot",
            &|src| put(src, "parts/lamp/notes.", ""),
            "ends in a dot or a space",
        ),
        (
            "two files one case apart",
            &|src| {
                put(src, "parts/lamp/README.md", "");
                put(src, "parts/lamp/readme.md", "");
            },
            "which are one file on a disk that ignores case",
        ),
        (
            "no manifest",
            &|src| fs::remove_file(src.join("registry.jsonc")).unwrap(),
            "has no registry.jsonc",
        ),
        (
            "a manifest whose name is not @<id>",
            &|src| put(src, "registry.jsonc", r#"{ "format": 1, "name": "acme" }"#),
            "a registry's name is `@` and an id",
        ),
    ];
    for (what, change, says) in probes {
        let src = third_party();
        change(src.path());
        let message = invalid(src.path());
        assert!(message.contains(says), "{what}: {message}");
    }
}

#[test]
fn every_item_is_a_part_or_a_block_of_the_repo() {
    let registry = build(&repo(), None).unwrap();
    let parts = fs::read_dir(repo().join("parts")).unwrap().count();
    let blocks = fs::read_dir(repo().join("blocks")).unwrap().count();
    let (mut p, mut b) = (0, 0);
    for item in &registry.items {
        match item {
            Item::Part(_) => p += 1,
            Item::Block(_) => b += 1,
        }
    }
    assert_eq!((p, b), (parts, blocks));
}
