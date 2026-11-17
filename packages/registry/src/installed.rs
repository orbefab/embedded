//! What `orbe check` runs on a part or Block after a project installs it.
//!
//! The part-file reader and the scene reader have already accepted these folders. This places
//! each part in a scene, and reads each Block's scene, through the reader `orbe check` uses. The
//! parts are the folders being built, so a board's children and a Block's parts resolve from this
//! source. A Block's project is those part folders, the Block's own files and the lock.
//!
//! A part is read as placed, before a user's scene sets it: a required param with no default
//! has no value, so a model that needs one is not compiled, and a scene ground is not needed.
//! A Block's scene sets what it uses, so it is read in full.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use orbe_scene::{Card, Diagnostic, FilePath, Files, Folder, REGISTRY};
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
    registry: &str,
    parts: &BTreeMap<String, Folder>,
    blocks: &BTreeMap<String, Folder>,
    cards: &[Card],
) -> Option<Refusal> {
    let catalog = orbe_scene::catalog_cards();
    let official: BTreeSet<&str> = catalog.iter().map(|card| card.name.as_str()).collect();
    // An id this binary catalogs locks as `@orbefab`, the line an install of that part uses.
    // `read` then runs the lock check, so a sealed folder that differs from this binary is
    // refused the same way that install is. A line this build does not write is not a finding.
    let project = project_files(parts, &lock_text(cards, &official, registry));
    let mut read = Vec::new();
    for card in cards.iter().filter(|card| parts.contains_key(&card.name)) {
        let placed = orbe_scene::read_placed(&part_project(&project, card), Path::new(SCENE), true);
        read.push((Item::Part, placed));
    }
    for (id, folder) in blocks {
        let files = block_project(&project, folder);
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

/// The project with a scene that places the part and sets nothing.
fn part_project(project: &BTreeMap<PathBuf, Vec<u8>>, card: &Card) -> Files {
    let scene = json!({
        "format": current_format(),
        "parts": { "r1": { "type": card.name } },
    });
    let mut text = serde_json::to_string_pretty(&scene).unwrap();
    text.push('\n');
    let mut files = project.clone();
    files.insert(PathBuf::from(SCENE), text.into_bytes());
    Files::memory(files)
}

/// The project an install of the Block makes: the part folders, the Block's files and the lock.
fn block_project(project: &BTreeMap<PathBuf, Vec<u8>>, folder: &Folder) -> Files {
    let mut files = project.clone();
    for (rel, bytes) in folder {
        if rel == "block.jsonc" || rel == "parts.lock" || rel.starts_with("parts/") {
            continue;
        }
        files.insert(PathBuf::from(rel), bytes.clone());
    }
    Files::memory(files)
}

fn current_format() -> u32 {
    orbe_scene::FORMATS.iter().copied().max().unwrap_or(1)
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
    use super::within;
    use std::path::Path;

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
