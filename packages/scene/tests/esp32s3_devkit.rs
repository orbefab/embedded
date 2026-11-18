//! The DevKitC-1 catalog board, placed with the esp32s3-hello app and partition table.
//! The chip's ROM is the part's default file. The reader's `rom` and `firmware` match
//! `packages/engine/tests/circuits/esp32s3-hello/netlist.json`.

mod common;

use std::fs;

use serde_json::Value;

#[test]
fn the_devkit_reader_matches_the_hello_rom_and_firmware() {
    let project = common::project("esp32s3-devkitc");
    let netlist = orbe_scene::read(
        &orbe_scene::Files::disk(),
        &project.path().join("scene.jsonc"),
    )
    .unwrap_or_else(|found| panic!("{found:#?}"));
    let read: Value = serde_json::from_str(&netlist.json).unwrap();
    let example: Value = serde_json::from_str(
        &fs::read_to_string(
            common::repo().join("packages/engine/tests/circuits/esp32s3-hello/netlist.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let chips: Vec<&Value> = read["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["kind"] == "esp32s3")
        .collect();
    assert_eq!(chips.len(), 1, "{chips:?}");
    let expected = example["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["kind"] == "esp32s3")
        .unwrap();
    assert_eq!(chips[0]["rom"], expected["rom"]);
    assert_eq!(chips[0]["firmware"], expected["firmware"]);
}
