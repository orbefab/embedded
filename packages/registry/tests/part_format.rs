//! Every part under `parts/` is in a format this binary reads, and loads under this engine.
//! A `released.lock` id whose folder is an older format this binary stopped reading is the one
//! skip: that file is refused on purpose, and loading it would only repeat the refusal.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use common::repo;
use jsonc_parser::{JsonValue, ParseOptions, parse_to_serde_value, parse_to_value};
use orbe_scene::{Diagnostic, Files, Literal, catalog_cards};
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
    // The probe is a project. Its `parts/` is this repo's, so a scene id and a child both resolve.
    std::os::unix::fs::symlink(root.join("parts"), dir.path().join("parts")).unwrap();
    fs::write(
        dir.path().join("probe-ground.jsonc"),
        serde_json::to_string_pretty(&probe_ground()).unwrap(),
    )
    .unwrap();
    let released = released_ids(&root);
    let cards = catalog_cards(&root.join("parts"));
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
        let scene = scene_for(&root, card, has_ground(&text));
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

/// The newest scene format this binary reads. The probe scene uses it, so dropping an older
/// format does not make the probe itself a `scene.format` error.
fn scene_format() -> u32 {
    orbe_scene::SCENE_FORMATS
        .iter()
        .copied()
        .max()
        .expect("this binary reads a scene format")
}

/// The newest part format this binary reads, for the probe's ground, for the same reason.
fn part_format() -> u32 {
    orbe_scene::PART_FORMATS
        .iter()
        .copied()
        .max()
        .expect("this binary reads a part format")
}

/// A voltage source the test writes. A catalog part here would fail every probe when that
/// part's fields change, and again when that part's format is no longer one this binary reads.
fn probe_ground() -> Value {
    json!({
        "format": part_format(),
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
    let Some(oldest) = orbe_scene::PART_FORMATS.iter().copied().min() else {
        return false;
    };
    released.contains(id) && n < oldest
}

fn reads_format(text: &str) -> bool {
    declared_format(text).is_some_and(|n| orbe_scene::PART_FORMATS.contains(&n))
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

fn scene_for(root: &Path, card: &orbe_scene::Card, own_ground: bool) -> String {
    let mut parts = Map::new();
    parts.insert("u1".into(), Value::Object(instance(root, card)));
    let mut scene = Map::new();
    scene.insert("format".into(), json!(scene_format()));
    if !own_ground {
        // The ground is a file this test writes, not a catalog part. A catalog resistor here
        // makes a resistor field change look like every part broke.
        parts.insert("g".into(), json!({ "type": "./probe-ground.jsonc" }));
        scene.insert("ground".into(), json!("g.neg"));
    }
    scene.insert("parts".into(), Value::Object(parts));
    serde_json::to_string_pretty(&Value::Object(scene)).unwrap()
}

fn instance(root: &Path, card: &orbe_scene::Card) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("type".into(), Value::String(card.name.clone()));
    for param in card.params.iter().filter(|param| param.required) {
        let Some(example) = &param.example else {
            continue;
        };
        let value = match example {
            Literal::Number(raw) => {
                serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
            }
            Literal::Name(name) if param.file.is_some() => {
                Value::String(flat_example(root, &card.name, &param.name, name))
            }
            Literal::Name(name) => Value::String(name.clone()),
        };
        object.insert(param.name.clone(), value);
    }
    object
}

/// `parts/<id>/<file>` in this test's flat catalog. A param that only passes its value on
/// names the file in the folder of the part that declares it.
fn flat_example(root: &Path, id: &str, param: &str, file: &str) -> String {
    let mut current = id.to_string();
    let mut name = param.to_string();
    for _ in 0..16 {
        let path = root.join("parts").join(&current).join("part.jsonc");
        let Ok(text) = fs::read_to_string(&path) else {
            break;
        };
        let Ok(value) = parse_to_serde_value::<Value>(&text, &ParseOptions::default()) else {
            break;
        };
        if owns_param(&value, &name) {
            return format!("parts/{current}/{file}");
        }
        let Some((child, child_param)) = passed_to(&value, &name) else {
            break;
        };
        current = child;
        name = child_param;
    }
    format!("parts/{current}/{file}")
}

fn owns_param(part: &Value, name: &str) -> bool {
    let Some(decl) = part
        .get("params")
        .and_then(|node| node.get(name))
        .and_then(|node| node.as_object())
    else {
        return true;
    };
    decl.contains_key("file") || decl.contains_key("unit") || decl.contains_key("choices")
}

fn passed_to(part: &Value, name: &str) -> Option<(String, String)> {
    let specs = part.get("parts")?.as_object()?;
    for spec in specs.values() {
        let spec = spec.as_object()?;
        let ty = spec.get("type")?.as_str()?;
        if ty.starts_with('.') || ty.starts_with('@') {
            continue;
        }
        for (key, value) in spec {
            if value.get("param").and_then(|node| node.as_str()) == Some(name) {
                return Some((ty.to_string(), key.clone()));
            }
        }
    }
    None
}

fn explain(id: &str, diagnostics: &[Diagnostic]) -> String {
    let fields = fields_of(diagnostics);
    let shown = diagnostics
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{id}: field {fields}. Raise parts/{id}/part.jsonc to the next format, move its model onto what this binary reads, and drop the format that file leaves. Add that format to the formats this binary reads. Drop an older format from those formats only once every part under parts/ has left it, apart from a released.lock id whose file is an older format this binary no longer reads. An old copy of {id} is the project's file. The hint says to remove the folder and run `orbe add {id}`.\n{shown}"
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
