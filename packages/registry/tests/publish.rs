//! A registry built from release tags keeps an older record and names the lowest orbe.

mod common;

use common::{put, record};
use orbe_registry::{Release, build, publish};

const GLOW: &str = r#"{
  "format": 4,
  "part": "glow",
  "version": "1.0.0",
  "title": "Glow",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "params": { "resistance": { "unit": "ohm", "default": "220", "min_exclusive": 0, "sets": "/resistance" } },
  "model": { "kind": "resistor", "ratings": { "power": null } }
}
"#;

fn source(part: &str, version: &str, title: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "registry.jsonc",
        r#"{ "format": 1, "name": "@acme" }"#,
    );
    let mut text = body.replace("\"part\": \"lamp\"", &format!("\"part\": \"{part}\""));
    text = text.replace(
        "\"version\": \"1.2.0\"",
        &format!("\"version\": \"{version}\""),
    );
    text = text.replace("\"title\": \"Lamp\"", &format!("\"title\": \"{title}\""));
    put(dir.path(), &format!("parts/{part}/part.jsonc"), &text);
    dir
}

#[test]
fn an_older_release_keeps_its_record_and_a_row_names_the_lowest_orbe() {
    let first = source("lamp", "1.0.0", "First", common::LAMP);
    let second_src = tempfile::tempdir().unwrap();
    put(
        second_src.path(),
        "registry.jsonc",
        r#"{ "format": 1, "name": "@acme" }"#,
    );
    let changed = common::LAMP
        .replace("\"version\": \"1.2.0\"", "\"version\": \"1.0.0\"")
        .replace("\"title\": \"Lamp\"", "\"title\": \"Replaced\"");
    put(second_src.path(), "parts/lamp/part.jsonc", &changed);
    put(second_src.path(), "parts/glow/part.jsonc", GLOW);
    let older = build(first.path()).unwrap();
    let newer = build(second_src.path()).unwrap();
    let kept = older.output["lamp@1.0.0.json"].clone();
    let merged = publish(&[
        Release {
            version: "0.2.0".into(),
            formats: vec![1, 2, 3],
            registry: older,
        },
        Release {
            version: "0.2.1".into(),
            formats: vec![1, 2, 3, 4],
            registry: newer,
        },
    ])
    .unwrap();
    assert_eq!(merged["lamp@1.0.0.json"], kept);
    let index: serde_json::Value = serde_json::from_slice(&merged["index.json"]).unwrap();
    let row = |name: &str| {
        index["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(row("lamp")["orbe"], "0.2.0");
    assert_eq!(row("lamp")["version"], "1.0.0");
    assert_eq!(row("glow")["orbe"], "0.2.1");
    assert_eq!(record(&merged, "glow@1.0.0")["title"], "Glow");
    assert!(merged.contains_key("item.schema.json"));
    assert!(merged.contains_key("index.schema.json"));
}

#[test]
fn a_release_with_no_tag_is_refused() {
    let error = publish(&[]).unwrap_err().to_string();
    assert!(error.contains("v0.2.0"), "{error}");
}
