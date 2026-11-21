//! A Block reads a catalog part's default file from the catalog. The part folder is not in the
//! Block: `Files::memory` holds the Block, and `with_catalog` holds the parts. One part defaults
//! a text file, the AVR hex, and one defaults a blob.

mod common;

use std::fs;
use std::path::Path;

use orbe_engine::blob::Sha256;
use orbe_scene::{Code, Files};
use serde_json::Value;

fn fixture() -> std::path::PathBuf {
    common::fixture("catalog-defaults")
}

/// The Block's own files, and each catalog part's files by `<id>/<path>`.
fn files() -> Files {
    let root = fixture();
    let own = ["block.jsonc", "scene.jsonc"].map(|name| (name, fs::read(root.join(name)).unwrap()));
    let mut catalog = Vec::new();
    for entry in fs::read_dir(root.join("catalog")).unwrap() {
        let dir = entry.unwrap().path();
        let id = dir.file_name().unwrap().to_str().unwrap().to_string();
        for file in fs::read_dir(&dir).unwrap() {
            let path = file.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            catalog.push((format!("{id}/{name}"), fs::read(path).unwrap()));
        }
    }
    Files::memory(own).with_catalog(catalog)
}

fn read() -> Result<orbe_scene::Netlist, Vec<orbe_scene::Diagnostic>> {
    orbe_scene::read(&files(), Path::new("scene.jsonc"))
}

fn part<'v>(netlist: &'v Value, name: &str) -> &'v Value {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == name)
        .unwrap_or_else(|| panic!("no part {name}"))
}

#[test]
fn a_block_reads_a_text_default_from_the_catalog() {
    let netlist = read().unwrap_or_else(|found| panic!("{found:#?}"));
    let read: Value = serde_json::from_str(&netlist.json).unwrap();
    let hex = fs::read_to_string(fixture().join("catalog/text-chip/program.hex")).unwrap();
    assert_eq!(part(&read, "mcu")["firmware"], hex);
}

#[test]
fn a_block_reads_a_blob_default_from_the_catalog() {
    let netlist = read().unwrap_or_else(|found| panic!("{found:#?}"));
    let read: Value = serde_json::from_str(&netlist.json).unwrap();
    let elf = fs::read(fixture().join("catalog/blob-chip/image.elf")).unwrap();
    let sha = Sha256::of(&elf);
    let blob = &part(&read, "s3")["firmware"]["elf"];
    assert_eq!(blob["sha256"], sha.to_string());
    assert_eq!(blob["bytes"], elf.len());
    let kept = netlist
        .blobs
        .get(&sha)
        .unwrap_or_else(|| panic!("the elf is not kept"));
    assert_eq!(&kept[..], &elf[..]);
}

#[test]
fn a_catalog_default_that_is_not_in_the_part_is_firmware_missing() {
    let root = fixture();
    let own = ["block.jsonc", "scene.jsonc"].map(|name| (name, fs::read(root.join(name)).unwrap()));
    let mut catalog = Vec::new();
    for entry in fs::read_dir(root.join("catalog")).unwrap() {
        let dir = entry.unwrap().path();
        let id = dir.file_name().unwrap().to_str().unwrap().to_string();
        for file in fs::read_dir(&dir).unwrap() {
            let path = file.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if id == "text-chip" && name == "program.hex" {
                continue;
            }
            catalog.push((format!("{id}/{name}"), fs::read(path).unwrap()));
        }
    }
    let files = Files::memory(own).with_catalog(catalog);
    let found = match orbe_scene::read(&files, Path::new("scene.jsonc")) {
        Err(found) => found,
        Ok(_) => panic!("the scene read with no program.hex in the catalog"),
    };
    let missing = found
        .iter()
        .find(|d| d.code == Code::FirmwareMissing && d.message.contains("program.hex"))
        .unwrap_or_else(|| panic!("{found:#?}"));
    assert!(
        missing.message.contains("no file at `program.hex`"),
        "{missing}"
    );
}
