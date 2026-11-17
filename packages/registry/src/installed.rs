//! What `orbe check` runs on a part or Block after a project installs it.
//!
//! The part-file reader and the scene reader have already accepted these folders. This places
//! each part in a scene, and reads each Block's scene, through [`orbe_scene::read`]. The parts
//! are the folders being built, so a board's children and a Block's parts resolve from this
//! source. A Block's project is those part folders, the Block's own files and the lock. The
//! ground and the firmware beside a part are files this build writes for that part's scene
//! alone. A catalog part there would make one part's fields look like every part failed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

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
        let named = names_ground(folder) || nested_ground(parts, &card.name);
        let scene = scene_for(card, named);
        found.extend(read_scene(&project, &scene, card, folder));
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

const PROBE_GROUND_FILE: &str = "probe-ground.jsonc";
const PROBE_FIRMWARE_FILE: &str = "firmware.hex";

fn read_scene(
    project: &BTreeMap<PathBuf, Vec<u8>>,
    scene: &str,
    card: &Card,
    folder: &Folder,
) -> Vec<Diagnostic> {
    let mut files = project.clone();
    files.insert(PathBuf::from("scene.jsonc"), scene.as_bytes().to_vec());
    // Only this generated scene names these. A Block's project does not have them, and a
    // `..` walk from a part file cannot land on them.
    files.insert(PathBuf::from(PROBE_GROUND_FILE), probe_ground());
    files.insert(PathBuf::from(PROBE_FIRMWARE_FILE), FIRMWARE.to_vec());
    let held = Files::memory(files).hide_from_parent_walk([PROBE_GROUND_FILE, PROBE_FIRMWARE_FILE]);
    match orbe_scene::read(&held, Path::new("scene.jsonc")) {
        Ok(_) => Vec::new(),
        Err(found) => place_part(card, folder, found),
    }
}

/// A finding on the generated scene quotes a value this probe wrote. One that names a param
/// whose bounds hold no number is the part's, at that param. Any other out-of-range line
/// there is the stand-in, and the registry has no `scene.jsonc` to blame.
fn place_part(card: &Card, folder: &Folder, found: Vec<Diagnostic>) -> Vec<Diagnostic> {
    found
        .into_iter()
        .filter_map(|mut diagnostic| {
            if diagnostic.file != "scene.jsonc"
                || diagnostic.code != orbe_scene::Code::Param
                || !diagnostic.message.contains("is out of range")
            {
                return Some(diagnostic);
            }
            let name = probed_param(&diagnostic.message)?;
            if !card
                .params
                .iter()
                .any(|param| param.name == name && interval_empty(param))
            {
                return None;
            }
            let text = std::str::from_utf8(folder.get("part.jsonc")?).ok()?;
            let (line, column) = key_at(text, name)?;
            diagnostic.file = format!("parts/{}/part.jsonc", card.name);
            diagnostic.line = line;
            diagnostic.column = column;
            Some(diagnostic)
        })
        .collect()
}

fn probed_param(message: &str) -> Option<&str> {
    message
        .strip_prefix('`')?
        .split_once('`')?
        .0
        .strip_prefix("r1.")
}

fn key_at(text: &str, key: &str) -> Option<(usize, usize)> {
    let pattern = format!("\"{key}\"");
    let start = text.find(&pattern)?;
    let line = text[..start].matches('\n').count() + 1;
    let line_start = text[..start].rfind('\n').map_or(0, |index| index + 1);
    let column = text[line_start..start].chars().count() + 1;
    Some((line, column))
}

