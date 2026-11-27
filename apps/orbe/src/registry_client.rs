//! `orbe add`, `orbe search`, `orbe part` and `orbe blocks`. They read the registry at
//! `https://orbefab.com/orbe/r/{name}.json`. `ORBE_REGISTRY` replaces that template, which
//! is how a test points at a registry built from the same tree. `{name}` is replaced with
//! `index` and with nothing else. Every other URL is resolved against the document that holds it.
//! An `https` URL is fetched by running `curl`. A `file` URL is read from disk. Records and
//! files that do not depend on each other are fetched together, at most 8 at a time. A file is
//! fetched only after its path and `url` have been checked. A record and a file are cached by
//! their SHA-256 and checked against the hash and size named for them.
//! The index is fetched on every command. Each URL is fetched at most once in one command.
//!
//! The copy written into the project is the copy orbe reads. The part reader runs on each
//! fetched part before anything is written, and a part this binary cannot read is not written.
//! Nothing here compares a written copy with the registry again. `check`, `open`, `test` and
//! `show` do not use this module.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use orbe_registry::{Index, Item, Kind, Row};
use orbe_scene::{
    BlockCard, Card, CardFile, Code, PlugPart, Version, breadboard_line, is_id, shell_word,
};
use serde_json::{Value, json};

use crate::part;

use orbe_session::{EXIT_FAILED as FAILED, EXIT_IO as IO, EXIT_USAGE as USAGE};
const NETWORK: &str = "check the network and run the command again";
const MISMATCH: &str = "the file does not match its record";
const TEMPLATE: &str = "https://orbefab.com/orbe/r/{name}.json";
/// A response larger than this is refused before it is kept. The largest part file is well under it.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const REDIRECTS: &str = "3";
/// What `orbe add` prints when a fetched part is newer than this binary reads.
const UPDATE: &str = "curl -fsSL https://orbefab.com/orbe/install.sh | sh";
/// How many registry URLs are fetched at once. An `https` URL is fetched by running `curl`.
const FETCH_LIMIT: usize = 8;

pub struct Done {
    pub text: String,
    pub json: Value,
}

#[derive(Clone, Debug)]
pub struct Fail {
    pub code: &'static str,
    pub message: String,
    pub hint: String,
    pub exit: u8,
}

pub enum Outcome {
    Done(Done),
    Fail(Fail),
}

struct Client {
    index_url: String,
    /// Record and file bytes by their SHA-256, so one blob used twice is downloaded once.
    /// A record is checked against its index row, and a file against its record.
    files: BTreeMap<String, Vec<u8>>,
    /// A response by the URL that was requested, and by the URL a redirect landed on.
    /// The stored URL is the one the fetch landed on, so a later read still names it.
    pages: BTreeMap<String, Got>,
    /// A URL this command already failed to fetch, so it is not fetched again.
    failed: BTreeMap<String, Fail>,
}

struct Node {
    files: BTreeMap<String, Vec<u8>>,
    /// Dependency ids, at the version the record names. The bytes live in `Forest::nodes`.
    uses: Vec<(String, Version)>,
}

/// One node per id and version. A named part is written at its highest version, and a board
/// that uses an older one keeps that older copy in its own folder.
struct Forest {
    nodes: BTreeMap<String, BTreeMap<String, Node>>,
}

pub fn add(dir: Option<&Path>, ids: &[String]) -> Outcome {
    let Ok(ids) = check_ids(ids) else {
        return Outcome::Fail(ids_err(ids));
    };
    if let Some(fail) = not_a_project(dir, &ids) {
        return Outcome::Fail(fail);
    }
    let named = dir.unwrap_or(Path::new("."));
    // The folder the user named, with a symlink followed and `..` resolved. A link inside
    // that folder is still refused. Hints keep the path the user typed.
    let root = match named.canonicalize() {
        Ok(path) => path,
        Err(error) => return Outcome::Fail(io_at(named, error)),
    };
    if let Err(fail) = clear_staging(&root) {
        return Outcome::Fail(fail);
    }
    let mut client = Client::new();
    let index = match client.index() {
        Ok(index) => index,
        Err(fail) => return Outcome::Fail(fail),
    };
    match install(&mut client, &index, &root, dir, &ids) {
        Ok(done) => Outcome::Done(done),
        Err(fail) => Outcome::Fail(fail),
    }
}

pub fn show_part(id: &str) -> Outcome {
    if let Some(fail) = one_id(id, "part") {
        return Outcome::Fail(fail);
    }
    let mut client = Client::new();
    let index = match client.index() {
        Ok(index) => index,
        Err(fail) => return Outcome::Fail(fail),
    };
    match part_card(&mut client, &index, id) {
        Ok(done) => Outcome::Done(done),
        Err(fail) => Outcome::Fail(fail),
    }
}

pub fn search(words: &[String]) -> Outcome {
    let mut client = Client::new();
    let index = match client.index() {
        Ok(index) => index,
        Err(fail) => return Outcome::Fail(fail),
    };
    match search_index(&index, words) {
        Ok(done) => Outcome::Done(done),
        Err(fail) => Outcome::Fail(fail),
    }
}

pub fn blocks() -> Outcome {
    let mut client = Client::new();
    let index = match client.index() {
        Ok(index) => index,
        Err(fail) => return Outcome::Fail(fail),
    };
    let rows = match highest(&index) {
        Ok(rows) => rows,
        Err(fail) => return Outcome::Fail(fail),
    };
    let mut listed: Vec<&Row> = rows
        .values()
        .filter(|row| row.kind == Kind::Block)
        .copied()
        .collect();
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    let text_rows: Vec<(String, String, Option<String>)> = listed
        .iter()
        .map(|row| (row.name.clone(), row.title.clone(), row.description.clone()))
        .collect();
    let json_rows: Vec<Value> = listed.iter().map(|row| row_json(row)).collect();
    Outcome::Done(Done {
        text: part::blocks_text(&text_rows),
        json: json!({ "ok": true, "data": { "blocks": json_rows } }),
    })
}

fn ids_err(ids: &[String]) -> Fail {
    check_ids(ids).expect_err("the caller already found an error")
}

fn check_ids(ids: &[String]) -> Result<Vec<String>, Fail> {
    if ids.is_empty() {
        return Err(fail(
            "usage",
            "name a part or a Block".into(),
            "orbe add led-5mm",
            USAGE,
        ));
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in ids {
        let id = parse_add_arg(raw)?;
        if !seen.insert(id.clone()) {
            return Err(fail("usage", format!("`{id}` is named twice"), "", USAGE));
        }
        out.push(id);
    }
    Ok(out)
}

/// A bare id. `@scope/id` and `id@version` are usage errors: this orbe reads `@orbefab` only,
/// and it installs the latest version.
fn parse_add_arg(raw: &str) -> Result<String, Fail> {
    if let Some(rest) = raw.strip_prefix('@') {
        let bare = rest.split_once('/').map(|(_, id)| id).unwrap_or(rest);
        return Err(fail(
            "usage",
            "this orbe reads the @orbefab registry only".into(),
            format!("orbe add {bare}"),
            USAGE,
        ));
    }
    if let Some((id, _)) = raw.split_once('@') {
        return Err(fail(
            "usage",
            format!("`{raw}` names a version, and `orbe add` installs the latest"),
            id,
            USAGE,
        ));
    }
    if !is_id(raw) {
        return Err(fail(
            "usage",
            format!("`{raw}` is not a part id"),
            "an id is kebab case, such as `led-5mm`",
            USAGE,
        ));
    }
    Ok(raw.to_string())
}

fn one_id(raw: &str, command: &str) -> Option<Fail> {
    if let Some(rest) = raw.strip_prefix('@') {
        let bare = rest.split_once('/').map(|(_, id)| id).unwrap_or(rest);
        return Some(fail(
            "usage",
            "this orbe reads the @orbefab registry only".into(),
            format!("orbe {command} {bare}"),
            USAGE,
        ));
    }
    if let Some((id, _)) = raw.split_once('@') {
        return Some(fail(
            "usage",
            format!("`{raw}` names a version, and `orbe {command}` shows the latest"),
            id,
            USAGE,
        ));
    }
    // A word that is not an id can still name one part, such as `1n4007` for `diode-1n4007`.
    // An unknown word is `unknown_part`, with the near names.
    None
}

/// With no `-C`, a folder inside another project must itself be one. A folder with no project
/// above it is where a new project starts. A folder `-C` names is trusted, and has to be a folder.
fn not_a_project(dir: Option<&Path>, ids: &[String]) -> Option<Fail> {
    if let Some(dir) = dir {
        return (!dir.is_dir()).then(|| {
            fail(
                "not_a_project",
                format!("`{}` is not a folder", dir.display()),
                "",
                FAILED,
            )
        });
    }
    if is_project(Path::new(".")) {
        return None;
    }
    let above = project_above(Path::new("."))?;
    let hint = format!("run `{}`", command(Some(&above), ids));
    Some(fail(
        "not_a_project",
        "the current folder is not an orbe project. it has no scene.jsonc and no parts folder"
            .into(),
        hint,
        FAILED,
    ))
}

fn is_project(dir: &Path) -> bool {
    dir.join("scene.jsonc").is_file() || dir.join("parts").is_dir()
}

fn project_above(root: &Path) -> Option<PathBuf> {
    let mut dir = root.to_path_buf();
    loop {
        dir = up(&dir);
        if is_project(&dir) {
            return Some(dir);
        }
        dir.canonicalize().ok()?.parent()?;
    }
}

fn up(dir: &Path) -> PathBuf {
    if dir == Path::new(".") {
        return PathBuf::from("..");
    }
    match dir.components().next_back() {
        Some(Component::Normal(_)) => match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        },
        _ => dir.join(".."),
    }
}

