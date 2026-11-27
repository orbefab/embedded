//! Format 5 carries a rating's `basis` and the model keys that came with it, which the
//! published 0.2.0 and 0.2.1 binaries refuse. A part that uses one of them before format 5 is
//! told to set format 5, at each key.

mod common;

use std::collections::BTreeMap;
use std::fs;

use orbe_scene::{Code, Diagnostic};

fn findings(text: &str) -> Vec<Diagnostic> {
    let mut files = BTreeMap::new();
    files.insert("part.jsonc".to_string(), text.as_bytes().to_vec());
    orbe_scene::part_findings("sample", &files)
}

/// The catalog part `id` with its format set to `to`.
fn in_format(id: &str, from: u32, to: u32) -> String {
    let text = fs::read_to_string(common::repo().join(format!("parts/{id}/part.jsonc"))).unwrap();
    let needle = format!("\"format\": {from},");
    assert!(text.contains(&needle), "{id} is not in format {from}");
    text.replacen(&needle, &format!("\"format\": {to},"), 1)
}

#[test]
fn each_format_5_key_before_format_5_is_told_to_set_format_5() {
    for (id, from, kind, keys) in [
        (
            "diode-sod123",
            5,
            "diode",
            &[("ratings.current.basis", 5)][..],
        ),
        (
            "capacitor-electrolytic-radial",
            5,
            "capacitor",
            &[
                ("series_resistance.tan_delta", 5),
                ("series_resistance.frequency", 5),
            ],
        ),
        (
            "lm7805-to220",
            5,
            "linear_regulator",
            &[
                ("ratings.input_voltage", 5),
                ("ratings.current.basis", 5),
                ("ratings.power.basis", 5),
            ],
        ),
        (
            "lm358-dip8",
            7,
            "op_amp",
            &[("ratings", 5), ("ratings", 5), ("ratings.supply_min", 7)],
        ),
        (
            "npn-2n2222-to92",
            5,
            "npn",
            &[
                ("forward_knee_current", 5),
                ("ratings.current.basis", 5),
                ("ratings.power.basis", 5),
            ],
        ),
        (
            "sk68xxmini-hs",
            7,
            "ws2812",
            &[
                ("draw", 5),
                ("min_supply", 5),
                ("ratings", 6),
                ("ratings.supply_min", 7),
            ],
        ),
    ] {
        let found = findings(&in_format(id, from, 4));
        let mut found: Vec<(Code, String, String)> = found
            .into_iter()
            .map(|d| (d.code, d.message, d.hint))
            .collect();
        let mut expected: Vec<(Code, String, String)> = keys
            .iter()
            .map(|(key, needs)| {
                (
                    Code::Syntax,
                    format!(
                        "the engine's `{kind}` takes `{key}` only in part format {needs}; this file is format 4"
                    ),
                    format!("set \"format\": {needs} in this part file"),
                )
            })
            .collect();
        found.sort();
        expected.sort();
        assert_eq!(found, expected, "{id}");
    }
}

#[test]
fn chip_voltage_ratings_before_format_6_are_told_to_set_format_6() {
    for (id, kind) in [
        ("esp32s3-qfn56", "esp32s3"),
        ("ssd1306-oled-128x64", "ssd1306"),
        ("hd44780-1602-i2c", "hd44780_i2c"),
        ("sk68xxmini-hs", "ws2812"),
    ] {
        let found = findings(&in_format(id, 7, 5));
        let rating_finding = found.iter().find(|d| d.message.contains("takes `ratings`"));
        let finding = rating_finding.unwrap_or_else(|| panic!("{id}: {found:#?}"));
        assert_eq!(finding.code, Code::Syntax, "{id}");
        assert_eq!(
            finding.message,
            format!(
                "the engine's `{kind}` takes `ratings` only in part format 6; this file is format 5"
            )
        );
        assert_eq!(finding.hint, "set \"format\": 6 in this part file");
    }
}

#[test]
fn supply_min_ratings_before_format_7_are_told_to_set_format_7() {
    for (id, kind, key) in [
        ("atmega328p-dip28", "atmega328p", "ratings.supply_min"),
        ("esp32s3-qfn56", "esp32s3", "ratings.supply_min"),
        ("bmp280", "bmp280", "ratings.supply_min"),
        ("bmp280", "bmp280", "ratings.io_supply_min"),
        ("ssd1306-oled-128x64", "ssd1306", "ratings.supply_min"),
        ("hd44780-1602-i2c", "hd44780_i2c", "ratings.supply_min"),
        ("hc595-dip16", "hc595", "ratings.supply_min"),
        ("sk68xxmini-hs", "ws2812", "ratings.supply_min"),
        ("lm358-dip8", "op_amp", "ratings.supply_min"),
        ("dht22", "dht22", "ratings"),
        ("servo-sg90", "sg90", "ratings"),
    ] {
        let found = findings(&in_format(id, 7, 6));
        let needle = format!("takes `{key}` only in part format 7; this file is format 6");
        let finding = found
            .iter()
            .find(|d| d.message.contains(&needle))
            .unwrap_or_else(|| panic!("{id} ({key}): {found:#?}"));
        assert_eq!(finding.code, Code::Syntax, "{id}");
        assert_eq!(
            finding.message,
            format!(
                "the engine's `{kind}` takes `{key}` only in part format 7; this file is format 6"
            )
        );
        assert_eq!(finding.hint, "set \"format\": 7 in this part file");
    }
}

