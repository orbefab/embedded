//! `released.lock`: a served record does not change at a version the file records.

mod common;

use std::fs;

use common::{put, repo, third_party};
use orbe_registry::{Error, RELEASED, build, released_changes, released_lock};
use orbe_scene::folder_hash;

const CAP: &str = r#"{
  "format": 1,
  "part": "cap",
  "version": "0.1.0",
  "title": "Cap",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "model": { "kind": "resistor" },
}
"#;

fn differs(id: &str, version: &str, file: &str) -> String {
    format!(
        "`{id}@{version}` builds to a record other than the one released.lock records. A served version never changes, so bump its version in {file}"
    )
}

fn lock(dir: &std::path::Path) -> String {
    let text = released_lock(dir).unwrap();
    fs::write(dir.join(RELEASED), &text).unwrap();
    text
}

#[test]
fn the_repo_lock_matches_its_parts_and_blocks() {
    let changes = released_changes(&repo()).unwrap();
    assert!(
        changes.is_empty(),
        "released.lock disagrees with parts/ or blocks/:\n{}",
        changes.join("\n")
    );
}

#[test]
fn the_lock_pins_the_record_the_build_writes() {
    let src = third_party();
    let text = lock(src.path());
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    let registry = build(src.path(), None).unwrap();
    let index: serde_json::Value = serde_json::from_slice(&registry.output["index.json"]).unwrap();
    let digest = |name: &str| {
        index["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap()["sha256"]
            .as_str()
            .unwrap()
    };
    assert_eq!(lines[0], format!("lamp 1.2.0 sha256:{}", digest("lamp")));
    assert_eq!(
        lines[1],
        format!("lamp-demo 0.1.0 sha256:{}", digest("lamp-demo"))
    );
}

#[test]
fn an_edited_part_at_the_same_version_names_the_part_and_says_to_bump() {
    let src = third_party();
    lock(src.path());
    let text = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &text.replace("Lamp", "Lamps"),
    );
    assert_eq!(
        released_changes(src.path()).unwrap(),
        vec![differs("lamp", "1.2.0", "parts/lamp/part.jsonc")]
    );
}

#[test]
fn a_part_bump_changes_the_block_record_at_the_same_block_version() {
    let src = third_party();
    lock(src.path());
    let served = src.path().join("served");
    build(src.path(), None).unwrap().write(&served).unwrap();
    let folder = folder_hash(&src.path().join("blocks/lamp-demo")).unwrap();
    let text = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &text.replace("1.2.0", "1.2.1"),
    );
    assert_eq!(
        folder_hash(&src.path().join("blocks/lamp-demo")).unwrap(),
        folder,
        "the Block folder is unchanged"
    );
    assert_eq!(
        released_changes(src.path()).unwrap(),
        vec![differs(
            "lamp-demo",
            "0.1.0",
            "blocks/lamp-demo/block.jsonc"
        )]
    );
    let Error::Invalid(message) = build(src.path(), Some(&served)).err().unwrap() else {
        panic!("--previous accepted the build");
    };
    assert!(
        message.contains(
            "`lamp-demo@0.1.0` builds to a record other than the one served at that version"
        ),
        "{message}"
    );
    assert!(
        message.contains("give the change a new version"),
        "{message}"
    );
}

/// `0.10.0` is newer than `0.9.0` by number, as a registry orders versions.
#[test]
fn a_part_at_a_newer_version_passes() {
    let src = third_party();
    let text = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &text.replace("1.2.0", "0.9.0"),
    );
    lock(src.path());
    let bumped = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &bumped.replace("0.9.0", "0.10.0").replace("Lamp", "Lamps"),
    );
    let block = fs::read_to_string(src.path().join("blocks/lamp-demo/block.jsonc")).unwrap();
    put(
        src.path(),
        "blocks/lamp-demo/block.jsonc",
        &block.replace("\"version\": \"0.1.0\"", "\"version\": \"0.2.0\""),
    );
    assert!(
        released_changes(src.path()).unwrap().is_empty(),
        "0.10.0 is newer than 0.9.0, and the Block takes a new version with its part"
    );
}

#[test]
fn an_edited_block_at_the_same_version_names_the_block_and_says_to_bump() {
    let src = third_party();
    lock(src.path());
    let text = fs::read_to_string(src.path().join("blocks/lamp-demo/block.jsonc")).unwrap();
    put(
        src.path(),
        "blocks/lamp-demo/block.jsonc",
        &text.replace("A lamp", "A lamps"),
    );
    assert_eq!(
        released_changes(src.path()).unwrap(),
        vec![differs(
            "lamp-demo",
            "0.1.0",
            "blocks/lamp-demo/block.jsonc"
        )]
    );
}