fn install(
    client: &mut Client,
    index: &Index,
    root: &Path,
    dir: Option<&Path>,
    ids: &[String],
) -> Result<Done, Fail> {
    let rows = highest(index)?;
    let mut chosen = Vec::new();
    for id in ids {
        let Some(row) = rows.get(id).copied() else {
            if let Some(fail) = needs_newer(index, id) {
                return Err(fail);
            }
            return Err(unknown(id, &rows, false));
        };
        chosen.push(row);
    }
    let blocks: Vec<&&Row> = chosen
        .iter()
        .filter(|row| row.kind == Kind::Block)
        .collect();
    if let Some(block) = blocks.first()
        && chosen.len() != 1
    {
        return Err(fail(
            "usage",
            format!(
                "`{}` is a Block, and `orbe add` installs a Block on its own",
                block.name
            ),
            command(dir, std::slice::from_ref(&block.name)),
            USAGE,
        ));
    }
    if chosen.len() == 1 && chosen[0].kind == Kind::Block {
        return install_block(client, index, root, dir, chosen[0]);
    }
    let existing = existing_parts(root, ids);
    if !existing.is_empty() {
        return Err(part_exists(dir, ids, &existing));
    }
    let records = client.row_blobs(&chosen);
    client.prefetch(&records);
    client.prefetch_level(index, &chosen);
    let mut forest = Forest {
        nodes: BTreeMap::new(),
    };
    let mut versions = BTreeMap::new();
    for id in ids {
        let version = fetch_part(client, index, id, None, &mut Vec::new(), &mut forest)?;
        versions.insert(id.clone(), version);
    }
    refuse_unreadable(&forest)?;
    place_all(root, dir, ids, &versions, &forest)?;
    let wrote: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "id": id, "version": versions[id], "path": format!("parts/{id}") }))
        .collect();
    let mut lines = Vec::new();
    for id in ids {
        lines.push(format!("wrote parts/{id} {}", versions[id]));
        if let Some(note) = newer_note(index, id, &versions[id]) {
            lines.push(note);
        }
    }
    Ok(Done {
        text: ended(lines),
        json: json!({ "ok": true, "data": { "wrote": wrote } }),
    })
}

fn install_block(
    client: &mut Client,
    index: &Index,
    root: &Path,
    dir: Option<&Path>,
    row: &Row,
) -> Result<Done, Fail> {
    let mut records = Vec::new();
    if let Some(blob) = client.blob_for_row(row) {
        records.push(blob);
    }
    client.prefetch(&records);
    let (url, item) = client.record(row)?;
    let Item::Block(block) = item else {
        return Err(registry(
            &url,
            "is a part, and the index row says a Block",
            "",
        ));
    };
    require_same(&url, &block.head.name, &block.head.version, row)?;
    let block_row = [row];
    client.prefetch_level(index, &block_row);
    let files = client.folder(&url, &block.head)?;
    let scene = files
        .get("scene.jsonc")
        .ok_or_else(|| registry(&url, "has no scene.jsonc", ""))?;
    let mut forest = Forest {
        nodes: BTreeMap::new(),
    };
    let mut versions = BTreeMap::new();
    let mut stack = Vec::new();
    let dep_rows: Vec<&Row> = block
        .head
        .registry_dependencies
        .iter()
        .filter_map(|dep| {
            if dep.registry.is_some() {
                None
            } else {
                row_at(index, &dep.id, dep.version).ok()
            }
        })
        .collect();
    client.prefetch_level(index, &dep_rows);
    for dep in &block.head.registry_dependencies {
        if dep.registry.is_some() {
            return Err(scoped_dep(&url, dep));
        }
        let version = fetch_part(
            client,
            index,
            &dep.id,
            Some(dep.version),
            &mut stack,
            &mut forest,
        )?;
        if let Some(have) = versions.get(&dep.id)
            && have != &version
        {
            return Err(registry(
                &url,
                &format!("`{}` is required at `{have}` and at `{version}`", dep.id),
                "",
            ));
        }
        versions.insert(dep.id.clone(), version);
    }
    let part_ids = orbe_scene::catalog_part_ids(&String::from_utf8_lossy(scene));
    for id in &part_ids {
        if !versions.contains_key(id) {
            return Err(registry(
                &url,
                &format!("names `{id}` in its scene, and its record does not depend on `{id}`"),
                "",
            ));
        }
    }
    let owned: Vec<(String, Vec<u8>)> = files
        .iter()
        .filter(|(path, _)| *path != "block.jsonc" && !path.starts_with("parts/"))
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect();
    let kept = block_conflicts(root, dir, &block.head.name, &owned)?;
    let existing = existing_parts(root, &part_ids);
    if !existing.is_empty() {
        return Err(part_exists(
            dir,
            std::slice::from_ref(&block.head.name),
            &existing,
        ));
    }
    refuse_unreadable(&forest)?;
    place_all(root, dir, &part_ids, &versions, &forest)?;
    let wrote_files = match write_block_files(root, &owned, &kept) {
        Ok(wrote) => wrote,
        Err(mut failed) => {
            let mut names = failed.written;
            names.extend(part_ids.iter().map(|id| format!("parts/{id}")));
            if !names.is_empty() {
                failed.error.message = format!(
                    "{}. remove {} and run `{}`",
                    failed.error.message,
                    name_list(&names),
                    command(dir, std::slice::from_ref(&block.head.name))
                );
            }
            return Err(failed.error);
        }
    };
    let mut lines = Vec::new();
    let mut written = Vec::new();
    let mut kept_paths = Vec::new();
    for file in &wrote_files {
        let verb = if file.wrote { "wrote" } else { "kept" };
        lines.push(format!("{verb} {}", file.path));
        if file.wrote {
            written.push(file.path.clone());
        } else {
            kept_paths.push(file.path.clone());
        }
    }
    let mut wrote = Vec::new();
    for id in &part_ids {
        let version = &versions[id];
        lines.push(format!("wrote parts/{id} {version}"));
        wrote.push(json!({ "id": id, "version": version, "path": format!("parts/{id}") }));
    }
    let next = next_command(dir);
    lines.push(format!("installed the Block `{}`. {next}", block.head.name));
    Ok(Done {
        text: ended(lines),
        json: json!({
            "ok": true,
            "data": {
                "block": block.head.name,
                "files": { "written": written, "kept": kept_paths },
                "wrote": wrote,
                "next": next,
            }
        }),
    })
}

fn part_card(client: &mut Client, index: &Index, id: &str) -> Result<Done, Fail> {
    let rows = highest(index)?;
    let row = if let Some(row) = rows.get(id) {
        *row
    } else {
        let names = listed_names(&rows);
        let matches = segments(id, &names);
        if matches.len() == 1 {
            *rows.get(&matches[0]).expect("the match is a row")
        } else {
            return Err(unknown(id, &rows, true));
        }
    };
    let (url, item) = client.record(row)?;
    match item {
        Item::Part(part) => {
            require_same(&url, &part.head.name, &part.head.version, row)?;
            let card = card_of(*part)?;
            let breadboard = breadboard(client, index, &card)?;
            let data = serde_json::to_value(&card).expect("a card serializes");
            Ok(Done {
                text: part::text(&card, breadboard.as_deref()),
                json: json!({ "ok": true, "data": data }),
            })
        }
        Item::Block(block) => {
            require_same(&url, &block.head.name, &block.head.version, row)?;
            let card = block_card_of(*block)?;
            let data = serde_json::to_value(&card).expect("a card serializes");
            Ok(Done {
                text: part::block_text(&card),
                json: json!({ "ok": true, "data": data }),
            })
        }
    }
}