fn interval_empty(param: &Param) -> bool {
    match (
        param.above.as_ref().and_then(literal_f64),
        param.below.as_ref().and_then(literal_f64),
    ) {
        (Some(low), Some(high)) => low >= high,
        _ => false,
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
    utf8(files, "part.jsonc").is_some_and(|text| text_names_ground(&text))
}

fn text_names_ground(text: &str) -> bool {
    let Ok(Some(JsonValue::Object(props))) = parse_to_value(text, &ParseOptions::default()) else {
        return false;
    };
    props
        .into_iter()
        .any(|(key, value)| key == "ground" && matches!(value, JsonValue::String(_)))
}

/// A nested part names a ground. The scene's own ground would hide it, so the probe leaves
/// its source out and those grounds are compared, the way a scene that only places the part does.
fn nested_ground(parts: &BTreeMap<String, Folder>, id: &str) -> bool {
    let mut seen = BTreeSet::new();
    seen.insert(id.to_string());
    let Some(folder) = parts.get(id) else {
        return false;
    };
    children_name_ground(parts, folder, "part.jsonc", &mut seen)
}

fn children_name_ground(
    parts: &BTreeMap<String, Folder>,
    folder: &Folder,
    file: &str,
    seen: &mut BTreeSet<String>,
) -> bool {
    let Some(text) = utf8(folder, file) else {
        return false;
    };
    let dir = parent_key(file);
    child_types(&text)
        .into_iter()
        .any(|ty| type_names_ground(parts, folder, &dir, &ty, seen))
}

fn type_names_ground(
    parts: &BTreeMap<String, Folder>,
    folder: &Folder,
    dir: &str,
    ty: &str,
    seen: &mut BTreeSet<String>,
) -> bool {
    if ty.starts_with("./") || ty.starts_with("../") {
        // A path that leaves this folder is not one of the part's files. The probe's own
        // files sit outside it, and a `..` walk must not treat them as the author's ground.
        let Some(key) = path_inside(dir, ty).and_then(|key| json_key(folder, &key)) else {
            return false;
        };
        let Some(text) = utf8(folder, &key) else {
            return false;
        };
        return text_names_ground(&text) || children_name_ground(parts, folder, &key, seen);
    }
    let Some(id) = part_id(ty) else {
        return false;
    };
    if !seen.insert(id.to_string()) {
        return false;
    }
    let Some(child) = parts.get(id) else {
        return false;
    };
    names_ground(child) || children_name_ground(parts, child, "part.jsonc", seen)
}

fn child_types(text: &str) -> Vec<String> {
    let Ok(Some(JsonValue::Object(props))) = parse_to_value(text, &ParseOptions::default()) else {
        return Vec::new();
    };
    let mut types = Vec::new();
    for (key, value) in props {
        if key != "parts" {
            continue;
        }
        let JsonValue::Object(children) = value else {
            return Vec::new();
        };
        for (_, child) in children {
            let JsonValue::Object(fields) = child else {
                continue;
            };
            for (key, value) in fields {
                if key == "type"
                    && let JsonValue::String(ty) = value
                {
                    types.push(ty.into_owned());
                }
            }
        }
    }
    types
}

fn part_id(ty: &str) -> Option<&str> {
    let id = match ty.strip_prefix('@').and_then(|rest| rest.split_once('/')) {
        Some((registry, id)) if orbe_scene::is_id(registry) => id,
        Some(_) => return None,
        None => ty,
    };
    orbe_scene::is_id(id).then_some(id)
}

/// `rel` joined onto `dir`, when every `..` stays inside the part folder.
fn path_inside(dir: &str, rel: &str) -> Option<String> {
    let mut stack: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for part in Path::new(rel).components() {
        match part {
            Component::CurDir => {}
            Component::Normal(name) => stack.push(name.to_str()?),
            Component::ParentDir => {
                stack.pop()?;
            }
            _ => return None,
        }
    }
    Some(stack.join("/"))
}

fn json_key(folder: &Folder, key: &str) -> Option<String> {
    let json = |name: &str| name.ends_with(".json") || name.ends_with(".jsonc");
    if json(key) && folder.contains_key(key) {
        return Some(key.to_string());
    }
    let nested = if key.is_empty() {
        "part.jsonc".to_string()
    } else {
        format!("{key}/part.jsonc")
    };
    folder.contains_key(&nested).then_some(nested)
}

fn parent_key(file: &str) -> String {
    match file.rfind('/') {
        Some(index) => file[..index].to_string(),
        None => String::new(),
    }
}

fn utf8(folder: &Folder, key: &str) -> Option<String> {
    std::str::from_utf8(folder.get(key)?)
        .ok()
        .map(str::to_string)
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
    let low = param.above.as_ref().and_then(literal_f64);
    let high = param.below.as_ref().and_then(literal_f64);
    let guess = match param.unit.as_deref() {
        Some("F") | Some("H") => 1e-6,
        Some("s") => 1e-3,
        _ => 1000.0,
    };
    stand_in(guess, low, high).to_string()
}

/// A number strictly inside the exclusive bounds. The guess stays when it is already inside.
/// An empty interval has no such number, and the guess is what the finding quotes.
fn stand_in(guess: f64, low: Option<f64>, high: Option<f64>) -> f64 {
    if inside(guess, low, high) {
        return guess;
    }
    match (low, high) {
        (Some(low), Some(high)) if low < high => {
            let mid = low + (high - low) / 2.0;
            if inside(mid, Some(low), Some(high)) {
                mid
            } else {
                let under = below_high(high);
                if inside(under, Some(low), Some(high)) {
                    under
                } else {
                    let over = above_low(low);
                    if inside(over, Some(low), Some(high)) {
                        over
                    } else {
                        guess
                    }
                }
            }
        }
        (Some(low), None) => above_low(low),
        (None, Some(high)) => below_high(high),
        _ => guess,
    }
}

fn inside(value: f64, low: Option<f64>, high: Option<f64>) -> bool {
    value.is_finite()
        && low.is_none_or(|bound| value > bound)
        && high.is_none_or(|bound| value < bound)
}

fn above_low(low: f64) -> f64 {
    low + low.abs() + 1.0
}

fn below_high(high: f64) -> f64 {
    if high == 0.0 {
        -1.0
    } else if high > 0.0 {
        high / 2.0
    } else {
        high - high.abs() - 1.0
    }
}

fn literal_f64(literal: &Literal) -> Option<f64> {
    match literal {
        Literal::Number(raw) => raw.parse().ok(),
        Literal::Name(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{nested_ground, number_text};
    use orbe_scene::{Folder, Literal, Param};
    use std::collections::BTreeMap;

    fn param(unit: &str, low: Option<&str>, high: Option<&str>) -> Param {
        Param {
            name: "voltage".into(),
            unit: Some(unit.into()),
            above: low.map(|raw| Literal::Number(raw.into())),
            below: high.map(|raw| Literal::Number(raw.into())),
            file: None,
            choices: None,
            required: true,
            default: None,
        }
    }

    #[test]
    fn a_stand_in_sits_inside_the_exclusive_bounds() {
        let cases = [
            (None, Some("0"), "-1"),
            (None, Some("-5"), "-11"),
            (None, Some("5"), "2.5"),
            (Some("-10"), Some("-1"), "-5.5"),
            (Some("0"), Some("5"), "2.5"),
            (Some("0"), None, "1000"),
            (Some("2000"), None, "4001"),
            (None, None, "1000"),
        ];
        for (low, high, expect) in cases {
            let text = number_text(&param("V", low, high));
            assert_eq!(text, expect, "bounds {low:?} .. {high:?}");
            let value: f64 = text.parse().unwrap();
            if let Some(low) = low {
                assert!(value > low.parse::<f64>().unwrap(), "{text}");
            }
            if let Some(high) = high {
                assert!(value < high.parse::<f64>().unwrap(), "{text}");
            }
        }
        assert_eq!(number_text(&param("F", None, None)), "0.000001");
        assert_eq!(number_text(&param("H", None, None)), "0.000001");
        assert_eq!(number_text(&param("s", None, None)), "0.001");
    }

    #[test]
    fn a_child_ground_counts_and_a_path_out_of_the_folder_does_not() {
        let mut left: Folder = BTreeMap::new();
        left.insert(
            "part.jsonc".into(),
            br#"{"ground": "a", "parts": {"g": {"type": "../../probe-ground.jsonc"}}}"#.to_vec(),
        );
        let mut twin: Folder = BTreeMap::new();
        twin.insert(
            "part.jsonc".into(),
            br#"{"parts": {"l": {"type": "left"}, "r": {"type": "right"}}}"#.to_vec(),
        );
        twin.insert("inner.jsonc".into(), br#"{"ground": "b"}"#.to_vec());
        let mut boxed: Folder = BTreeMap::new();
        boxed.insert(
            "part.jsonc".into(),
            br#"{"parts": {"n": {"type": "./inner.jsonc"}}}"#.to_vec(),
        );
        boxed.insert("inner.jsonc".into(), br#"{"ground": "b"}"#.to_vec());
        let mut plain: Folder = BTreeMap::new();
        plain.insert(
            "part.jsonc".into(),
            br#"{"parts": {"g": {"type": "../../probe-ground.jsonc"}}}"#.to_vec(),
        );
        let mut parts = BTreeMap::new();
        parts.insert("left".into(), left);
        parts.insert("twin".into(), twin);
        parts.insert("boxed".into(), boxed);
        parts.insert("plain".into(), plain);
        assert!(nested_ground(&parts, "twin"));
        assert!(nested_ground(&parts, "boxed"));
        assert!(!nested_ground(&parts, "left"));
        assert!(!nested_ground(&parts, "plain"));
    }
}
