//! Format 4 carries the keys the published 0.2.0 binary refuses. A part that uses one of them
//! before format 4 is told to set format 4. A `ws2812` current the engine does not have yet is
//! still told to set format 4, so the message does not depend on that field loading.

mod common;

use std::collections::BTreeMap;
use std::fs;

use orbe_scene::{Code, Diagnostic};

fn findings(text: &str) -> Vec<Diagnostic> {
    let mut files = BTreeMap::new();
    files.insert("part.jsonc".to_string(), text.as_bytes().to_vec());
    orbe_scene::part_findings("sample", &files)
}

fn catalog(rel: &str) -> String {
    fs::read_to_string(common::repo().join(rel)).unwrap()
}

#[test]
fn live_and_a_joint_by_before_format_4_are_told_to_set_format_4() {
    let text =
        catalog("parts/trimmer-3362p/part.jsonc").replacen("\"format\": 4,", "\"format\": 1,", 1);
    let kept = findings(&text);
    let found: Vec<(Code, &str, &str)> = kept
        .iter()
        .map(|d| (d.code, d.message.as_str(), d.hint.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            (
                Code::Param,
                "param `position` takes \"live\" only in part format 4; this file is format 1",
                "set \"format\": 4 in this part file"
            ),
            (
                Code::Param,
                "joint `rotor` takes \"by\" only in part format 4; this file is format 1",
                "set \"format\": 4 in this part file"
            ),
        ]
    );
}

#[test]
fn a_capacitor_rating_before_format_4_is_told_to_set_format_4() {
    let text = catalog("parts/capacitor-electrolytic-radial/part.jsonc").replacen(
        "\"format\": 4,",
        "\"format\": 1,",
        1,
    );
    let kept = findings(&text);
    let found: Vec<(Code, &str, &str)> = kept
        .iter()
        .map(|d| (d.code, d.message.as_str(), d.hint.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            (
                Code::Syntax,
                "the engine's `capacitor` takes `series_resistance` only in part format 4; this file is format 1",
                "set \"format\": 4 in this part file"
            ),
            (
                Code::Syntax,
                "the engine's `capacitor` takes `ratings` only in part format 4; this file is format 1",
                "set \"format\": 4 in this part file"
            ),
        ]
    );
}

#[test]
fn a_pixel_current_before_format_4_is_told_to_set_format_4() {
    let text = catalog("parts/sk68xxmini-hs/part.jsonc").replacen(
        "\"kind\": \"ws2812\",",
        "\"kind\": \"ws2812\", \"channel_current\": { \"value\": 0.012, \"unit\": \"A\", \"source\": \"user\" }, \"idle_current\": { \"value\": 0.001, \"unit\": \"A\", \"source\": \"user\" },",
        1,
    );
    let kept = findings(&text);
    let found: Vec<(Code, &str)> = kept.iter().map(|d| (d.code, d.message.as_str())).collect();
    assert!(
        found.contains(&(
            Code::Syntax,
            "the engine's `ws2812` takes `channel_current` only in part format 4; this file is format 3"
        )),
        "{found:#?}"
    );
    assert!(
        found.contains(&(
            Code::Syntax,
            "the engine's `ws2812` takes `idle_current` only in part format 4; this file is format 3"
        )),
        "{found:#?}"
    );
}

#[test]
fn a_pixel_current_in_format_4_is_not_a_format_gate() {
    let text = catalog("parts/sk68xxmini-hs/part.jsonc")
        .replacen("\"format\": 3,", "\"format\": 4,", 1)
        .replacen(
            "\"kind\": \"ws2812\",",
            "\"kind\": \"ws2812\", \"channel_current\": { \"value\": 0.012, \"unit\": \"A\", \"source\": \"user\" }, \"idle_current\": { \"value\": 0.001, \"unit\": \"A\", \"source\": \"user\" },",
            1,
        );
    let found = findings(&text);
    assert!(
        found.iter().all(|d| !d.message.contains("part format 4")),
        "{found:#?}"
    );
}