fn breadboard(client: &mut Client, index: &Index, card: &Card) -> Result<Option<String>, Fail> {
    let mut plugs = BTreeMap::new();
    let mut want = vec![card.name.clone()];
    want.extend(card.uses.keys().cloned());
    want.push("breadboard-400".into());
    want.push("breadboard-830".into());
    if let Ok(best) = highest(index) {
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        for id in &want {
            let Some(row) = best.get(id).copied() else {
                continue;
            };
            if row.kind != Kind::Part {
                continue;
            }
            if seen.insert((row.name.clone(), row.version.clone())) {
                rows.push(row);
            }
        }
        let records = client.row_blobs(&rows);
        client.prefetch(&records);
        let mut files = Vec::new();
        for row in &rows {
            let Some((record_url, item)) = client.item_from_cache(row) else {
                continue;
            };
            let anchors = format!("{}.anchors.json", row.name);
            // A bad path or url is not fetched here. `plug` reports it.
            if check_files(&record_url, &item.head().files).is_ok() {
                files.extend(file_blobs(&record_url, &item.head().files, |path| {
                    path == "part.jsonc" || path == anchors
                }));
            }
        }
        client.prefetch(&files);
    }
    for id in want {
        if let Some(plug) = client.plug(index, &id)? {
            plugs.insert(id, plug);
        }
    }
    Ok(breadboard_line(&card.name, &|id| plugs.get(id).cloned()))
}

fn search_index(index: &Index, words: &[String]) -> Result<Done, Fail> {
    let rows = highest(index)?;
    if words.is_empty() {
        let mut parts = Vec::new();
        let mut blocks = Vec::new();
        for row in rows.values() {
            if !row.listed {
                continue;
            }
            match row.kind {
                Kind::Part => parts.push((
                    row.name.clone(),
                    row.categories
                        .as_ref()
                        .and_then(|list| list.first().cloned()),
                    row.title.clone(),
                )),
                Kind::Block => blocks.push((row.name.clone(), row.title.clone())),
            }
        }
        let groups = part::catalog_groups(&parts, &blocks);
        let json_groups: Vec<Value> = groups
            .iter()
            .map(|(category, items)| {
                json!({
                    "category": category,
                    "items": items.iter().map(|(id, title)| json!({ "id": id, "title": title })).collect::<Vec<_>>(),
                })
            })
            .collect();
        return Ok(Done {
            text: part::catalog_text(&groups),
            json: json!({ "ok": true, "data": { "groups": json_groups } }),
        });
    }
    let lowered: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
    let joined = lowered.join(" ");
    let mut ranked: Vec<(u8, &Row)> = rows
        .values()
        .filter_map(|row| rank_row(row, &lowered, &joined).map(|rank| (rank, *row)))
        .collect();
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.name.cmp(&b.1.name))
            .then(a.1.kind.cmp(&b.1.kind))
    });
    let parts: Vec<&Row> = ranked
        .iter()
        .filter(|(_, row)| row.kind == Kind::Part)
        .map(|(_, row)| *row)
        .collect();
    let blocks: Vec<&Row> = ranked
        .iter()
        .filter(|(_, row)| row.kind == Kind::Block)
        .map(|(_, row)| *row)
        .collect();
    let mut shown: Vec<(String, String)> = parts
        .iter()
        .map(|row| (row.name.clone(), row.title.clone()))
        .collect();
    shown.extend(
        blocks
            .iter()
            .map(|row| (row.name.clone(), row.title.clone())),
    );
    Ok(Done {
        text: part::search_text(&shown, words),
        json: json!({
            "ok": true,
            "data": {
                "parts": parts.iter().map(|row| row_json(row)).collect::<Vec<_>>(),
                "blocks": blocks.iter().map(|row| row_json(row)).collect::<Vec<_>>(),
            }
        }),
    })
}

fn rank_row(row: &Row, words: &[String], joined: &str) -> Option<u8> {
    if !row.listed {
        return None;
    }
    let maker = row.maker.as_deref().unwrap_or("");
    let number = row.number.as_deref().unwrap_or("");
    let description = row.description.as_deref().unwrap_or("");
    let cats = row.categories.as_deref().unwrap_or(&[]);
    let mut fields: Vec<&str> = Vec::with_capacity(5 + cats.len());
    fields.extend_from_slice(&[&row.name, &row.title, maker, number, description]);
    for category in cats {
        fields.push(category);
    }
    let haystack = fields.join("\n").to_lowercase();
    let title_words = words_of(&row.title);
    let name_words = words_of(&row.name);
    let mut norm_tokens: Vec<String> = Vec::new();
    for field in &fields {
        norm_tokens.extend(hyphen_tokens(field));
    }
    norm_tokens.push(compact(&row.name));
    norm_tokens.push(compact(maker));
    norm_tokens.push(compact(number));
    for category in cats {
        norm_tokens.push(compact(category));
        norm_tokens.extend(words_of(category));
    }
    norm_tokens.extend(name_words.iter().cloned());
    norm_tokens.extend(title_words.iter().cloned());
    norm_tokens.extend(pairs_of(&title_words));
    norm_tokens.extend(pairs_of(&name_words));
    let matches = words.iter().all(|word| {
        if haystack.contains(word) {
            return true;
        }
        let c = compact(word);
        !c.is_empty() && norm_tokens.iter().any(|tok| tok.starts_with(&c))
    });
    if !matches {
        return None;
    }
    let cid = compact(&row.name);
    let cjoined = compact(joined);
    if row.name.eq_ignore_ascii_case(joined)
        || words.iter().any(|word| row.name.eq_ignore_ascii_case(word))
        || (!cjoined.is_empty() && cid == cjoined)
        || words.iter().any(|word| {
            let c = compact(word);
            !c.is_empty() && c == cid
        })
    {
        return Some(0);
    }
    let mut whole = name_words.clone();
    whole.extend(title_words.iter().cloned());
    for category in cats {
        whole.extend(words_of(category));
        whole.push(compact(category));
    }
    whole.extend(pairs_of(&title_words));
    whole.extend(pairs_of(&name_words));
    if words.iter().all(|word| {
        if whole.iter().any(|have| have == word) {
            return true;
        }
        let c = compact(word);
        !c.is_empty() && whole.iter().any(|have| have == &c)
    }) {
        Some(1)
    } else {
        Some(2)
    }
}

fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn hyphen_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .filter(|tok| tok.contains('-') || tok.contains('_'))
        .map(compact)
        .filter(|tok| !tok.is_empty())
        .collect()
}

fn pairs_of(words: &[String]) -> Vec<String> {
    words
        .windows(2)
        .map(|pair| format!("{}{}", pair[0], pair[1]))
        .collect()
}

fn row_json(row: &Row) -> Value {
    let mut value = json!({
        "id": row.name,
        "version": row.version,
        "title": row.title,
        "type": match row.kind {
            Kind::Part => "part",
            Kind::Block => "block",
        },
    });
    let object = value.as_object_mut().expect("a row is an object");
    if let Some(description) = &row.description {
        object.insert("description".into(), json!(description));
    }
    if let Some(categories) = &row.categories {
        object.insert("categories".into(), json!(categories));
    }
    if let Some(maker) = &row.maker {
        object.insert("maker".into(), json!(maker));
    }
    if let Some(number) = &row.number {
        object.insert("number".into(), json!(number));
    }
    value
}

fn listed_names(rows: &BTreeMap<String, &Row>) -> Vec<String> {
    rows.iter()
        .filter(|(_, row)| row.listed)
        .map(|(name, _)| name.clone())
        .collect()
}

fn unknown(id: &str, rows: &BTreeMap<String, &Row>, list: bool) -> Fail {
    let names = listed_names(rows);
    let matches = segments(id, &names);
    // `orbe part` lists every close id, and every id a word is a segment of when several match.
    // `orbe add` names the one closest id, the same suggestion a typo in a scene gets.
    let near = if list && matches.len() > 1 {
        matches
    } else if list {
        near_names(id, &names)
    } else {
        closest(id, &names).into_iter().collect()
    };
    let hint = if near.is_empty() {
        "the registry has no Block or part with that id".to_string()
    } else {
        let listed = near
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("did you mean {listed}?")
    };
    fail(
        "unknown_part",
        format!("no Block or part named `{id}`"),
        hint,
        FAILED,
    )
}

