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
        ("lm358-dip8", 5, "op_amp", &[("ratings", 5), ("ratings", 5)]),
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
            6,
            "ws2812",
            &[("draw", 5), ("min_supply", 5), ("ratings", 6)],
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
        let found = findings(&in_format(id, 6, 5));
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
        "npn-2n2222-to92",
        "esp32s3-qfn56",
        "ssd1306-oled-128x64",
        "hd44780-1602-i2c",
        "thermistor-ntc-10k",
        "battery-9v-cell",
        "battery-aa-4x-cell",
    ] {
        let text =
            fs::read_to_string(common::repo().join(format!("parts/{id}/part.jsonc"))).unwrap();
        assert_eq!(findings(&text), [], "{id}");
    }
}
