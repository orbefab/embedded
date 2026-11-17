//! `released.lock`: a part or Block folder does not change at a version the file records.

mod common;

use std::fs;

use common::{put, repo, third_party};
use orbe_registry::{RELEASED, released_changes, released_lock};
use orbe_scene::folder_hash;

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
fn the_lock_uses_the_folder_hash_parts_lock_records() {
    let src = third_party();
    let text = lock(src.path());
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    let part = folder_hash(&src.path().join("parts/lamp")).unwrap();
    let block = folder_hash(&src.path().join("blocks/lamp-demo")).unwrap();
    assert_eq!(lines[0], format!("lamp 1.2.0 {part}"));
    assert_eq!(lines[1], format!("lamp-demo 0.1.0 {block}"));
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
        vec![
            "lamp 1.2.0 differs from the hash released.lock records. Bump its version in parts/lamp/part.jsonc"
                .to_string()
        ]
    );
}

#[test]
fn a_bumped_part_passes() {
    let src = third_party();
    lock(src.path());
    let text = fs::read_to_string(src.path().join("parts/lamp/part.jsonc")).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &text.replace("1.2.0", "1.2.1").replace("Lamp", "Lamps"),
    );
    assert!(released_changes(src.path()).unwrap().is_empty());
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
    assert!(released_changes(src.path()).unwrap().is_empty());
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
        vec![
            "lamp-demo 0.1.0 differs from the hash released.lock records. Bump its version in blocks/lamp-demo/block.jsonc"
                .to_string()
        ]
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