fn segments(word: &str, ids: &[String]) -> Vec<String> {
    let want = word.to_lowercase();
    let mut found: Vec<String> = ids
        .iter()
        .filter(|id| words_of(id).iter().any(|segment| segment == &want))
        .cloned()
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The id closest to `name`, when one is close enough to be a likely typo.
fn closest(name: &str, ids: &[String]) -> Option<String> {
    let want = name.to_lowercase();
    ids.iter()
        .filter(|id| id.as_str() != name)
        .map(|id| (strsim::jaro_winkler(&want, &id.to_lowercase()), id.clone()))
        .filter(|(score, _)| *score >= 0.8)
        .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
        .map(|(_, id)| id)
}

fn near_names(name: &str, ids: &[String]) -> Vec<String> {
    let want = name.to_lowercase();
    let mut scored: Vec<(f64, String)> = ids
        .iter()
        .filter(|id| id.as_str() != name)
        .map(|id| (strsim::jaro_winkler(&want, &id.to_lowercase()), id.clone()))
        .filter(|(score, _)| *score >= 0.8)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(8);
    scored.into_iter().map(|(_, id)| id).collect()
}

fn this_orbe() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the package version is three numbers")
}

/// A row with no `orbe` is one every orbe may try. A row that names a newer orbe is skipped.
fn readable(row: &Row) -> bool {
    match &row.orbe {
        None => true,
        Some(orbe) => Version::parse(orbe).is_some_and(|need| need <= this_orbe()),
    }
}

/// The newest version of each item this orbe can read. The index is checked first.
fn highest(index: &Index) -> Result<BTreeMap<String, &Row>, Fail> {
    let url = index_url();
    if let Err(detail) = orbe_registry::validate_index(index) {
        return Err(registry(&url, &detail, ""));
    }
    let mut best: BTreeMap<String, &Row> = BTreeMap::new();
    for row in index.items.iter().filter(|row| readable(row)) {
        let version = Version::parse(&row.version).expect("validate_index checked the version");
        match best.get(&row.name) {
            Some(have)
                if version
                    > Version::parse(&have.version)
                        .expect("validate_index checked the version") =>
            {
                best.insert(row.name.clone(), row);
            }
            Some(_) => {}
            None => {
                best.insert(row.name.clone(), row);
            }
        }
    }
    Ok(best)
}

/// The installed version is not the newest row. The newer one needs a newer orbe.
fn newer_note(index: &Index, id: &str, installed: &str) -> Option<String> {
    let installed = Version::parse(installed)?;
    let newer = index
        .items
        .iter()
        .filter(|row| row.name == id)
        .any(|row| Version::parse(&row.version).is_some_and(|version| version > installed));
    newer.then(|| format!("`{id}` has a newer version. {UPDATE}"))
}

/// Every version of `id` needs a newer orbe. `None` when the index has no such id.
fn needs_newer(index: &Index, id: &str) -> Option<Fail> {
    let mut versions: Vec<&Row> = index.items.iter().filter(|row| row.name == id).collect();
    if versions.is_empty() {
        return None;
    }
    versions.sort_by_key(|row| Version::parse(&row.version));
    let row = versions.last().copied()?;
    Some(fail(
        "part_format",
        format!(
            "`{id}` {} needs a newer orbe. nothing was written",
            row.version
        ),
        UPDATE,
        FAILED,
    ))
}

fn fetch_part(
    client: &mut Client,
    index: &Index,
    id: &str,
    version: Option<Version>,
    stack: &mut Vec<String>,
    forest: &mut Forest,
) -> Result<String, Fail> {
    if stack.iter().any(|seen| seen == id) {
        return Err(registry(
            &client.index_url,
            &format!("`{id}` depends on itself through {}", stack.join(", ")),
            "",
        ));
    }
    let row = match version {
        Some(version) => row_at(index, id, version)?,
        None => highest(index)?
            .get(id)
            .copied()
            .ok_or_else(|| unknown(id, &highest(index).unwrap_or_default(), false))?,
    };
    if row.kind != Kind::Part {
        return Err(registry(
            &client.index_url,
            &format!("`{id}` is a Block, and a part depends on it"),
            "",
        ));
    }
    let stored = row.version.clone();
    if forest
        .nodes
        .get(id)
        .is_some_and(|versions| versions.contains_key(&stored))
    {
        return Ok(stored);
    }
    let (url, item) = client.record(row)?;
    let Item::Part(part) = item else {
        return Err(registry(
            &url,
            "is a Block, and the index row says a part",
            "",
        ));
    };
    require_same(&url, &part.head.name, &part.head.version, row)?;
    let files = client.folder(&url, &part.head)?;
    let mut uses = Vec::new();
    for dep in &part.head.registry_dependencies {
        if dep.registry.is_some() {
            return Err(scoped_dep(&url, dep));
        }
        uses.push((dep.id.clone(), dep.version));
    }
    forest.nodes.entry(id.to_string()).or_default().insert(
        stored.clone(),
        Node {
            files,
            uses: uses.clone(),
        },
    );
    stack.push(id.to_string());
    let child_rows: Vec<&Row> = uses
        .iter()
        .filter_map(|(dep, version)| row_at(index, dep, *version).ok())
        .collect();
    client.prefetch_level(index, &child_rows);
    for (dep, version) in uses {
        fetch_part(client, index, &dep, Some(version), stack, forest)?;
    }
    stack.pop();
    Ok(stored)
}

fn row_at<'a>(index: &'a Index, id: &str, version: Version) -> Result<&'a Row, Fail> {
    let text = version.to_string();
    index
        .items
        .iter()
        .find(|row| row.name == id && row.version == text)
        .ok_or_else(|| registry(&index_url(), &format!("has no `{id}@{text}`"), ""))
}

fn require_same(url: &str, name: &str, version: &str, row: &Row) -> Result<(), Fail> {
    if name == row.name && version == row.version {
        Ok(())
    } else {
        Err(registry(
            url,
            &format!(
                "is `{name}` `{version}`, and its index row is `{}` `{}`",
                row.name, row.version
            ),
            "",
        ))
    }
}

fn scoped_dep(url: &str, dep: &orbe_registry::Dependency) -> Fail {
    registry(
        url,
        &format!("depends on `{dep}`, and this orbe reads the @orbefab registry only"),
        "",
    )
}

fn card_of(part: orbe_registry::PartItem) -> Result<Card, Fail> {
    let orbe_registry::PartItem {
        head,
        real,
        pins,
        params,
        ratings,
        not_simulated,
        docs,
        buy,
        lit,
        dim,
        pressable,
        holes,
    } = part;
    let uses = bare_uses(&head)?;
    let files = card_files(&head.files);
    Ok(Card {
        name: head.name,
        title: head.title,
        version: head.version,
        license: head.license,
        hash: head.hash,
        description: head.description,
        categories: head.categories,
        real,
        pins,
        params,
        ratings,
        not_simulated,
        docs,
        files,
        uses,
        used_by: Vec::new(),
        buy,
        lit,
        dim,
        pressable,
        holes,
    })
}

fn block_card_of(block: orbe_registry::BlockItem) -> Result<BlockCard, Fail> {
    let parts = bare_uses(&block.head)?;
    let files = card_files(&block.head.files);
    Ok(BlockCard {
        name: block.head.name,
        version: block.head.version,
        hash: block.head.hash,
        title: block.head.title,
        description: block.head.description,
        categories: block.head.categories,
        board: block.board,
        license: block.head.license,
        parts,
        files,
    })
}

fn bare_uses(head: &orbe_registry::Head) -> Result<BTreeMap<String, String>, Fail> {
    let mut uses = BTreeMap::new();
    for dep in &head.registry_dependencies {
        if dep.registry.is_some() {
            return Err(scoped_dep(&head.name, dep));
        }
        uses.insert(dep.id.clone(), dep.version.to_string());
    }
    Ok(uses)
}

fn card_files(files: &[orbe_registry::File]) -> Vec<CardFile> {
    files
        .iter()
        .map(|file| CardFile {
            path: file.path.clone(),
            sha256: file.sha256.clone(),
            bytes: file.bytes,
            license: file.license.clone(),
        })
        .collect()
}

fn existing_parts(root: &Path, ids: &[String]) -> Vec<String> {
    ids.iter().filter_map(|id| already(root, id)).collect()
}

