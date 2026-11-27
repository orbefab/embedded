//! A registry built from release tags keeps an older record and names the lowest orbe.

mod common;

use common::{put, record};
use orbe_registry::{Release, build, publish, unbumped};

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
fn a_format_no_release_reads_names_the_next_release() {
    let body = common::LAMP.replace("\"format\": 1", "\"format\": 5");
    let src = source("lamp", "1.2.0", "Lamp", &body);
    put(
        src.path(),
        "blocks/lamp-demo/block.jsonc",
        r#"{ "format": 1, "block": "lamp-demo", "version": "0.1.0", "title": "A lamp", "board": "lamp", "license": "MIT" }"#,
    );
    put(
        src.path(),
        "blocks/lamp-demo/scene.jsonc",
        r#"{ "format": 1, "parts": { "l1": { "type": "lamp" } }, "ground": "l1.a" }"#,
    );
    let registry = build(src.path()).unwrap();
    let merged = publish(&[Release {
        version: "0.2.1".into(),
        formats: vec![1, 2, 3, 4],
        registry,
    }])
    .unwrap();
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
    assert_eq!(row("lamp")["orbe"], "0.2.2");
    assert_eq!(row("lamp-demo")["orbe"], "0.2.2");
}

#[test]
fn a_release_with_no_tag_is_refused() {
    let error = publish(&[]).unwrap_err().to_string();
    assert!(error.contains("v0.2.0"), "{error}");
}

#[test]
fn a_later_release_unlisting_a_part_unlists_its_older_rows() {
    let first = tempfile::tempdir().unwrap();
    put(
        first.path(),
        "registry.jsonc",
        r#"{ "format": 1, "name": "@acme" }"#,
    );
    let part = |name: &str, version: &str| {
        common::LAMP
            .replace("\"part\": \"lamp\"", &format!("\"part\": \"{name}\""))
            .replace(
                "\"version\": \"1.2.0\"",
                &format!("\"version\": \"{version}\""),
            )
    };
    put(
        first.path(),
        "parts/lamp/part.jsonc",
        &part("lamp", "1.0.0"),
    );
    put(
        first.path(),
        "parts/chip/part.jsonc",
        &part("chip", "1.0.0"),
    );
    put(
        first.path(),
        "parts/diode/part.jsonc",
        &part("diode", "1.0.0"),
    );

    let second = tempfile::tempdir().unwrap();
    put(
        second.path(),
        "registry.jsonc",
        r#"{ "format": 1, "name": "@acme", "unlisted": ["chip", "diode"] }"#,
    );
    put(
        second.path(),
        "parts/lamp/part.jsonc",
        &part("lamp", "1.0.0"),
    );
    // chip keeps its 1.0.0 record from the older release.
    put(
        second.path(),
        "parts/chip/part.jsonc",
        &part("chip", "1.0.0"),
    );
    // diode ships a newer version alongside its older 1.0.0 row.
    put(
        second.path(),
        "parts/diode/part.jsonc",
        &part("diode", "1.1.0"),
    );

    let older = build(first.path()).unwrap();
    let newer = build(second.path()).unwrap();
    let kept_chip = older.output["chip@1.0.0.json"].clone();
    let merged = publish(&[
        Release {
            version: "0.2.0".into(),
            formats: vec![1, 2, 3],
            registry: older,
        },
        Release {
            version: "0.2.1".into(),
            formats: vec![1, 2, 3],
            registry: newer,
        },
    ])
    .unwrap();

    assert_eq!(merged["chip@1.0.0.json"], kept_chip);
    let index: serde_json::Value = serde_json::from_slice(&merged["index.json"]).unwrap();
    let rows = index["items"].as_array().unwrap();
    let row = |name: &str, version: &str| {
        rows.iter()
            .find(|row| row["name"] == name && row["version"] == version)
            .unwrap()
    };
    assert!(row("lamp", "1.0.0").get("listed").is_none());
    assert_eq!(row("chip", "1.0.0")["listed"], serde_json::json!(false));
    assert_eq!(row("diode", "1.0.0")["listed"], serde_json::json!(false));
    assert_eq!(row("diode", "1.1.0")["listed"], serde_json::json!(false));
}

