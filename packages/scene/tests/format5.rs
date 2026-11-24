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

/// The catalog part `id` with its format set to 4.
fn in_format_4(id: &str) -> String {
    let text = fs::read_to_string(common::repo().join(format!("parts/{id}/part.jsonc"))).unwrap();
    assert!(text.contains("\"format\": 5,"), "{id} is not in format 5");
    text.replacen("\"format\": 5,", "\"format\": 4,", 1)
}

#[test]
fn each_format_5_key_before_format_5_is_told_to_set_format_5() {
    for (id, kind, keys) in [
        ("diode-sod123", "diode", &["ratings.current.basis"][..]),
        (
            "capacitor-electrolytic-radial",
            "capacitor",
            &["series_resistance.tan_delta", "series_resistance.frequency"],
        ),
        (
            "lm7805-to220",
            "linear_regulator",
            &[
                "ratings.input_voltage",
                "ratings.current.basis",
                "ratings.power.basis",
            ],
        ),
        ("lm358-dip8", "op_amp", &["ratings", "ratings"]),
        (
            "npn-2n2222-to92",
            "npn",
            &[
                "forward_knee_current",
                "ratings.current.basis",
                "ratings.power.basis",
            ],
        ),
        ("sk68xxmini-hs", "ws2812", &["draw", "min_supply"]),
    ] {
        let found = findings(&in_format_4(id));
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
                        "the engine's `{kind}` takes `{key}` only in part format 5; this file is format 4"
                    ),
                    "set \"format\": 5 in this part file".to_string(),
                )
            })
            .collect();
        found.sort();
        expected.sort();
        assert_eq!(found, expected, "{id}");
    }
}

#[test]
fn the_catalog_parts_in_format_5_have_no_findings() {
    for id in [
        "diode-sod123",
        "lm7805-to220",
        "sk68xxmini-hs",
        "npn-2n2222-to92",
    ] {
        let text =
            fs::read_to_string(common::repo().join(format!("parts/{id}/part.jsonc"))).unwrap();
        assert_eq!(findings(&text), [], "{id}");
    }
}