fn already(root: &Path, id: &str) -> Option<String> {
    let path = root.join("parts").join(id);
    let meta = fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() {
        return Some(format!("parts/{id} is already a link, not a folder"));
    }
    if !meta.is_dir() {
        return Some(format!("parts/{id} is already a file, not a folder"));
    }
    match read_version(&path) {
        Some(version) => Some(format!("parts/{id} is already {id} {version}")),
        None => Some(format!("parts/{id} is already there")),
    }
}

fn read_version(dir: &Path) -> Option<String> {
    let bytes = fs::read(dir.join("part.jsonc")).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let value =
        jsonc_parser::parse_to_serde_value::<Value>(&text, &jsonc_parser::ParseOptions::default())
            .ok()?;
    let version = value.get("version")?.as_str()?;
    Version::parse(version).map(|version| version.to_string())
}

fn part_exists(dir: Option<&Path>, ids: &[String], lines: &[String]) -> Fail {
    let pronoun = if lines.len() == 1 { "it" } else { "them" };
    fail(
        "part_exists",
        lines.join("\n"),
        format!("remove {pronoun} and run `{}`", command(dir, ids)),
        FAILED,
    )
}

enum Seen {
    Missing,
    Match,
    Differ,
    Link(String),
    Blocked,
}

/// Scene first, so a project that already has one hears `scene_exists` whatever else differs.
fn block_conflicts(
    root: &Path,
    dir: Option<&Path>,
    id: &str,
    files: &[(String, Vec<u8>)],
) -> Result<BTreeSet<String>, Fail> {
    let mut ordered: Vec<&(String, Vec<u8>)> = files
        .iter()
        .filter(|file| file.0 == "scene.jsonc")
        .collect();
    ordered.extend(files.iter().filter(|file| file.0 != "scene.jsonc"));
    let mut kept = BTreeSet::new();
    for (rel, bytes) in ordered {
        match seen(root, rel, bytes).map_err(|error| io_at(&root.join(rel), error))? {
            Seen::Match => {
                kept.insert(rel.clone());
            }
            Seen::Missing => {}
            Seen::Link(at) => return Err(is_a_link(dir, id, &at)),
            Seen::Differ | Seen::Blocked => return Err(in_the_way(dir, id, rel)),
        }
    }
    Ok(kept)
}

fn seen(root: &Path, rel: &str, bytes: &[u8]) -> io::Result<Seen> {
    let mut cur = root.to_path_buf();
    let segments: Vec<&str> = rel.split('/').collect();
    for (index, segment) in segments.iter().enumerate() {
        cur.push(segment);
        let meta = match fs::symlink_metadata(&cur) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Seen::Missing),
            Err(error) => return Err(error),
        };
        if meta.file_type().is_symlink() {
            return Ok(Seen::Link(rel_of(&segments[..=index])));
        }
        let last = index + 1 == segments.len();
        if last {
            if !meta.is_file() {
                return Ok(Seen::Blocked);
            }
            let got = fs::read(&cur)?;
            return Ok(if got == bytes {
                Seen::Match
            } else {
                Seen::Differ
            });
        } else if !meta.is_dir() {
            return Ok(Seen::Blocked);
        }
    }
    Ok(Seen::Missing)
}

fn rel_of(segments: &[&str]) -> String {
    segments.join("/")
}

fn in_the_way(dir: Option<&Path>, id: &str, rel: &str) -> Fail {
    let fresh = format!(
        "mkdir {} && {}",
        shell_word(id),
        command(Some(Path::new(id)), &[id.to_string()])
    );
    let path = shown(dir, rel);
    if rel == "scene.jsonc" {
        fail(
            "scene_exists",
            format!(
                "{path} already holds a scene, and the Block `{id}` brings its own. orbe does not merge scenes. nothing was written"
            ),
            format!("install it into an empty folder: `{fresh}`"),
            FAILED,
        )
    } else {
        fail(
            "file_exists",
            format!("{path} is already there and is not the Block `{id}`'s. nothing was written"),
            format!(
                "move {path} out of the way and run `{}` again, or install into an empty folder: `{fresh}`",
                command(dir, &[id.to_string()])
            ),
            FAILED,
        )
    }
}

fn is_a_link(dir: Option<&Path>, id: &str, rel: &str) -> Fail {
    let path = shown(dir, rel);
    fail(
        "symlink",
        format!(
            "{path} is a link, and orbe add does not write through a link. nothing was written"
        ),
        format!(
            "remove the link {path}, then run `{}` again",
            command(dir, &[id.to_string()])
        ),
        FAILED,
    )
}

/// A part this binary's reader refuses is not written. A format newer than this binary reads
/// asks for the install command. Any other finding keeps that finding's own hint.
fn refuse_unreadable(forest: &Forest) -> Result<(), Fail> {
    for (id, versions) in &forest.nodes {
        for node in versions.values() {
            let found = orbe_scene::part_findings(id, &node.files);
            let Some(first) = found.first() else {
                continue;
            };
            let newer = first.code == Code::PartFormat
                && first.message.contains("newer than this binary reads");
            let hint = if newer {
                UPDATE.to_string()
            } else {
                first.hint.clone()
            };
            return Err(fail(
                first.code.as_str(),
                format!("{id}: {}. nothing was written", first.message),
                hint,
                FAILED,
            ));
        }
    }
    Ok(())
}

fn place_all(
    root: &Path,
    dir: Option<&Path>,
    ids: &[String],
    versions: &BTreeMap<String, String>,
    forest: &Forest,
) -> Result<(), Fail> {
    let mut staged: Vec<PathBuf> = Vec::new();
    for id in ids {
        let stage = root.join("parts").join(format!(".add-{id}"));
        if let Err(error) = write_tree(&stage, id, &versions[id], forest, &mut Vec::new()) {
            let _ = remove_path(&stage);
            for other in &staged {
                let _ = remove_path(other);
            }
            return Err(error);
        }
        staged.push(stage);
    }
    let mut committed = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let dest = root.join("parts").join(id);
        if let Err(error) = fs::rename(&staged[index], &dest) {
            for left in &staged[index..] {
                let _ = remove_path(left);
            }
            let message = if committed.is_empty() {
                io_at(&dest, error).message
            } else {
                format!(
                    "{}. remove {} and run `{}`",
                    io_at(&dest, error).message,
                    folder_list(&committed),
                    command(dir, ids)
                )
            };
            return Err(fail("io_error", message, "", IO));
        }
        committed.push(id.clone());
    }
    Ok(())
}

fn write_tree(
    dest: &Path,
    id: &str,
    version: &str,
    forest: &Forest,
    stack: &mut Vec<String>,
) -> Result<(), Fail> {
    if stack.iter().any(|seen| seen == id) {
        return Err(fail(
            "registry",
            format!("`{id}` depends on itself"),
            "",
            FAILED,
        ));
    }
    let Some(node) = forest
        .nodes
        .get(id)
        .and_then(|versions| versions.get(version))
    else {
        return Err(fail(
            "registry",
            format!("`{id}` was not fetched"),
            "",
            FAILED,
        ));
    };
    let files = node.files.clone();
    let uses = node.uses.clone();
    for (rel, bytes) in &files {
        write_new(&join_rel(dest, rel), bytes)?;
    }
    stack.push(id.to_string());
    for (dep, dep_version) in &uses {
        write_tree(
            &dest.join("parts").join(dep),
            dep,
            &dep_version.to_string(),
            forest,
            stack,
        )?;
    }
    stack.pop();
    Ok(())
}

struct BlockFile {
    path: String,
    wrote: bool,
}

struct BlockWrite {
    error: Fail,
    written: Vec<String>,
}

