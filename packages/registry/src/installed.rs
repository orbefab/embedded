//! What `orbe check` runs on a part or Block after a project installs it.
//!
//! The part-file reader and the scene reader have already accepted these folders. This places
//! each part in a scene, and reads each Block's scene, through the reader `orbe check` uses. The
//! parts are the folders being built, so a board's children and a Block's parts resolve from this
//! source. A Block's project is those part folders and the Block's own files.
//!
//! A part has no scene of its own. The build places it alone and sets each required param to
//! its example. A required param with none is a finding at the key of the param that declares
//! it. An example that names a file names the copy already in the nested part folder.
//! When the part names no ground, and no part inside it does, the scene adds a voltage source
//! and names one of its pins as ground. When a part in that tree names a ground, those grounds
//! must agree. A Block's scene sets what it uses, so it is read as the Block wrote it.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use orbe_scene::{Card, Code, Diagnostic, FilePath, Files, Folder, Literal};
use serde_json::{Map, Value, json};

/// Each part placed, and each Block scene, as `orbe check` reads it after an install. A file
/// param that names a path outside its part's or its Block's folder is
/// [`Refusal::Outside`], since an install does not copy it. Otherwise the findings `orbe check`
/// would print, with file paths that name the source folder.
pub(crate) enum Refusal {
    Outside(Vec<String>),
    Check(Vec<Diagnostic>),
}

pub(crate) fn refusal(
    parts: &BTreeMap<String, Folder>,
    blocks: &BTreeMap<String, Folder>,
    cards: &[Card],
) -> Option<Refusal> {
    let mut read = Vec::new();
    let mut gaps = Vec::new();
    for card in cards.iter().filter(|card| parts.contains_key(&card.name)) {
        let missing = example_gaps(parts, card);
        let covered: Vec<String> = missing.iter().map(|gap| gap.name.clone()).collect();
        gaps.extend(missing.into_iter().map(|gap| gap.diagnostic));
        let placed = orbe_scene::read_paths(&part_project(parts, cards, card), Path::new(SCENE));
        read.push((Item::Part(covered), placed));
    }
    for (id, folder) in blocks {
        let files = block_project(parts, cards, folder);
        let placed = orbe_scene::read_paths(&files, Path::new(SCENE));
        read.push((Item::Block(id), placed));
    }

    let mut away: Vec<String> = read
        .iter()
        .flat_map(|(item, placed)| {
            placed
                .file_paths
                .iter()
                .filter_map(|path| outside(path, item))
        })
        .collect();
    if !away.is_empty() {
        away.sort();
        away.dedup();
        return Some(Refusal::Outside(away));
    }

    let mut found = Vec::new();
    for (item, placed) in read {
        match item {
            Item::Part(covered) => found.extend(
                placed
                    .findings
                    .into_iter()
                    .filter(|diagnostic| !example_covered(diagnostic, &covered)),
            ),
            Item::Block(id) => found.extend(name_block(id, placed.findings)),
        }
    }
    found.extend(gaps);
    found.sort_by(|a, b| {
        (&a.file, a.line, a.column, a.code, &a.message)
            .cmp(&(&b.file, b.line, b.column, b.code, &b.message))
    });
    found.dedup();
    (!found.is_empty()).then_some(Refusal::Check(found))
}

const SCENE: &str = "scene.jsonc";

/// A required param with no example, named as the scene would ask for it, and the finding at
/// the key of the param that declares it.
struct Gap {
    name: String,
    diagnostic: Diagnostic,
}

/// Each required param of `card` that has no example, when the key of the param that declares
/// it can be read. A param that only passes its value on is reported on the part that gives it
/// a unit, choices or file.
fn example_gaps(parts: &BTreeMap<String, Folder>, card: &Card) -> Vec<Gap> {
    card.params
        .iter()
        .filter(|param| param.required && param.example.is_none())
        .filter_map(|param| example_gap(parts, &card.name, &param.name))
        .collect()
}

