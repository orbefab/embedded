//! What `orbe check` runs on a part or Block after a project installs it.
//!
//! The part-file reader and the scene reader have already accepted these folders. This places
//! each part in a scene, and reads each Block's scene, through [`orbe_scene::read`]. The parts
//! are the folders being built, so a board's children and a Block's parts resolve from this
//! source. The ground beside a part is a voltage source this build writes. A catalog part there
//! would make one part's fields look like every part failed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use jsonc_parser::{JsonValue, ParseOptions, parse_to_value};
use orbe_scene::{Card, Diagnostic, Files, Folder, Literal, Param, REGISTRY};
use serde_json::{Map, Value, json};

/// A text file a required file param can name. `read` only checks that the path is text.
const FIRMWARE: &[u8] = include_bytes!("../../../packages/engine/tests/firmware/sketch/button.hex");

/// Findings `orbe check` would print for an install of these folders. Empty when every part
/// and every Block scene reads. File paths name the source folder.
pub(crate) fn findings(
    registry: &str,
    parts: &BTreeMap<String, Folder>,
    blocks: &BTreeMap<String, Folder>,
    cards: &[Card],
) -> Vec<Diagnostic> {
    let catalog = orbe_scene::catalog_cards();
    let official: BTreeSet<&str> = catalog.iter().map(|card| card.name.as_str()).collect();
    // An id this binary catalogs locks as `@orbefab`, the line an install of that part uses.
    // `read` then runs the lock check, so a sealed folder that differs from this binary is
    // refused the same way that install is. A line this build does not write is not a finding.
    let project = project_files(parts, &lock_text(cards, &official, registry));
    let mut found = Vec::new();
    for card in cards {
        let Some(folder) = parts.get(&card.name) else {
            continue;
        };
        let scene = scene_for(card, names_ground(folder));
        found.extend(read_scene(&project, &scene));
    }
    for (id, folder) in blocks {
        found.extend(name_block(id, read_block(&project, folder)));
    }
    found.sort_by(|a, b| {
        (&a.file, a.line, a.column, a.code, &a.message)
            .cmp(&(&b.file, b.line, b.column, b.code, &b.message))
    });
    found.dedup();
    found
}

fn project_files(parts: &BTreeMap<String, Folder>, lock: &str) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for (id, folder) in parts {
        for (rel, bytes) in folder {
            files.insert(PathBuf::from(format!("parts/{id}/{rel}")), bytes.clone());
        }
    }
    files.insert(PathBuf::from("parts.lock"), lock.as_bytes().to_vec());
    files.insert(PathBuf::from("probe-ground.jsonc"), probe_ground());
    files.insert(PathBuf::from("firmware.hex"), FIRMWARE.to_vec());
    files
}

fn lock_text(cards: &[Card], official: &BTreeSet<&str>, registry: &str) -> String {
    let mut lines: Vec<String> = cards
        .iter()
        .map(|card| {
            let registry = if official.contains(card.name.as_str()) {
                REGISTRY
            } else {
                registry
            };
            format!("{} {} {} {}", card.name, card.version, registry, card.hash)
        })
        .collect();
    lines.sort();
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn read_scene(project: &BTreeMap<PathBuf, Vec<u8>>, scene: &str) -> Vec<Diagnostic> {
    let mut files = project.clone();
    files.insert(PathBuf::from("scene.jsonc"), scene.as_bytes().to_vec());
    let held = Files::memory(files);
    match orbe_scene::read(&held, Path::new("scene.jsonc")) {
        Ok(_) => Vec::new(),
        Err(found) => found,
    }
}

fn read_block(project: &BTreeMap<PathBuf, Vec<u8>>, folder: &Folder) -> Vec<Diagnostic> {
    let mut files = project.clone();
    for (rel, bytes) in folder {
        if rel == "block.jsonc" || rel == "parts.lock" || rel.starts_with("parts/") {
            continue;
        }
        let path = if rel == "scene.jsonc" {
            PathBuf::from("scene.jsonc")
        } else {
            PathBuf::from(rel)
        };
        files.insert(path, bytes.clone());
    }
    let held = Files::memory(files);
    match orbe_scene::read(&held, Path::new("scene.jsonc")) {
        Ok(_) => Vec::new(),
        Err(found) => found,
    }
}

fn name_block(id: &str, found: Vec<Diagnostic>) -> Vec<Diagnostic> {
    found
        .into_iter()
        .map(|mut diagnostic| {
            if diagnostic.file == "scene.jsonc" {
                diagnostic.file = format!("blocks/{id}/scene.jsonc");
            }
            diagnostic
        })
        .collect()
}

fn names_ground(files: &Folder) -> bool {
    let Some(bytes) = files.get("part.jsonc") else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Ok(Some(value)) = parse_to_value(text, &ParseOptions::default()) else {
        return false;
    };
    let JsonValue::Object(props) = value else {
        return false;
    };
    props
        .into_iter()
        .any(|(key, value)| key == "ground" && matches!(value, JsonValue::String(_)))
}

fn current_format() -> u32 {
    orbe_scene::FORMATS.iter().copied().max().unwrap_or(1)
}

fn probe_ground() -> Vec<u8> {
    let value = json!({
        "format": current_format(),
        "pins": { "pos": [1], "neg": [2] },
        "model": {
            "kind": "voltage_source",
            "voltage": { "value": 0, "unit": "V", "source": "user" },
            "pins": { "p": "pos", "n": "neg" }
        }
    });
    let mut text = serde_json::to_string_pretty(&value).unwrap();
    text.push('\n');
    text.into_bytes()
}

fn scene_for(card: &Card, own_ground: bool) -> String {
    let mut instance = Map::new();
    instance.insert("type".into(), Value::String(card.name.clone()));
    for param in card.params.iter().filter(|param| param.required) {
        instance.insert(param.name.clone(), param_value(param));
    }
    let mut parts = Map::new();
    parts.insert("r1".into(), Value::Object(instance));
    let mut root = Map::new();
    root.insert("format".into(), json!(current_format()));
    if !own_ground {
        parts.insert("g".into(), json!({ "type": "./probe-ground.jsonc" }));
        root.insert("ground".into(), json!("g.neg"));
    }
    root.insert("parts".into(), Value::Object(parts));
    let mut text = serde_json::to_string_pretty(&Value::Object(root)).unwrap();
    text.push('\n');
    text
}

fn param_value(param: &Param) -> Value {
    if let Some(choices) = &param.choices {
        let name = choices
            .first()
            .map(|choice| choice.name.clone())
            .unwrap_or_default();
        return Value::String(name);
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
