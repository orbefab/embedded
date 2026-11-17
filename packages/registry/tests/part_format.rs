//! Every catalog part, and every part `released.lock` still has a folder for, loads under this
//! engine, or its `format` is no longer one this binary reads. The lock holds a served record's
//! hash. The record is rebuilt from the part file in source, so this reads that file.

mod common;

use std::fs;
use std::path::Path;

use common::repo;
use jsonc_parser::{JsonValue, ParseOptions, parse_to_value};
use orbe_scene::{Diagnostic, Files, Literal, Param, catalog_cards};
use serde_json::{Map, Value, json};

#[test]
fn every_catalog_part_and_released_part_loads_or_its_format_was_raised() {
    let root = repo();
    let ids = part_ids(&root);
    assert!(
        ids.len() > 1,
        "the catalog has no parts, so this check would pass without loading one"
    );
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("block.jsonc"),
        "{ \"format\": 1, \"block\": \"probe\", \"version\": \"0.1.0\", \"title\": \"Probe\", \"board\": \"probe\", \"license\": \"MIT\" }\n",
    )
    .unwrap();
    fs::copy(
        root.join("packages/engine/tests/firmware/sketch/button.hex"),
        dir.path().join("firmware.hex"),
    )
    .unwrap();
    let cards = catalog_cards();
    let mut loaded = 0usize;
    let mut failures = Vec::new();
    for id in &ids {
        let text = fs::read_to_string(root.join("parts").join(id).join("part.jsonc")).unwrap();
        if raised(&text) {
            continue;
        }
        let card = cards
            .iter()
            .find(|card| card.name == *id)
            .unwrap_or_else(|| panic!("{id} is in parts/ and has no card"));
        let scene = scene_for(card, has_ground(&text));
        let path = dir.path().join("scene.jsonc");
        fs::write(&path, &scene).unwrap();
        match orbe_scene::read(&Files::disk(), &path) {
            Ok(_) => loaded += 1,
            Err(diagnostics) => failures.push(explain(id, &diagnostics)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert!(
        loaded > 0,
        "no part is in a format this binary reads, so the check loaded nothing"
    );
}

/// Part ids in `parts/`, plus any `released.lock` part whose folder is still there. A line
/// whose folder is gone cannot be rebuilt. A Block has no `model`.
fn part_ids(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(root.join("parts"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| root.join("parts").join(id).join("part.jsonc").is_file())
        .collect();
    let lock = fs::read_to_string(root.join("released.lock")).unwrap_or_default();
    for line in lock.lines().filter(|line| !line.is_empty()) {
        let Some(id) = line.split_whitespace().next() else {
            continue;
        };
        if root.join("parts").join(id).join("part.jsonc").is_file()
            && !ids.iter().any(|seen| seen == id)
        {
            ids.push(id.to_string());
        }
    }
    ids.sort();
    ids
}

/// A whole `format` this binary does not read. That is the raise: the old file is refused
/// before the engine sees it.
fn raised(text: &str) -> bool {
    declared_format(text).is_some_and(|n| !orbe_scene::FORMATS.contains(&n))
}

fn declared_format(text: &str) -> Option<u32> {
    let JsonValue::Number(raw) = top_level(text, "format")? else {
        return None;
    };
    let n: f64 = raw.parse().ok()?;
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX) {
        Some(n as u32)
    } else {
        None
    }
}

fn has_ground(text: &str) -> bool {
    matches!(top_level(text, "ground"), Some(JsonValue::String(_)))
}

fn top_level<'a>(text: &'a str, key: &str) -> Option<JsonValue<'a>> {
    let value = parse_to_value(text, &ParseOptions::default())
        .ok()
        .flatten()?;
    let JsonValue::Object(props) = value else {
        return None;
    };
    props
        .into_iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn scene_for(card: &orbe_scene::Card, own_ground: bool) -> String {
    let mut parts = Map::new();
    parts.insert("u1".into(), Value::Object(instance(card)));
    let mut root = Map::new();
    root.insert("format".into(), json!(1));
    if !own_ground {
        parts.insert(
            "g".into(),
            json!({ "type": "resistor-axial", "resistance": "1k" }),
        );
        root.insert("ground".into(), json!("g.a"));
    }
    root.insert("parts".into(), Value::Object(parts));
    serde_json::to_string_pretty(&Value::Object(root)).unwrap()
}

fn instance(card: &orbe_scene::Card) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("type".into(), Value::String(card.name.clone()));
    for param in card.params.iter().filter(|param| param.required) {
        object.insert(param.name.clone(), param_value(param));
    }
    object
}

fn param_value(param: &Param) -> Value {
    if let Some(choices) = &param.choices {
        let name = choices
            .first()
            .unwrap_or_else(|| panic!("{} has a required choice and no choice", param.name));
        return Value::String(name.name.clone());
    }
    if param.file.is_some() {
        return json!("firmware.hex");
    }
    json!(number_text(param))
}

fn number_text(param: &Param) -> String {
    let above = param.above.as_ref().and_then(literal_f64);
    let below = param.below.as_ref().and_then(literal_f64);
    let mut value = match param.unit.as_deref() {
        Some("F") | Some("H") => 1e-6,
        Some("s") => 1e-3,
        _ => 1000.0,
    };
    if let Some(low) = above
        && value <= low
    {
        value = low + low.abs() + 1.0;
    }
    if let Some(high) = below
        && value >= high
    {
        let low = above.unwrap_or(0.0);
        value = (low + high) / 2.0;
    }
    format!("{value}")
}

fn literal_f64(literal: &Literal) -> Option<f64> {
    match literal {
        Literal::Number(raw) => raw.parse().ok(),
        Literal::Name(_) => None,
    }
}

fn explain(id: &str, diagnostics: &[Diagnostic]) -> String {
    let fields = fields_of(diagnostics);
    let shown = diagnostics
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{id}: field {fields}. Raise the part format in parts/{id}/part.jsonc, and drop that format from the formats this binary reads, so an old copy is refused with `orbe add {id} --overwrite` instead of a field error.\n{shown}"
    )
}

fn fields_of(diagnostics: &[Diagnostic]) -> String {
    let mut names = Vec::new();
    for diagnostic in diagnostics {
        for key in field_names(&diagnostic.message) {
            if !names.iter().any(|seen| seen == &key) {
                names.push(key);
            }
        }
    }
    if names.is_empty() {
        "(none named)".into()
    } else {
        names.join(", ")
    }
}

fn field_names(message: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = message;
    while let Some(at) = rest.find('`') {
        let after = &rest[at + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let key = &after[..end];
        let before = &rest[..at];
        if !key.is_empty() && (before.ends_with("field ") || before.ends_with("needs ")) {
            found.push(key.to_string());
        }
        rest = &after[end + 1..];
    }
    found
}
