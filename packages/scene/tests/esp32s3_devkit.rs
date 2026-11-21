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

/// The bridge's serial source drives the chip's U0RXD, GPIO44, and nothing drives GPIO43,
/// U0TXD. A crossed pair would leave the serial source on GPIO43.
#[test]
fn the_devkit_bridge_drives_u0rxd() {
    let project = common::project("esp32s3-devkitc");
    let netlist = orbe_scene::read(
        &orbe_scene::Files::disk(),
        &project.path().join("scene.jsonc"),
    )
    .unwrap_or_else(|found| panic!("{found:#?}"));
    let read: Value = serde_json::from_str(&netlist.json).unwrap();
    let parts = read["parts"].as_array().unwrap();
    let chip = parts.iter().find(|part| part["kind"] == "esp32s3").unwrap();
    let rx = chip["pins"]["GPIO44"].as_str().unwrap();
    let tx = chip["pins"]["GPIO43"].as_str().unwrap();
    let serial: Vec<&Value> = parts
        .iter()
        .filter(|part| part["kind"] == "voltage_source" && part.to_string().contains("\"serial\""))
        .collect();
    assert_eq!(serial.len(), 1, "{serial:?}");
    assert_eq!(serial[0]["pins"]["p"], rx, "{serial:?}");
    assert_ne!(rx, tx);
}
