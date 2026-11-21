//! A `ws2812`'s `DOUT` is part format 3. The catalog's SK68XXMINI-HS wires it, so two of them in
//! a row pass the bits on. An older part whose `DOUT` pin shares the name stays off the chain,
//! as it did before the engine had the pin, and an older part that maps `DOUT` or reads
//! `"of": "rgb"` is told to move to format 3.

mod common;

use std::fs;
use std::path::Path;

use orbe_scene::{Code, Diagnostic, Files};
use serde_json::Value;
use tempfile::TempDir;

const SUPPLY: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "params": { "voltage": { "unit": "V", "required": true, "sets": "/voltage" } },
  "model": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } }
}
"#;

const SCENE: &str = r#"{
  "format": 1,
  "parts": {
    "v1": { "type": "./supply/part.jsonc", "voltage": "5" },
    "led1": { "type": "./led/part.jsonc" },
    "led2": { "type": "./led/part.jsonc" },
  },
  "wires": [
    ["v1.pos", "led1.VDD"], ["v1.neg", "led1.GND"],
    ["v1.pos", "led2.VDD"], ["v1.neg", "led2.GND"],
    ["v1.pos", "led1.DIN"],
    ["led1.DOUT", "led2.DIN"],
  ],
  "ground": "v1.neg",
}
"#;

/// The catalog part's file, edited by `edit`, as `./led/part.jsonc` of a scene with two of it.
fn project(edit: impl FnOnce(String) -> String) -> TempDir {
    project_with(edit, SCENE)
}

fn project_with(edit: impl FnOnce(String) -> String, scene: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let part = fs::read_to_string(common::repo().join("parts/sk68xxmini-hs/part.jsonc")).unwrap();
    for (file, text) in [
        ("supply/part.jsonc", SUPPLY.to_string()),
        ("led/part.jsonc", edit(part)),
        ("scene.jsonc", scene.to_string()),
    ] {
        let path = dir.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

fn read(project: &Path) -> Result<Value, Vec<Diagnostic>> {
    let netlist = orbe_scene::read(&Files::disk(), &project.join("scene.jsonc"))?;
    Ok(serde_json::from_str(&netlist.json).unwrap())
}

fn chain<'v>(netlist: &'v Value, name: &str) -> &'v Value {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == name)
        .unwrap_or_else(|| panic!("no part {name}"))
}

/// The part file in format `format` with its `dout` and its reading taken out, as 0.1.1 was.
fn before_dout(text: String, format: u32) -> String {
    let mut text = text.replacen("\"format\": 3,", &format!("\"format\": {format},"), 1);
    let start = text.find("    \"dout\": {").unwrap();
    let end = text.find("  \"readings\"").unwrap();
    text.replace_range(start..end, "  },\n");
    let start = text.find("  \"readings\"").unwrap();
    let end = text.find("  \"not_simulated\"").unwrap();
    text.replace_range(start..end, "");
    text.replace(
        "\"DIN\": \"DIN\", \"DOUT\": \"DOUT\" }",
        "\"DIN\": \"DIN\" }",
    )
    .replacen(
        "\"min\"\n          }\n        }\n      }\n  },",
        "\"min\"\n          }\n        }\n      }\n    }\n  },",
        1,
    )
}

#[test]
fn the_catalog_part_passes_its_bits_on_through_dout() {
    let project = project(|text| text);
    let netlist = read(project.path()).unwrap();
    let (led1, led2) = (chain(&netlist, "led1"), chain(&netlist, "led2"));
    assert_eq!(led1["pins"]["DOUT"], led2["pins"]["DIN"]);
    assert_eq!(led1["dout"]["delay"]["value"], 500000);
    assert_eq!(led1["dout"]["delay"]["unit"], "ps");
}

#[test]
fn a_part_before_format_3_keeps_its_dout_pin_off_the_chain() {
    let scene = SCENE.replace("    [\"led1.DOUT\", \"led2.DIN\"],\n", "");
    for format in [1, 2] {
        let project = project_with(|text| before_dout(text, format), &scene);
        let netlist = read(project.path()).unwrap_or_else(|e| panic!("format {format}: {e:#?}"));
        let led1 = chain(&netlist, "led1");
        assert_eq!(led1["pins"].get("DOUT"), None, "format {format}");
        assert_eq!(led1.get("dout"), None, "format {format}");
    }
}

#[test]
fn a_part_before_format_3_that_maps_dout_is_told_to_move_to_format_3() {
    let project = project(|text| text.replacen("\"format\": 3,", "\"format\": 2,", 1));
    let found = read(project.path()).unwrap_err();
    let found: Vec<(Code, &str, &str)> = found
        .iter()
        .map(|d| (d.code, d.message.as_str(), d.hint.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            (
                Code::UnknownPin,
                "the engine's `ws2812` takes `DOUT` only in part format 3; this file is format 2",
                "set \"format\": 3 in this part file"
            ),
            (
                Code::Syntax,
                "reading `color` takes \"of\": \"rgb\" only in part format 3; this file is format 2",
                "set \"format\": 3 in this part file"
            ),
        ]
    );
}

#[test]
fn a_colour_reading_on_a_part_that_is_no_led_chain_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let supply = SUPPLY
        .replacen("\"format\": 1,", "\"format\": 3,", 1)
        .replacen(
            "\"model\":",
            "\"readings\": { \"color\": { \"of\": \"rgb\" } },\n  \"model\":",
            1,
        );
    let scene = r#"{
  "format": 1,
  "parts": { "v1": { "type": "./supply/part.jsonc", "voltage": "5" } },
  "ground": "v1.neg",
}
"#;
    for (file, text) in [
        ("supply/part.jsonc", supply.as_str()),
        ("scene.jsonc", scene),
    ] {
        let path = dir.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let found = read(dir.path()).unwrap_err();
    let found: Vec<(Code, &str)> = found.iter().map(|d| (d.code, d.message.as_str())).collect();
    assert_eq!(
        found,
        [(
            Code::Syntax,
            "reading `color` takes \"of\": \"rgb\" only on a part whose model is `ws2812`"
        )]
    );
}