fn example_gap(parts: &BTreeMap<String, Folder>, root: &str, param: &str) -> Option<Gap> {
    let (id, name) = declaring(parts, root, param)?;
    let text = part_text(parts.get(&id)?)?;
    let at = param_key(&text, &name)?;
    let (line, column) = line_col(&text, at);
    Some(Gap {
        name: param.to_string(),
        diagnostic: Diagnostic {
            code: Code::Param,
            file: format!("parts/{id}/part.jsonc"),
            line,
            column,
            message: format!("required param `{name}` has no `example`"),
            hint: "add \"example\" with a value a scene could set".into(),
        },
    })
}

/// The part that declares `param`, and that param's name there. A param that only passes its
/// value on is followed until a part gives it a unit, choices or file. The walk stops at 16 steps.
fn declaring(
    parts: &BTreeMap<String, Folder>,
    root: &str,
    param: &str,
) -> Option<(String, String)> {
    let mut id = root.to_string();
    let mut name = param.to_string();
    for _ in 0..16 {
        let value = parts.get(&id).and_then(part_value)?;
        if declares_param(&value, &name) {
            return Some((id, name));
        }
        let (child, child_param) = passed_to(&value, &name)?;
        id = child;
        name = child_param;
    }
    None
}

fn declares_param(part: &Value, name: &str) -> bool {
    let Some(decl) = part
        .get("params")
        .and_then(|node| node.get(name))
        .and_then(|node| node.as_object())
    else {
        return false;
    };
    decl.contains_key("file") || decl.contains_key("unit") || decl.contains_key("choices")
}

