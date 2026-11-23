//! Format 4 carries the keys the published 0.2.0 binary refuses. A part that uses one of them
//! before format 4 is told to set format 4.

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
    let text = catalog("parts/capacitor-tantalum-3216/part.jsonc").replacen(
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
