//! The folder hash of a fixture that is not a catalog part, so a part change does not touch it.
//!
//! `__pycache__/a.pyc` is in the fixture, and both the repo's `.gitignore` and the fixture's
//! own ignore that directory. `folder_hash` reads every file and does not look at `.gitignore`,
//! so the digest below covers the `.pyc`. A change to the hash function changes the digest.

use std::path::PathBuf;

#[test]
fn the_fixture_folder_hashes_to_the_recorded_digest() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/folder-hash");
    assert_eq!(
        orbe_scene::folder_hash(&dir).unwrap(),
        "sha256:36a4d4a3245b7e153b87cf32b8a60526e75ab34377d9f0a6029ddd47ad7f83f8"
    );
}