fn write_block_files(
    root: &Path,
    files: &[(String, Vec<u8>)],
    kept: &BTreeSet<String>,
) -> Result<Vec<BlockFile>, BlockWrite> {
    let mut out = Vec::new();
    let mut written = Vec::new();
    let mut ordered: Vec<&(String, Vec<u8>)> = files
        .iter()
        .filter(|file| file.0 == "scene.jsonc")
        .collect();
    ordered.extend(files.iter().filter(|file| file.0 != "scene.jsonc"));
    for (rel, bytes) in ordered {
        let wrote = !kept.contains(rel);
        if wrote {
            if let Err(error) = write_new(&join_rel(root, rel), bytes) {
                return Err(BlockWrite { error, written });
            }
            written.push(rel.clone());
        }
        out.push(BlockFile {
            path: rel.clone(),
            wrote,
        });
    }
    Ok(out)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Fail> {
    if let Some(parent) = path.parent() {
        mkdir(parent)?;
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(fail(
                "symlink",
                format!(
                    "{} is a link, and orbe add does not write through a link. nothing was written",
                    path.display()
                ),
                "",
                FAILED,
            ));
        }
        Ok(_) => {
            return Err(fail(
                "io_error",
                format!("{} is already there", path.display()),
                "",
                IO,
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_at(path, error)),
    }
    let tmp = temp_beside(path);
    let result = (|| -> Result<(), Fail> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|error| io_at(path, error))?;
        file.write_all(bytes).map_err(|error| io_at(path, error))?;
        drop(file);
        fs::rename(&tmp, path).map_err(|error| io_at(path, error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn temp_beside(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        ".add-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

fn mkdir(path: &Path) -> Result<(), Fail> {
    let mut cur = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {
                if cur.as_os_str().is_empty() {
                    cur.push(".");
                }
                continue;
            }
            Component::RootDir | Component::Prefix(_) => {
                cur.push(component);
                continue;
            }
            Component::ParentDir => {
                return Err(fail(
                    "io_error",
                    format!("{} leaves the project", path.display()),
                    "",
                    IO,
                ));
            }
            Component::Normal(segment) => cur.push(segment),
        }
        match fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(fail(
                    "symlink",
                    format!(
                        "{} is a link, and orbe add does not write through a link. nothing was written",
                        cur.display()
                    ),
                    "",
                    FAILED,
                ));
            }
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(fail(
                    "io_error",
                    format!("{} is not a folder", cur.display()),
                    "",
                    IO,
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&cur).map_err(|error| io_at(&cur, error))?;
            }
            Err(error) => return Err(io_at(&cur, error)),
        }
    }
    Ok(())
}

fn clear_staging(root: &Path) -> Result<(), Fail> {
    let parts = root.join("parts");
    let entries = match fs::read_dir(&parts) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_at(&parts, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|error| io_at(&parts, error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(".add-") {
            remove_path(&entry.path())?;
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), Fail> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_at(path, error)),
    };
    let result = if meta.file_type().is_symlink() || meta.is_file() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    };
    result.map_err(|error| io_at(path, error))
}

fn join_rel(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for segment in rel.split('/') {
        path.push(segment);
    }
    path
}

fn folder_list(ids: &[String]) -> String {
    let names: Vec<String> = ids.iter().map(|id| format!("parts/{id}")).collect();
    if names.is_empty() {
        "the part folders".into()
    } else {
        name_list(&names)
    }
}

fn name_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        many => {
            let (last, rest) = many.split_last().expect("at least three");
            format!("{} and {last}", rest.join(", "))
        }
    }
}

fn command(dir: Option<&Path>, ids: &[String]) -> String {
    let mut text = match dir {
        Some(dir) if !dir.as_os_str().is_empty() => {
            let mut shown = dir.display().to_string();
            if shown.starts_with('-') {
                shown = format!("./{shown}");
            }
            format!("orbe add -C {}", shell_word(&shown))
        }
        _ => "orbe add".into(),
    };
    for id in ids {
        text.push(' ');
        text.push_str(&shell_word(id));
    }
    text
}

fn shown(dir: Option<&Path>, rel: &str) -> String {
    let path = match dir {
        Some(dir) if !dir.as_os_str().is_empty() => {
            let base = if dir.display().to_string().starts_with('-') {
                Path::new(".").join(dir)
            } else {
                dir.to_path_buf()
            };
            base.join(rel)
        }
        _ => PathBuf::from(rel),
    };
    shell_word(&path.display().to_string())
}

fn next_command(dir: Option<&Path>) -> String {
    let scene = shown(dir, "scene.jsonc");
    let tests = match dir {
        Some(dir) if !dir.as_os_str().is_empty() => {
            let mut shown = dir.display().to_string();
            if shown.starts_with('-') {
                shown = format!("./{shown}");
            }
            format!(" {}", shell_word(&shown))
        }
        _ => String::new(),
    };
    format!("run `orbe check {scene}` and `orbe test{tests}`")
}

fn ended(lines: Vec<String>) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn io_at(path: &Path, error: io::Error) -> Fail {
    fail("io_error", format!("{}: {error}", path.display()), "", IO)
}

fn fail(code: &'static str, message: String, hint: impl Into<String>, exit: u8) -> Fail {
    Fail {
        code,
        message,
        hint: hint.into(),
        exit,
    }
}

fn registry(url: &str, detail: &str, hint: &str) -> Fail {
    fail("registry", format!("{url} {detail}"), hint, FAILED)
}

impl Client {
    fn new() -> Client {
        Client {
            index_url: String::new(),
            files: BTreeMap::new(),
            pages: BTreeMap::new(),
            failed: BTreeMap::new(),
        }
    }

    fn index(&mut self) -> Result<Index, Fail> {
        let url = index_url();
        let got = self.fetch(&url, MAX_BYTES)?;
        self.index_url.clone_from(&got.url);
        let url = got.url;
        serde_json::from_slice(&got.bytes)
            .map_err(|error| registry(&url, &format!("is not a registry index: {error}"), NETWORK))
    }

    fn record(&mut self, row: &Row) -> Result<(String, Item), Fail> {
        let url = resolve(&self.index_url, &row.url)?;
        let bytes = self.checked(&url, &row.sha256, row.bytes, "its index row")?;
        let item: Item = serde_json::from_slice(&bytes)
            .map_err(|error| registry(&url, &format!("is not a registry record: {error}"), ""))?;
        if item.kind() != row.kind {
            return Err(registry(
                &url,
                &format!(
                    "is a {}, and its index row says a {}",
                    item.kind(),
                    row.kind
                ),
                "",
            ));
        }
        Ok((url, item))
    }

    fn folder(
        &mut self,
        record_url: &str,
        head: &orbe_registry::Head,
    ) -> Result<BTreeMap<String, Vec<u8>>, Fail> {
        check_files(record_url, &head.files)?;
        let mut files = BTreeMap::new();
        for file in &head.files {
            let url = resolve(record_url, &file.url)?;
            let bytes = self.checked(&url, &file.sha256, file.bytes, "its record")?;
            files.insert(file.path.clone(), bytes);
        }
        orbe_registry::validate_record(&head.files, Some(&files), &head.hash)
            .map_err(|detail| registry(record_url, &detail, MISMATCH))?;
        Ok(files)
    }

    fn plug(&mut self, index: &Index, id: &str) -> Result<Option<PlugPart>, Fail> {
        let rows = highest(index)?;
        let Some(row) = rows.get(id).copied() else {
            return Ok(None);
        };
        if row.kind != Kind::Part {
            return Ok(None);
        }
        let (url, item) = self.record(row)?;
        let Item::Part(part) = item else {
            return Ok(None);
        };
        let Some(part_file) = part
            .head
            .files
            .iter()
            .find(|file| file.path == "part.jsonc")
        else {
            return Err(registry(&url, "has no part.jsonc", ""));
        };
        check_files(&url, &part.head.files)?;
        let part_url = resolve(&url, &part_file.url)?;
        let part_bytes =
            self.checked(&part_url, &part_file.sha256, part_file.bytes, "its record")?;
        let value =
            jsonc_value(&part_bytes).ok_or_else(|| registry(&part_url, "is not JSON", ""))?;
        let anchors_name = format!("{id}.anchors.json");
        let anchors = if let Some(file) = part
            .head
            .files
            .iter()
            .find(|file| file.path == anchors_name)
        {
            let anchors_url = resolve(&url, &file.url)?;
            let bytes = self.checked(&anchors_url, &file.sha256, file.bytes, "its record")?;
            jsonc_value(&bytes)
                .and_then(|value| value.get("anchors").and_then(Value::as_array).cloned())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Some(PlugPart { value, anchors }))
    }