fn part_text(folder: &Folder) -> Option<String> {
    let text = String::from_utf8_lossy(folder.get("part.jsonc")?);
    Some(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

/// The byte offset of `name`'s key in `params`, including its quotes.
fn param_key(text: &str, name: &str) -> Option<usize> {
    let parsed = jsonc_parser::parse_to_ast(
        text,
        &jsonc_parser::CollectOptions::default(),
        &json_options(),
    )
    .ok()?;
    let root = parsed.value?;
    let params = root.as_object()?.get("params")?.value.as_object()?;
    Some(name_start(&params.get(name)?.name))
}

fn name_start(name: &jsonc_parser::ast::ObjectPropName<'_>) -> usize {
    match name {
        jsonc_parser::ast::ObjectPropName::String(lit) => lit.range.start,
        jsonc_parser::ast::ObjectPropName::Word(lit) => lit.range.start,
    }
}

/// The 1-based line and column of `offset`, the column counted in characters.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[line_start..].chars().count() + 1)
}

/// JSON as the scene reader takes it: comments and trailing commas, and nothing looser.
fn json_options() -> jsonc_parser::ParseOptions {
    jsonc_parser::ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

/// The in-memory scene's `` `r1` needs param `` line for a param whose missing example was
/// reported on the part file. Other findings stay, including one the scene still reports when
/// the key could not be read.
fn example_covered(diagnostic: &Diagnostic, names: &[String]) -> bool {
    diagnostic.file == SCENE
        && names
            .iter()
            .any(|name| diagnostic.message == format!("`r1` needs param `{name}`"))
}

/// The project with a scene that places the part and sets each required param to its example.
/// The part's own files are `parts/<id>/`, and each part it uses is nested one folder further
/// down. When the part and the parts inside it name no ground, the scene adds a voltage source.
fn part_project(parts: &BTreeMap<String, Folder>, cards: &[Card], card: &Card) -> Files {
    let mut files = BTreeMap::new();
    let mut stack = Vec::new();
    nest(
        &mut files,
        PathBuf::from("parts").join(&card.name),
        &card.name,
        parts,
        cards,
        &mut stack,
    );
    let mut instance = Map::new();
    instance.insert("type".into(), json!(card.name));
    for (name, value) in scene_params(parts, card) {
        instance.insert(name, value);
    }
    let mut placed = Map::new();
    placed.insert("r1".into(), Value::Object(instance));
    let mut scene = Map::new();
    scene.insert("format".into(), json!(current_format()));
    if !tree_names_ground(parts, &card.name, &mut Vec::new()) {
        files.insert(PathBuf::from("probe-ground.jsonc"), probe_ground());
        placed.insert("g".into(), json!({ "type": "./probe-ground.jsonc" }));
        scene.insert("ground".into(), json!("g.neg"));
    }
    scene.insert("parts".into(), Value::Object(placed));
    let mut text = serde_json::to_string_pretty(&Value::Object(scene)).unwrap();
    text.push('\n');
    files.insert(PathBuf::from(SCENE), text.into_bytes());
    Files::memory(files)
}

/// Each required param's example, as the scene sets it. A file example is the path of the copy
/// in the nested folder of the part that declares the param.
fn scene_params(parts: &BTreeMap<String, Folder>, card: &Card) -> Map<String, Value> {
    let mut params = Map::new();
    for param in card.params.iter().filter(|param| param.required) {
        let Some(example) = &param.example else {
            continue;
        };
        let value = match example {
            Literal::Number(raw) => {
                serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
            }
            Literal::Name(name) if param.file.is_some() => {
                Value::String(nested_example(parts, &card.name, &param.name, name))
            }
            Literal::Name(name) => Value::String(name.clone()),
        };
        params.insert(param.name.clone(), value);
    }
    params
}

/// `parts/<id>/…/<file>`, walking a param that only passes its value on until the part that
/// declares it. The walk stops at 16 steps.
fn nested_example(parts: &BTreeMap<String, Folder>, root: &str, param: &str, file: &str) -> String {
    let mut id = root.to_string();
    let mut name = param.to_string();
    let mut prefix = format!("parts/{root}");
    for _ in 0..16 {
        let Some(value) = parts.get(&id).and_then(part_value) else {
            break;
        };
        if owns_param(&value, &name) {
            return format!("{prefix}/{file}");
        }
        let Some((child, child_param)) = passed_to(&value, &name) else {
            break;
        };
        prefix = format!("{prefix}/parts/{child}");
        id = child;
        name = child_param;
    }
    format!("{prefix}/{file}")
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
        if !orbe_scene::is_id(ty) {
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

/// Whether `id` or a part inside it names a ground. A scene ground would hide those.
fn tree_names_ground(parts: &BTreeMap<String, Folder>, id: &str, stack: &mut Vec<String>) -> bool {
    if stack.iter().any(|seen| seen == id) {
        return false;
    }
    let Some(folder) = parts.get(id) else {
        return false;
    };
    if part_value(folder)
        .and_then(|value| {
            value
                .get("ground")
                .and_then(|node| node.as_str())
                .map(|_| ())
        })
        .is_some()
    {
        return true;
    }
    stack.push(id.to_string());
    let named = child_ids(folder)
        .iter()
        .any(|child| tree_names_ground(parts, child, stack));
    stack.pop();
    named
}

fn part_value(folder: &Folder) -> Option<Value> {
    let bytes = folder.get("part.jsonc")?;
    let text = String::from_utf8_lossy(bytes);
    jsonc_parser::parse_to_serde_value(&text, &jsonc_parser::ParseOptions::default()).ok()
}

/// A voltage source the scene names as ground when the part names none. It is not a catalog part.
fn probe_ground() -> Vec<u8> {
    let value = json!({
        "format": orbe_scene::PART_FORMATS.iter().copied().max().unwrap_or(1),
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

/// The project an install of the Block makes: each part the scene names at `parts/<id>/`,
/// with that part's own uses nested inside it, and the Block's files except `block.jsonc`.
fn block_project(parts: &BTreeMap<String, Folder>, cards: &[Card], folder: &Folder) -> Files {
    let mut files = BTreeMap::new();
    for (rel, bytes) in folder {
        if rel == "block.jsonc" || rel.starts_with("parts/") {
            continue;
        }
        files.insert(PathBuf::from(rel), bytes.clone());
    }
    if let Some(scene) = folder.get("scene.jsonc") {
        for id in scene_part_ids(scene) {
            let mut stack = Vec::new();
            nest(
                &mut files,
                PathBuf::from("parts").join(&id),
                &id,
                parts,
                cards,
                &mut stack,
            );
        }
    }
    Files::memory(files)
}

fn nest(
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    dest: PathBuf,
    id: &str,
    parts: &BTreeMap<String, Folder>,
    cards: &[Card],
    stack: &mut Vec<String>,
) {
    if stack.iter().any(|seen| seen == id) {
        return;
    }
    let Some(folder) = parts.get(id) else {
        return;
    };
    for (rel, bytes) in folder {
        files.insert(dest.join(rel), bytes.clone());
    }
    let mut deps: Vec<String> = cards
        .iter()
        .find(|card| card.name == id)
        .map(|card| card.uses.keys().cloned().collect())
        .unwrap_or_default();
    for child in child_ids(folder) {
        if !deps.iter().any(|seen| seen == &child) {
            deps.push(child);
        }
    }
    stack.push(id.to_string());
    for dep in deps {
        nest(
            files,
            dest.join("parts").join(&dep),
            &dep,
            parts,
            cards,
            stack,
        );
    }
    stack.pop();
}

/// Catalog ids a part file's `parts` name. `uses` lists the same ids when the part was
/// published; a part that only names them here still nests those copies.
fn child_ids(folder: &Folder) -> Vec<String> {
    let Some(bytes) = folder.get("part.jsonc") else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(bytes);
    let Ok(value) = jsonc_parser::parse_to_serde_value::<serde_json::Value>(
        &text,
        &jsonc_parser::ParseOptions::default(),
    ) else {
        return Vec::new();
    };
    let Some(parts) = value.get("parts").and_then(|node| node.as_object()) else {
        return Vec::new();
    };
    parts
        .values()
        .filter_map(|spec| spec.get("type").and_then(|node| node.as_str()))
        .filter(|ty| orbe_scene::is_id(ty))
        .map(str::to_string)
        .collect()
}

/// Ids the scene names, skipping a `./` or `../` path.
fn scene_part_ids(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let Ok(value) = jsonc_parser::parse_to_serde_value::<serde_json::Value>(
        &text,
        &jsonc_parser::ParseOptions::default(),
    ) else {
        return Vec::new();
    };
    let Some(parts) = value.get("parts").and_then(|node| node.as_object()) else {
        return Vec::new();
    };
    parts
        .values()
        .filter_map(|spec| spec.get("type").and_then(|node| node.as_str()))
        .filter(|ty| !ty.starts_with('.') && !ty.starts_with('@'))
        .map(str::to_string)
        .collect()
}

fn current_format() -> u32 {
    orbe_scene::SCENE_FORMATS.iter().copied().max().unwrap_or(1)
}

enum Item<'a> {
    /// Required params of this part whose missing example was reported on a part file.
    Part(Vec<String>),
    Block(&'a str),
}

/// A sentence for a file param path read from outside the folder of the item that wrote it. A
/// path written in `parts/<id>/` must stay in that folder, and a Block's must stay in the
/// Block's. An absolute path never does.
fn outside(path: &FilePath, item: &Item) -> Option<String> {
    let mut written = path.file.split('/');
    let (home, at, site, what) = match (written.next(), written.next()) {
        (Some("parts"), Some(id)) => {
            let site = match item {
                // `r1` is the generated scene's, not a name the author wrote.
                Item::Part(_) => path.site.strip_prefix("r1.").unwrap_or(&path.site),
                Item::Block(_) => &path.site,
            };
            (
                PathBuf::from("parts").join(id),
                path.file.clone(),
                site,
                "a part's files must ship in its folder",
            )
        }
        _ => {
            let Item::Block(id) = item else {
                return None;
            };
            (
                PathBuf::new(),
                format!("blocks/{id}/{}", path.file),
                path.site.as_str(),
                "a Block's files must ship in its folder",
            )
        }
    };
    let folder = Path::new(&path.file).parent().unwrap_or(Path::new(""));
    if within(&folder.join(&path.path), &home) {
        return None;
    }
    Some(format!(
        "{at}:{}:{}: `{site}` names `{}`, outside the folder an install copies; {what}",
        path.line, path.column, path.path
    ))
}

/// `path` stays inside `home` once each `.` and `..` is taken in order.
fn within(path: &Path, home: &Path) -> bool {
    let mut stack: Vec<&std::ffi::OsStr> = Vec::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::Normal(name) => stack.push(name),
            Component::ParentDir => {
                if stack.pop().is_none() {
                    return false;
                }
            }
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    stack.iter().collect::<PathBuf>().starts_with(home)
}

fn name_block(id: &str, found: Vec<Diagnostic>) -> Vec<Diagnostic> {
    found
        .into_iter()
        .map(|mut diagnostic| {
            if diagnostic.file == SCENE {
                diagnostic.file = format!("blocks/{id}/{SCENE}");
            }
            diagnostic
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{SCENE, part_project, within};
    use std::path::{Path, PathBuf};

    /// The catalog's ESP32-S3 parts, read as the build reads them. The chip's `rom` default
    /// is read from the part file. `app` and `partitions` are the examples in the chip's folder,
    /// set from the scene.
    #[test]
    fn the_esp32s3_parts_read_the_rom_and_the_app_and_partition_examples() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let parts = crate::folders(&repo, "parts").unwrap();
        let cards = orbe_scene::cards(&parts).unwrap();
        for (id, rom_file, rom_site, app) in [
            (
                "esp32s3-qfn56",
                "parts/esp32s3-qfn56/part.jsonc",
                "r1.rom",
                "parts/esp32s3-qfn56",
            ),
            (
                "esp32s3-wroom-1-n16r8",
                "parts/esp32s3-wroom-1-n16r8/parts/esp32s3-qfn56/part.jsonc",
                "r1.u1.rom",
                "parts/esp32s3-wroom-1-n16r8/parts/esp32s3-qfn56",
            ),
            (
                "esp32s3-devkitc-1",
                "parts/esp32s3-devkitc-1/parts/esp32s3-wroom-1-n16r8/parts/esp32s3-qfn56/part.jsonc",
                "r1.mod.u1.rom",
                "parts/esp32s3-devkitc-1/parts/esp32s3-wroom-1-n16r8/parts/esp32s3-qfn56",
            ),
        ] {
            let card = cards.iter().find(|card| card.name == id).unwrap();
            let placed =
                orbe_scene::read_paths(&part_project(&parts, &cards, card), Path::new(SCENE));
            assert_eq!(placed.findings, [], "{id}");
            let read: Vec<(String, String, String)> = placed
                .file_paths
                .iter()
                .map(|p| (p.file.clone(), p.site.clone(), p.path.clone()))
                .collect();
            let expect = [
                (
                    rom_file.to_string(),
                    rom_site.to_string(),
                    "esp32s3_rev0_rom.elf".to_string(),
                ),
                (
                    "scene.jsonc".to_string(),
                    "r1.app".to_string(),
                    format!("{app}/hello_world.bin"),
                ),
                (
                    "scene.jsonc".to_string(),
                    "r1.partitions".to_string(),
                    format!("{app}/partition-table.bin"),
                ),
            ];
            assert_eq!(read, expect, "{id}");
        }
    }

    #[test]
    fn a_path_stays_within_its_folder_only_while_every_walk_does() {
        let lamp = Path::new("parts/lamp");
        assert!(within(Path::new("parts/lamp/fw.hex"), lamp));
        assert!(within(Path::new("parts/lamp/./sub/../fw.hex"), lamp));
        assert!(!within(Path::new("parts/lamp/../other/fw.hex"), lamp));
        assert!(!within(Path::new("parts/lamp/../../fw.hex"), lamp));
        assert!(!within(Path::new("/home/fw.hex"), lamp));
        assert!(within(Path::new("fw.hex"), Path::new("")));
        assert!(!within(Path::new("../fw.hex"), Path::new("")));
        assert!(!within(Path::new("sketch/../../fw.hex"), Path::new("")));
    }
}
