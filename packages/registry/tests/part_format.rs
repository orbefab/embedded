//! Every part under `parts/` is in a format this binary reads, and loads under this engine.
//! A `released.lock` id whose folder is an older format this binary stopped reading is the one
//! skip: that file is refused on purpose, and loading it would only repeat the refusal.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use common::repo;
use jsonc_parser::{JsonValue, ParseOptions, parse_to_value};
use orbe_scene::{Diagnostic, Files, Literal, Param, catalog_cards};
use serde_json::{Map, Value, json};

#[test]
fn every_part_under_parts_loads_in_a_format_this_binary_reads() {
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
    fs::write(
        dir.path().join("probe-ground.jsonc"),
        serde_json::to_string_pretty(&probe_ground()).unwrap(),
    )
    .unwrap();
    fs::copy(
        root.join("packages/engine/tests/firmware/sketch/button.hex"),
        dir.path().join("firmware.hex"),
    )
    .unwrap();
    let released = released_ids(&root);
    let cards = catalog_cards();
    let mut loaded = 0usize;
    let mut failures = Vec::new();
    for id in &ids {
        let text = fs::read_to_string(root.join("parts").join(id).join("part.jsonc")).unwrap();
        if released_older(id, &text, &released) {
            continue;
        }
        if !reads_format(&text) {
            failures.push(format!(
                "{id}: format {} is not one this binary reads. Every part under parts/ has to be in a format this binary reads, and load. The only skip is a released.lock id whose file is an older format this binary stopped reading.",
                format_token(&text)
            ));
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

#[test]
fn an_enum_refusal_names_the_field_the_engine_does_not_read() {
    let message = "`u1`: the engine does not read `emission_coefficient.source.datasheet.basis`: unknown variant `typical`, expected one of `typ`";
    assert_eq!(
        field_names(message),
        vec!["emission_coefficient.source.datasheet.basis".to_string()]
    );
}

/// The newest format this binary reads. The probe scene and its ground use it, so dropping an
/// older format does not make the probe itself a `scene.format` error.
fn current_format() -> u32 {
    orbe_scene::FORMATS
        .iter()
        .copied()
        .max()
        .expect("this binary reads a format")
}

/// A voltage source the test writes. A catalog part here would fail every probe when that
/// part's fields change, and again when that part's format is no longer one this binary reads.
fn probe_ground() -> Value {
    json!({
        "format": current_format(),
        "pins": { "pos": [1], "neg": [2] },
        "model": {
            "kind": "voltage_source",
            "voltage": { "value": 0, "unit": "V", "source": "user" },
            "pins": { "p": "pos", "n": "neg" }
        }
    })
}

/// Part ids in `parts/`, plus any `released.lock` part whose folder is still there. A line
/// whose folder is gone cannot be rebuilt. A Block has no `model`.
fn part_ids(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(root.join("parts"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| root.join("parts").join(id).join("part.jsonc").is_file())
        .collect();
    for id in released_ids(root) {
        if root.join("parts").join(&id).join("part.jsonc").is_file()
            && !ids.iter().any(|seen| seen == &id)
        {
            ids.push(id);
        }
    }
    ids.sort();
    ids
}

fn released_ids(root: &Path) -> BTreeSet<String> {
    let lock = fs::read_to_string(root.join("released.lock")).unwrap_or_default();
    lock.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect()
}

/// `id` is in `released.lock` and its file's format is older than every format this binary
/// reads. The binary refuses that file on purpose.
fn released_older(id: &str, text: &str, released: &BTreeSet<String>) -> bool {
    let Some(n) = declared_format(text) else {
        return false;
    };
    let Some(oldest) = orbe_scene::FORMATS.iter().copied().min() else {
        return false;
    };
    released.contains(id) && n < oldest
}

fn reads_format(text: &str) -> bool {
    declared_format(text).is_some_and(|n| orbe_scene::FORMATS.contains(&n))
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

fn format_token(text: &str) -> String {
    match top_level(text, "format") {
        Some(JsonValue::Number(raw)) => raw.to_string(),
        Some(JsonValue::String(raw)) => format!("\"{raw}\""),
        Some(_) => "unreadable".into(),
        None => "missing".into(),
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
    root.insert("format".into(), json!(current_format()));
    if !own_ground {
        // The ground is a file this test writes, not a catalog part. A catalog resistor here
        // makes a resistor field change look like every part broke.
        parts.insert("g".into(), json!({ "type": "./probe-ground.jsonc" }));
        root.insert("ground".into(), json!("g.neg"));
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
        "{id}: field {fields}. Raise parts/{id}/part.jsonc to the next format, move its model onto what this binary reads, and drop the format that file leaves. Add that format to the formats this binary reads. Drop an older format from those formats only once every part under parts/ has left it, apart from a released.lock id whose file is an older format this binary no longer reads. An old copy of {id} is then refused with `orbe add {id} --overwrite` instead of a field error.\n{shown}"
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

/// The field a message names in backticks after `field `, `needs `, or `read `. The engine's
/// schema refusal is `the engine does not read `<field>``, and an unknown variant is that shape.
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
        if !key.is_empty()
            && (before.ends_with("field ")
                || before.ends_with("needs ")
                || before.ends_with("read "))
        {
            found.push(key.to_string());
        }
        rest = &after[end + 1..];
    }
    found
}