    /// Fetches `blobs` that this command does not already hold, at most [`FETCH_LIMIT`]
    /// at a time. A cached blob is used when its bytes hash to `sha` and its length is
    /// `size`. Records and files share that cache. A failure is kept, and the walk
    /// reports the first one in its own order.
    fn prefetch(&mut self, blobs: &[Blob]) {
        let mut pending = Vec::new();
        let mut shas = Vec::new();
        let mut seen_url = BTreeSet::new();
        let mut seen_sha = BTreeSet::new();
        for blob in blobs {
            if blob.size > MAX_BYTES {
                continue;
            }
            if self.cached_file(&blob.sha, blob.size).is_some() {
                continue;
            }
            if self.pages.contains_key(&blob.url) || self.failed.contains_key(&blob.url) {
                continue;
            }
            if !seen_sha.insert(blob.sha.clone()) || !seen_url.insert(blob.url.clone()) {
                continue;
            }
            pending.push((blob.url.clone(), blob.size));
            shas.push(blob.sha.clone());
        }
        let results = fetch_many(&pending);
        for (index, result) in results.into_iter().enumerate() {
            let (url, size) = &pending[index];
            let sha = &shas[index];
            match result {
                Ok(got) => {
                    let matches = got.bytes.len() as u64 == *size && sha256(&got.bytes) == *sha;
                    if matches {
                        self.keep_file(sha, &got.bytes);
                    }
                    self.store_page(url, &got);
                }
                Err(fail) => {
                    self.failed.insert(url.clone(), fail);
                }
            }
        }
    }

    /// The files of `rows`, and the records of the parts they depend on. A row whose
    /// bytes are not already checked is skipped. The walk reports that row.
    fn prefetch_level(&mut self, index: &Index, rows: &[&Row]) {
        let mut blobs = Vec::new();
        for row in rows {
            let Some((record_url, item)) = self.item_from_cache(row) else {
                continue;
            };
            // Steps 5 and 6 run before any file of this record is fetched. A bad
            // path or url is left for the walk, which reports it.
            if check_files(&record_url, &item.head().files).is_ok() {
                blobs.extend(file_blobs(&record_url, &item.head().files, |_| true));
            }
            for dep in &item.head().registry_dependencies {
                if dep.registry.is_some() {
                    continue;
                }
                let Ok(child) = row_at(index, &dep.id, dep.version) else {
                    continue;
                };
                if let Some(blob) = self.blob_for_row(child) {
                    blobs.push(blob);
                }
            }
        }
        self.prefetch(&blobs);
    }

    fn row_blobs(&self, rows: &[&Row]) -> Vec<Blob> {
        rows.iter()
            .filter_map(|row| self.blob_for_row(row))
            .collect()
    }

    fn blob_for_row(&self, row: &Row) -> Option<Blob> {
        Some(Blob {
            url: resolve(&self.index_url, &row.url).ok()?,
            sha: row.sha256.clone(),
            size: row.bytes,
        })
    }

    /// The record, when this command already holds bytes that match the row.
    fn item_from_cache(&mut self, row: &Row) -> Option<(String, Item)> {
        let url = resolve(&self.index_url, &row.url).ok()?;
        let bytes = if let Some(bytes) = self.cached_file(&row.sha256, row.bytes) {
            bytes
        } else {
            let got = self.pages.get(&url)?;
            if got.bytes.len() as u64 != row.bytes || sha256(&got.bytes) != row.sha256 {
                return None;
            }
            got.bytes.clone()
        };
        let item: Item = serde_json::from_slice(&bytes).ok()?;
        if item.kind() != row.kind {
            return None;
        }
        let head = item.head();
        if head.name != row.name || head.version != row.version {
            return None;
        }
        Some((url, item))
    }

    fn store_page(&mut self, requested: &str, got: &Got) {
        self.pages.insert(got.url.clone(), got.clone());
        if got.url != requested {
            self.pages.insert(requested.to_string(), got.clone());
        }
    }

    fn checked(&mut self, url: &str, sha: &str, size: u64, says: &str) -> Result<Vec<u8>, Fail> {
        if size > MAX_BYTES {
            return Err(registry(
                url,
                &format!("is {size} bytes, which is more than orbe reads"),
                MISMATCH,
            ));
        }
        if let Some(bytes) = self.cached_file(sha, size) {
            return Ok(bytes);
        }
        let got = self.fetch(url, size)?;
        if got.bytes.len() as u64 != size {
            return Err(registry(
                &got.url,
                &format!("is {} bytes, and {says} {}", got.bytes.len(), size),
                MISMATCH,
            ));
        }
        let have = sha256(&got.bytes);
        if have != sha {
            return Err(registry(
                &got.url,
                &format!("SHA-256 is {have}, and {says} {sha}"),
                MISMATCH,
            ));
        }
        self.keep_file(sha, &got.bytes);
        Ok(got.bytes)
    }

    /// A record or file this command already holds, or a cached copy whose bytes hash to `sha`.
    fn cached_file(&mut self, sha: &str, size: u64) -> Option<Vec<u8>> {
        if let Some(bytes) = self.files.get(sha) {
            if bytes.len() as u64 == size {
                return Some(bytes.clone());
            }
            return None;
        }
        let bytes = read_cache(sha, size)?;
        if sha256(&bytes) != sha || bytes.len() as u64 != size {
            return None;
        }
        self.files.insert(sha.to_string(), bytes.clone());
        Some(bytes)
    }

    fn keep_file(&mut self, sha: &str, bytes: &[u8]) {
        self.files.insert(sha.to_string(), bytes.to_vec());
        write_cache(sha, bytes);
    }

    fn fetch(&mut self, url: &str, size: u64) -> Result<Got, Fail> {
        if let Some(got) = self.pages.get(url) {
            return Ok(got.clone());
        }
        if let Some(fail) = self.failed.get(url) {
            return Err(fail.clone());
        }
        match fetch_url(url, size) {
            Ok(got) => {
                self.store_page(url, &got);
                Ok(got)
            }
            Err(fail) => {
                self.failed.insert(url.to_string(), fail.clone());
                Err(fail)
            }
        }
    }
}

struct Blob {
    url: String,
    sha: String,
    size: u64,
}

/// Steps 5 and 6 of "Verify what you fetch". Nothing is fetched here.
fn check_files(record_url: &str, files: &[orbe_registry::File]) -> Result<(), Fail> {
    orbe_registry::validate_record(files, None, "")
        .map_err(|detail| registry(record_url, &detail, ""))
}

fn file_blobs(
    record_url: &str,
    files: &[orbe_registry::File],
    keep: impl Fn(&str) -> bool,
) -> Vec<Blob> {
    let mut blobs = Vec::new();
    for file in files {
        if !keep(&file.path) {
            continue;
        }
        let Ok(url) = resolve(record_url, &file.url) else {
            continue;
        };
        blobs.push(Blob {
            url,
            sha: file.sha256.clone(),
            size: file.bytes,
        });
    }
    blobs
}

fn fetch_url(url: &str, size: u64) -> Result<Got, Fail> {
    fetch_in(url, size, 1)
}

fn fetch_in(url: &str, size: u64, parallel: usize) -> Result<Got, Fail> {
    if url.starts_with("file:") {
        fetch_file(url, size)
    } else if url.starts_with("https://") {
        fetch_https(url, size, parallel)
    } else {
        Err(registry(url, "is not an https URL", ""))
    }
}

/// Fetches in waves of at most [`FETCH_LIMIT`]. Each https request names the width of
/// its wave, so a test server can hold that wave until every request in it has arrived.
fn fetch_many(requests: &[(String, u64)]) -> Vec<Result<Got, Fail>> {
    if requests.len() <= 1 {
        return requests
            .iter()
            .map(|(url, size)| fetch_url(url, *size))
            .collect();
    }
    let mut results = Vec::with_capacity(requests.len());
    for chunk in requests.chunks(FETCH_LIMIT) {
        let width = chunk.len();
        let slots = Mutex::new(Vec::with_capacity(width));
        slots
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .resize_with(width, || None);
        thread::scope(|scope| {
            for (index, (url, size)) in chunk.iter().enumerate() {
                let slots = &slots;
                scope.spawn(move || {
                    let result = fetch_in(url, *size, width);
                    slots.lock().unwrap_or_else(|err| err.into_inner())[index] = Some(result);
                });
            }
        });
        results.extend(
            slots
                .into_inner()
                .unwrap_or_else(|err| err.into_inner())
                .into_iter()
                .map(|slot| slot.expect("each fetch stores its result")),
        );
    }
    results
}

fn fetch_file(url: &str, size: u64) -> Result<Got, Fail> {
    let path = file_path(url)?;
    let meta =
        fs::symlink_metadata(&path).map_err(|_| registry(url, "could not be reached", NETWORK))?;
    if !meta.is_file() {
        return Err(registry(url, "is not a file", ""));
    }
    if meta.len() > size {
        return Err(registry(
            url,
            &format!("is {} bytes, and the fetch allows {size}", meta.len()),
            MISMATCH,
        ));
    }
    let bytes = fs::read(&path).map_err(|_| registry(url, "could not be reached", NETWORK))?;
    Ok(Got {
        url: url.to_string(),
        bytes,
    })
}