#[test]
fn a_bumped_block_passes() {
    let src = third_party();
    lock(src.path());
    let text = fs::read_to_string(src.path().join("blocks/lamp-demo/block.jsonc")).unwrap();
    put(
        src.path(),
        "blocks/lamp-demo/block.jsonc",
        &text
            .replace("\"version\": \"0.1.0\"", "\"version\": \"0.2.0\"")
            .replace("A lamp", "A lamps"),
    );
    assert!(released_changes(src.path()).unwrap().is_empty());
}

#[test]
fn an_item_with_no_line_passes() {
    let src = third_party();
    let text = lock(src.path());
    let kept: String = text
        .lines()
        .filter(|line| line.starts_with("lamp-demo "))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(src.path().join(RELEASED), kept).unwrap();
    let part = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &part.replace("Lamp", "Lamps"),
    );
    assert!(released_changes(src.path()).unwrap().is_empty());
}

#[test]
fn an_empty_lock_passes() {
    let src = third_party();
    fs::write(src.path().join(RELEASED), "").unwrap();
    let part = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &part.replace("Lamp", "Lamps"),
    );
    assert!(released_changes(src.path()).unwrap().is_empty());
}

#[test]
fn a_missing_lock_is_an_error() {
    let src = third_party();
    let error = released_changes(src.path()).unwrap_err().to_string();
    assert!(
        error.contains("released.lock") && error.contains("scripts/released.sh"),
        "{error}"
    );
}

#[test]
fn released_lock_keeps_the_recorded_hash_at_the_same_version() {
    let src = third_party();
    let first = lock(src.path());
    let text = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &text.replace("Lamp", "Lamps"),
    );
    assert_eq!(released_lock(src.path()).unwrap(), first);
}

#[test]
fn released_lock_refuses_to_move_a_version_backwards() {
    let src = third_party();
    lock(src.path());
    let text = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &text.replace("1.2.0", "1.1.0"),
    );
    let error = released_lock(src.path()).unwrap_err().to_string();
    assert!(
        error.contains("lamp is 1.1.0") && error.contains("does not go backwards"),
        "{error}"
    );
}

fn removed_then_restored(
    src: &std::path::Path,
    id: &str,
    folder: &str,
    file: &str,
    from: &str,
    to: &str,
) {
    let served = src.join("served");
    build(src, None).unwrap().write(&served).unwrap();
    let first = fs::read_to_string(src.join(RELEASED)).unwrap();
    let line = first
        .lines()
        .find(|line| line.starts_with(&format!("{id} ")))
        .unwrap()
        .to_string();
    let root = src.join(folder);
    let mut saved = Vec::new();
    fn walk(root: &std::path::Path, dir: &std::path::Path, saved: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, saved);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy();
                saved.push((rel.replace('\\', "/"), fs::read(&path).unwrap()));
            }
        }
    }
    walk(&root, &root, &mut saved);
    fs::remove_dir_all(&root).unwrap();
    assert!(released_changes(src).unwrap().is_empty());
    let carried = build(src, Some(&served)).unwrap();
    assert_eq!(carried.report.carried, 1, "{id} is carried, not rebuilt");
    let kept = released_lock(src).unwrap();
    assert!(
        kept.lines().any(|kept_line| kept_line == line),
        "{id} line dropped:\n{kept}"
    );
    fs::write(src.join(RELEASED), &kept).unwrap();
    for (rel, bytes) in &saved {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
    }
    let text = fs::read_to_string(src.join(file)).unwrap();
    assert!(text.contains(from), "{file} has no `{from}`");
    put(src, file, &text.replace(from, to));
    assert_eq!(
        released_changes(src).unwrap(),
        vec![differs(id, "0.1.0", file)]
    );
    let Error::Invalid(message) = build(src, Some(&served)).err().unwrap() else {
        panic!("--previous accepted the restored {id}");
    };
    assert!(
        message.contains(&format!(
            "`{id}@0.1.0` builds to a record other than the one served at that version"
        )),
        "{message}"
    );
    assert!(
        message.contains("give the change a new version"),
        "{message}"
    );
}

#[test]
fn a_removed_part_keeps_its_line_and_a_changed_restore_fails() {
    let src = third_party();
    put(src.path(), "parts/cap/part.jsonc", CAP);
    lock(src.path());
    removed_then_restored(
        src.path(),
        "cap",
        "parts/cap",
        "parts/cap/part.jsonc",
        "\"Cap\"",
        "\"Caps\"",
    );
}

#[test]
fn a_removed_block_keeps_its_line_and_a_changed_restore_fails() {
    let src = third_party();
    lock(src.path());
    removed_then_restored(
        src.path(),
        "lamp-demo",
        "blocks/lamp-demo",
        "blocks/lamp-demo/block.jsonc",
        "A lamp",
        "A planted lamp",
    );
}
