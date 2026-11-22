//! What `orbe check` runs on a part or Block after a project installs it.
//!
//! The part-file reader and the scene reader have already accepted these folders. This places
//! each part in a scene, and reads each Block's scene, through the reader `orbe check` uses. The
//! parts are the folders being built, so a board's children and a Block's parts resolve from this
//! source. A Block's project is those part folders and the Block's own files.
//!
//! A part is read as placed, before a user's scene sets it: a required param with no default
//! has no value, so a body that param writes is not compiled, and a scene ground is not needed.
//! A body it does not write is compiled. A file param, text or blob, is no different: a default
//! is read from the part's folder, and a required one with no default names no file. A Block's
//! scene sets what it uses, so it is read in full.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use orbe_scene::{Card, Diagnostic, FilePath, Files, Folder};
use serde_json::json;

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
    for card in cards.iter().filter(|card| parts.contains_key(&card.name)) {
        let placed =
            orbe_scene::read_placed(&part_project(parts, cards, card), Path::new(SCENE), true);
        read.push((Item::Part, placed));
    }
    for (id, folder) in blocks {
        let files = block_project(parts, cards, folder);
        let placed = orbe_scene::read_placed(&files, Path::new(SCENE), false);
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
    for card in cards {
        if let Some(folder) = parts.get(&card.name) {
            found.extend(orbe_scene::unusable_params(&card.name, folder));
        }
    }
    for (item, placed) in read {
        found.extend(match item {
            Item::Part => placed.findings,
            Item::Block(id) => name_block(id, placed.findings),
        });
    }
    found.sort_by(|a, b| {
        (&a.file, a.line, a.column, a.code, &a.message)
            .cmp(&(&b.file, b.line, b.column, b.code, &b.message))
    });
    found.dedup();
    (!found.is_empty()).then_some(Refusal::Check(found))
}

const SCENE: &str = "scene.jsonc";

/// The project with a scene that places the part and sets nothing. The part's own files are
/// `parts/<id>/`, and each part it uses is nested one folder further down.
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
    let scene = json!({
        "format": current_format(),
        "parts": { "r1": { "type": card.name } },
    });
    let mut text = serde_json::to_string_pretty(&scene).unwrap();
    text.push('\n');
    files.insert(PathBuf::from(SCENE), text.into_bytes());
    Files::memory(files)
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
    Part,
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
                Item::Part => path.site.strip_prefix("r1.").unwrap_or(&path.site),
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

    /// The catalog's ESP32-S3 parts, probed as the build probes them. The chip's `rom` default
    /// is read. `app` and `partitions` have no value and name no file.
    #[test]
    fn the_esp32s3_parts_probe_the_rom_and_leave_app_and_partitions_unset() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let parts = crate::folders(&repo, "parts").unwrap();
        let cards = orbe_scene::cards(&parts).unwrap();
        for (id, rom, file) in [
            ("esp32s3-qfn56", "r1.rom", "parts/esp32s3-qfn56/part.jsonc"),
            (
                "esp32s3-wroom-1-n16r8",
                "r1.u1.rom",
                "parts/esp32s3-wroom-1-n16r8/parts/esp32s3-qfn56/part.jsonc",
            ),
            (
                "esp32s3-devkitc-1",
                "r1.mod.u1.rom",
                "parts/esp32s3-devkitc-1/parts/esp32s3-wroom-1-n16r8/parts/esp32s3-qfn56/part.jsonc",
            ),
        ] {
            let card = cards.iter().find(|card| card.name == id).unwrap();
            let placed = orbe_scene::read_placed(
                &part_project(&parts, &cards, card),
                Path::new(SCENE),
                true,
            );
            assert_eq!(placed.findings, [], "{id}");
            let read: Vec<(&str, &str, &str)> = placed
                .file_paths
                .iter()
                .map(|p| (p.file.as_str(), p.site.as_str(), p.path.as_str()))
                .collect();
            assert_eq!(read, [(file, rom, "esp32s3_rev0_rom.elf")], "{id}");
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