fn fetch_https(url: &str, size: u64, parallel: usize) -> Result<Got, Fail> {
    // A record URL contains `@`. Asking for `%40` skips the redirect a host inserts
    // before it serves the same file.
    let request = url.replace('@', "%40");
    let agent = format!("orbe/{}", env!("CARGO_PKG_VERSION"));
    let header = format!("X-Orbe-Parallel: {parallel}");
    let mut command = Command::new("curl");
    command.args([
        "-q",
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-redirs",
        REDIRECTS,
        "--connect-timeout",
        "10",
        "--max-time",
        "60",
        "--output",
        "-",
        "--user-agent",
        &agent,
        "--write-out",
        "%{stderr}%{url_effective}",
    ]);
    if parallel > 1 {
        command.args(["--header", header.as_str()]);
    }
    let mut child = match command
        .arg(&request)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(registry(
                url,
                "could not be fetched, because curl is not on PATH",
                "`orbe add` needs curl",
            ));
        }
        Err(_) => return Err(registry(url, "could not be reached", NETWORK)),
    };
    let mut stdout = child.stdout.take().expect("curl stdout is piped");
    let mut stderr = child.stderr.take().expect("curl stderr is piped");
    let err_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let read = read_limited(&mut stdout, size);
    let (body, too_big) = match read {
        Ok(got) => got,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = err_thread.join();
            return Err(registry(url, "could not be reached", NETWORK));
        }
    };
    if too_big {
        let _ = child.kill();
        let _ = child.wait();
        let _ = err_thread.join();
        return Err(registry(
            url,
            &format!("is larger than {size} bytes"),
            MISMATCH,
        ));
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(_) => {
            let _ = err_thread.join();
            return Err(registry(url, "could not be reached", NETWORK));
        }
    };
    let err_bytes = err_thread.join().unwrap_or_default();
    let err_text = String::from_utf8_lossy(&err_bytes);
    if !status.success() {
        return Err(curl_failed(url, &err_text));
    }
    Ok(Got {
        url: effective_url(&err_text).unwrap_or_else(|| url.to_string()),
        bytes: body,
    })
}

#[derive(Clone)]
struct Got {
    url: String,
    bytes: Vec<u8>,
}

fn read_limited(reader: &mut impl Read, size: u64) -> io::Result<(Vec<u8>, bool)> {
    let mut body = Vec::new();
    let mut buf = [0u8; 8192];
    let cap = size.saturating_add(1);
    loop {
        if body.len() as u64 >= cap {
            return Ok((body, true));
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok((body, false));
        }
        let room = cap - body.len() as u64;
        let take = n.min(usize::try_from(room).unwrap_or(usize::MAX));
        body.extend_from_slice(&buf[..take]);
        if n > take {
            return Ok((body, true));
        }
    }
}

fn curl_failed(url: &str, stderr: &str) -> Fail {
    if let Some(code) = http_status(stderr) {
        return registry(url, &format!("returned HTTP {code}"), "");
    }
    if stderr.contains("redirects followed") {
        return registry(
            url,
            &format!("followed more than {REDIRECTS} redirects"),
            "",
        );
    }
    registry(url, "could not be reached", NETWORK)
}

fn http_status(stderr: &str) -> Option<u16> {
    let marker = "returned error: ";
    let rest = stderr.get(stderr.rfind(marker)? + marker.len()..)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let code = digits.parse().ok()?;
    (100..600).contains(&code).then_some(code)
}

fn effective_url(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .map(str::to_string)
}

fn index_url() -> String {
    let template = std::env::var("ORBE_REGISTRY").unwrap_or_else(|_| TEMPLATE.to_string());
    template.replacen("{name}", "index", 1)
}

fn resolve(base: &str, rel: &str) -> Result<String, Fail> {
    if rel.contains("://") {
        return Ok(rel.to_string());
    }
    let (scheme, rest) = base
        .split_once("://")
        .ok_or_else(|| registry(base, "is not a URL", ""))?;
    if rel.starts_with("//") {
        return Ok(format!("{scheme}:{rel}"));
    }
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index + 1..]),
        None => (rest, ""),
    };
    let origin = format!("{scheme}://{authority}");
    if rel.starts_with('/') {
        return Ok(format!("{origin}{rel}"));
    }
    let dir = match path.rfind('/') {
        Some(index) => &path[..index],
        None => "",
    };
    let mut parts: Vec<&str> = dir
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    for segment in rel.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    Ok(format!("{origin}/{}", parts.join("/")))
}

fn file_path(url: &str) -> Result<PathBuf, Fail> {
    let Some(rest) = url.strip_prefix("file://") else {
        return Err(registry(url, "is not a file URL", ""));
    };
    let path = if let Some(stripped) = rest.strip_prefix("localhost") {
        stripped
    } else if rest.starts_with('/') {
        rest
    } else {
        return Err(registry(url, "is not a file URL", ""));
    };
    let decoded =
        percent_decode(path).map_err(|_| registry(url, "has a bad percent-encoding", ""))?;
    Ok(PathBuf::from(decoded))
}

fn percent_decode(text: &str) -> Result<String, ()> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).map_err(|_| ())?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| ())?);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).map_err(|_| ())
}

fn jsonc_value(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    jsonc_parser::parse_to_serde_value::<Value>(text, &jsonc_parser::ParseOptions::default()).ok()
}

fn sha256(bytes: &[u8]) -> String {
    orbe_registry::sha256(bytes)
}

/// `$XDG_CACHE_HOME/orbe` when that is an absolute path. A relative path is ignored, as the
/// XDG spec says. Otherwise `~/Library/Caches/orbe` on macOS and `~/.cache/orbe` on Linux.
/// Records and files are stored by their SHA-256 under `files/`. A missing home, or a cache
/// that cannot be written, means the command fetches without a cache.
fn cache_dir() -> Option<PathBuf> {
    let nonempty = |key: &str| std::env::var_os(key).filter(|value| !value.is_empty());
    if let Some(dir) = nonempty("XDG_CACHE_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path.join("orbe"));
        }
    }
    let home = PathBuf::from(nonempty("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library").join("Caches").join("orbe"))
    } else {
        Some(home.join(".cache").join("orbe"))
    }
}

fn cache_path(sha: &str) -> Option<PathBuf> {
    if sha.len() != 64
        || !sha
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    Some(cache_dir()?.join("files").join(sha))
}

/// Cached bytes for `sha` when the file is a regular file of exactly `size` bytes.
/// A size above [`MAX_BYTES`] is a miss. The file is opened without following a symlink,
/// and nothing is read until the open handle is a regular file of that length.
fn read_cache(sha: &str, size: u64) -> Option<Vec<u8>> {
    if size > MAX_BYTES {
        return None;
    }
    let path = cache_path(sha)?;
    let file = open_cache(&path)?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() != size {
        return None;
    }
    let mut bytes = Vec::with_capacity(size as usize);
    io::Read::take(file, size).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 != size {
        return None;
    }
    Some(bytes)
}

/// Opens a cache path. On Unix the open does not follow a symlink, so a link planted at a
/// real hash's path is a miss rather than a read of whatever it points at.
#[cfg(unix)]
fn open_cache(path: &Path) -> Option<fs::File> {
    use rustix::fs::{Mode, OFlags};
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = rustix::fs::open(path, flags, Mode::empty()).ok()?;
    Some(fs::File::from(fd))
}

#[cfg(not(unix))]
fn open_cache(path: &Path) -> Option<fs::File> {
    fs::File::open(path).ok()
}

fn write_cache(sha: &str, bytes: &[u8]) {
    let Some(path) = cache_path(sha) else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = temp_beside(&path);
    let written = (|| -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        drop(file);
        fs::rename(&tmp, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn a_file_url_resolves_against_the_record() {
        assert_eq!(
            resolve("http://h/r/led@1.json", "files/abc").unwrap(),
            "http://h/r/files/abc"
        );
    }

    #[test]
    fn a_record_url_resolves_against_the_index() {
        assert_eq!(
            resolve("http://h/r/index.json", "led@1.json").unwrap(),
            "http://h/r/led@1.json"
        );
    }

    #[test]
    fn an_absolute_url_is_kept() {
        assert_eq!(
            resolve("http://h/r/index.json", "https://other/x").unwrap(),
            "https://other/x"
        );
    }

    #[test]
    fn a_file_url_stays_in_its_directory() {
        assert_eq!(
            resolve("file:///tmp/reg/index.json", "led@1.json").unwrap(),
            "file:///tmp/reg/led@1.json"
        );
    }
}