#[test]
fn a_part_needs_the_orbe_every_part_inside_it_needs() {
    let first = tempfile::tempdir().unwrap();
    put(first.path(), "registry.jsonc", common::MANIFEST);
    put(first.path(), "parts/lamp/part.jsonc", common::LAMP);
    let second = tempfile::tempdir().unwrap();
    put(second.path(), "registry.jsonc", common::MANIFEST);
    put(second.path(), "parts/glow/part.jsonc", GLOW);
    // Both sort before glow, so the release lists each before the part it is built from.
    put(
        second.path(),
        "parts/a-pair/part.jsonc",
        r#"{ "format": 1, "part": "a-pair", "version": "0.1.0", "title": "A pair", "license": "MIT", "uses": { "glow": "1.0.0" }, "parts": { "g": { "type": "glow" } } }"#,
    );
    put(
        second.path(),
        "parts/a-box/part.jsonc",
        r#"{ "format": 1, "part": "a-box", "version": "0.1.0", "title": "A box", "license": "MIT", "uses": { "a-pair": "0.1.0" }, "parts": { "inner": { "type": "a-pair" } } }"#,
    );
    let merged = publish(&[
        Release {
            version: "0.2.0".into(),
            formats: vec![1, 2, 3],
            registry: build(first.path()).unwrap(),
        },
        Release {
            version: "0.2.1".into(),
            formats: vec![1, 2, 3, 4],
            registry: build(second.path()).unwrap(),
        },
    ])
    .unwrap();
    let index: serde_json::Value = serde_json::from_slice(&merged["index.json"]).unwrap();
    let orbe = |name: &str| {
        index["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap()["orbe"]
            .clone()
    };
    assert_eq!(orbe("lamp"), "0.2.0");
    assert_eq!(orbe("glow"), "0.2.1");
    assert_eq!(orbe("a-pair"), "0.2.1");
    assert_eq!(orbe("a-box"), "0.2.1");
}

#[test]
fn a_changed_item_at_a_published_version_is_named() {
    let published = common::third_party();
    let release = Release {
        version: "0.2.0".into(),
        formats: vec![1, 2, 3],
        registry: build(published.path()).unwrap(),
    };

    let same = common::third_party();
    assert!(unbumped(std::slice::from_ref(&release), &build(same.path()).unwrap()).is_empty());

    let edited = common::third_party();
    put(
        edited.path(),
        "parts/lamp/part.jsonc",
        &common::LAMP.replace("\"title\": \"Lamp\"", "\"title\": \"Brighter lamp\""),
    );
    let lines = unbumped(
        std::slice::from_ref(&release),
        &build(edited.path()).unwrap(),
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("`lamp` 1.2.0 has other files than v0.2.0"),
        "{lines:?}"
    );
    assert!(
        lines[0].contains("raise the version in parts/lamp/part.jsonc"),
        "{lines:?}"
    );

    // A raised part version changes the Block's parts, so the Block needs a new version too.
    let raised = common::third_party();
    put(
        raised.path(),
        "parts/lamp/part.jsonc",
        &common::LAMP
            .replace("\"version\": \"1.2.0\"", "\"version\": \"1.3.0\"")
            .replace("\"title\": \"Lamp\"", "\"title\": \"Brighter lamp\""),
    );
    let lines = unbumped(
        std::slice::from_ref(&release),
        &build(raised.path()).unwrap(),
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("`lamp-demo` 0.1.0 has other dependencies than v0.2.0"),
        "{lines:?}"
    );
    assert!(
        lines[0].contains("blocks/lamp-demo/block.jsonc"),
        "{lines:?}"
    );
}