#[test]
fn format_7_keys_before_format_7_are_told_to_set_format_7() {
    for (id, kind, keys) in [
        (
            "thermistor-ntc-10k",
            "resistor",
            &[
                "resistance.curve.steinhart_hart",
                "ratings.power.characterized",
                "ratings.voltage",
            ][..],
        ),
        (
            "battery-9v-cell",
            "voltage_source",
            &["ratings.current.characterized"][..],
        ),
        (
            "led-5mm-rgb-cc",
            "led",
            &[
                "ratings.reverse_voltage",
                "ratings.reverse_voltage",
                "ratings.reverse_voltage",
            ][..],
        ),
        ("joystick-ky023-pot", "potentiometer", &["ratings"][..]),
    ] {
        let found = findings(&in_format(id, 7, 6));
        let mut found: Vec<(Code, String, String)> = found
            .into_iter()
            .map(|d| (d.code, d.message, d.hint))
            .collect();
        let mut expected: Vec<(Code, String, String)> = keys
            .iter()
            .map(|key| {
                (
                    Code::Syntax,
                    format!(
                        "the engine's `{kind}` takes `{key}` only in part format 7; this file is format 6"
                    ),
                    "set \"format\": 7 in this part file".to_string(),
                )
            })
            .collect();
        found.sort();
        expected.sort();
        assert_eq!(found, expected, "{id}");
    }
}

#[test]
fn the_catalog_parts_in_format_5_and_6_have_no_findings() {
    for id in [
        "diode-sod123",
        "lm7805-to220",
        "sk68xxmini-hs",
        "esp32s3-devkitc-1",
        "npn-2n2222-to92",
        "esp32s3-qfn56",
        "ssd1306-oled-128x64",
        "hd44780-1602-i2c",
        "thermistor-ntc-10k",
        "battery-9v-cell",
        "battery-aa-4x-cell",
        "led-5mm-rgb-cc",
        "led-5mm-rgb-ca",
        "joystick-ky023-pot",
    ] {
        let text =
            fs::read_to_string(common::repo().join(format!("parts/{id}/part.jsonc"))).unwrap();
        assert_eq!(findings(&text), [], "{id}");
    }
}

#[test]
fn accepted_warnings_checks_format_7_and_rejects_wildcards_and_leaf_parts() {
    // Composite catalog part in format 6 is told to set format 7 for accepted_warnings.
    let found = findings(&in_format("esp32s3-devkitc-1", 7, 6));
    let finding = found
        .iter()
        .find(|d| d.message.contains("`accepted_warnings`"))
        .unwrap_or_else(|| panic!("expected accepted_warnings format error: {found:#?}"));
    assert_eq!(finding.code, Code::Syntax);
    assert_eq!(finding.hint, "set \"format\": 7 in this part file");

    // Wildcard in code or pin is rejected.
    let composite = |code: &str, pin: &str| {
        format!(
            r#"{{
  "format": 7,
  "part": "board",
  "version": "0.1.0",
  "title": "Board",
  "summary": "Composite test part.",
  "tags": ["test"],
  "license": "MIT OR Apache-2.0",
  "docs": {{}},
  "real": {{ "generic": true, "maker": null, "number": null, "datasheet": "test" }},
  "pins": {{ "VCC": [1], "GND": [2] }},
  "accepted_warnings": [
    {{ "code": "{code}", "pin": "{pin}", "why": "By board design." }}
  ],
  "parts": {{ "r1": {{ "type": "resistor-axial", "resistance": "1k" }} }},
  "wires": [["VCC", "r1.a"], ["GND", "r1.b"]]
}}"#
        )
    };
    let wild_code = findings(&composite("rating.*", "r1.a"));
    assert!(
        wild_code
            .iter()
            .any(|d| d.message.contains("not a single warning code")),
        "{wild_code:#?}"
    );
    let wild_pin = findings(&composite("rating.supply_low", "r1.*"));
    assert!(
        wild_pin.iter().any(|d| d.message.contains("wildcard")),
        "{wild_pin:#?}"
    );

    // Leaf part cannot declare accepted_warnings.
    let leaf = r#"{
  "format": 7,
  "part": "leaf",
  "version": "0.1.0",
  "title": "Leaf",
  "summary": "Leaf test part.",
  "tags": ["test"],
  "license": "MIT OR Apache-2.0",
  "docs": {},
  "real": { "generic": true, "maker": null, "number": null, "datasheet": "test" },
  "pins": { "a": [1], "b": [2] },
  "accepted_warnings": [
    { "code": "rating.supply_low", "pin": "u.VCC", "why": "Not allowed on a leaf." }
  ],
  "model": { "kind": "resistor", "resistance": { "value": 1000, "unit": "ohm", "source": "user" } }
}"#;
    let leaf_found = findings(leaf);
    assert!(
        leaf_found
            .iter()
            .any(|d| d.message.contains("composite part")),
        "{leaf_found:#?}"
    );
}
