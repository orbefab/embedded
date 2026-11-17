//! The folder hash of a fixture that is not a catalog part, so a part change does not touch it.
//!
//! `__pycache__/a.pyc` is in the fixture. The hash leaves that build output out and covers
//! `note.txt` and the fixture's `.gitignore`. It does not read that `.gitignore`. The digest
//! below is that set. A change to the hash function changes the digest.

use std::fs;
use std::path::PathBuf;

#[test]
fn the_fixture_folder_hashes_to_the_recorded_digest() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/folder-hash");
    assert_eq!(
        orbe_scene::folder_hash(&dir).unwrap(),
        "sha256:979ea1c8b9d286a5a5c6699b7ba0d890068f829e31f64e222598ae67c15503ab"
    );
}

/// `__pycache__` and a `.pyc` beside `part.jsonc` are build output. An author file is not.
#[test]
fn bytecode_does_not_change_the_folder_hash_and_an_author_file_does() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("part.jsonc"), "{}\n").unwrap();
    let published = orbe_scene::folder_hash(root).unwrap();
    fs::write(root.join("a.pyc"), [0, 1, 2]).unwrap();
    fs::create_dir(root.join("__pycache__")).unwrap();
    fs::write(
        root.join("__pycache__").join("part.cpython-312.pyc"),
        [3, 4],
    )
    .unwrap();
    fs::create_dir(root.join(".pytest_cache")).unwrap();
    fs::write(root.join(".pytest_cache").join("v"), b"cache").unwrap();
    fs::create_dir(root.join(".venv")).unwrap();
    fs::write(root.join(".venv").join("pyvenv.cfg"), b"home = /tmp\n").unwrap();
    assert_eq!(orbe_scene::folder_hash(root).unwrap(), published);
    fs::write(root.join("notes.txt"), b"kept\n").unwrap();
    assert_ne!(orbe_scene::folder_hash(root).unwrap(), published);
}
