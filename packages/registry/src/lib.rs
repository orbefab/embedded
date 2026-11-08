//! Builds a registry from a folder that holds `parts/<id>/` and `blocks/<id>/`: an index, one
//! record per item version, every file once by its SHA-256, and the JSON Schemas of both
//! records and index. `docs/registry.md` is the contract a client reads it by.

mod item;

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use orbe_scene::{Folder, Missing};
use schemars::generate::SchemaSettings;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub use item::{BlockItem, File, Index, Item, Kind, ModelItem, PartItem, Row, file_url};

/// The address of the index, which no item may take as its name.
pub const INDEX: &str = "index";
pub const ITEM_SCHEMA: &str = "item.schema.json";
pub const INDEX_SCHEMA: &str = "index.schema.json";

#[derive(Debug, PartialEq)]
pub enum Error {
    /// The source holds something a registry cannot: a part that does not read, a Block whose
    /// parts are missing, two items with one name.
    Invalid(String),
    /// The output folder already holds files.
    NotEmpty(PathBuf),
    /// A file could not be read or written.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(message) | Error::Io(message) => f.write_str(message),
            Error::NotEmpty(out) => write!(
                f,
                "{} is not empty, and a registry is written into an empty folder",
                out.display()
            ),
        }
    }
}

fn io_error(path: &Path, error: io::Error) -> Error {
    Error::Io(format!("{}: {error}", path.display()))
}

/// A registry, before it is written.
pub struct Registry {
    pub items: Vec<Item>,
    /// Every file of the registry by its `/`-separated path from the registry's root.
    pub output: BTreeMap<String, Vec<u8>>,
    pub report: Report,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub parts: usize,
    pub blocks: usize,
    pub models: usize,
    /// Files the records list, once per record that lists them.
    pub listed: usize,
    /// Files stored under `files/`, once per distinct content.
    pub stored: usize,
    /// Every file the registry holds: index, schemas, records and stored files.
    pub files: usize,
    pub bytes: u64,
    pub missing: Gaps,
}

/// What the items lack, counted. A part or a Block shows a missing field as absent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Gaps {
    pub parts: Missing,
    pub blocks: BlockGaps,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlockGaps {
    pub blocks: usize,
    pub description: usize,
    pub categories: usize,
}

impl Gaps {
    /// One sentence per kind of item that lacks something.
    pub fn warnings(&self) -> Vec<String> {
        let p = &self.parts;
        let b = &self.blocks;
        let mut lines = Vec::new();
        if [p.description, p.categories, p.docs, p.datasheet_url, p.buy] != [0; 5] {
            lines.push(p.to_string());
        }
        if [b.description, b.categories] != [0; 2] {
            lines.push(format!(
                "{} Blocks: {} lack a description, {} lack categories",
                b.blocks, b.description, b.categories
            ));
        }
        lines
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} parts, {} Blocks and {} models: {} records list {} files, stored as {} by hash. {} files, {} bytes in all",
            self.parts,
            self.blocks,
            self.models,
            self.parts + self.blocks + self.models,
            self.listed,
            self.stored,
            self.files,
            self.bytes
        )
    }
}

/// Reads `src`, which holds `parts/<id>/` and `blocks/<id>/` folders, into a registry. Only
/// folders there are items. Every file in an item's folder is one of its files. The error is a
/// sentence that says what in `src` does not read.
pub fn build(src: &Path) -> Result<Registry, Error> {
    let parts = folders(src, "parts")?;
    let blocks = folders(src, "blocks")?;
    if parts.is_empty() && blocks.is_empty() {
        return Err(Error::Invalid(format!(
            "{} holds no part in parts/ and no Block in blocks/",
            src.display()
        )));
    }
    for id in parts.keys().chain(blocks.keys()) {
        if id == INDEX {
            return Err(Error::Invalid(format!(
                "`{INDEX}` is the index's address, so no part or Block takes it"
            )));
        }
    }
    if let Some(id) = parts.keys().find(|id| blocks.contains_key(*id)) {
        return Err(Error::Invalid(format!(
            "`{id}` is both a part and a Block, and one name names one item"
        )));
    }
    let cards = orbe_scene::cards(&parts).map_err(Error::Invalid)?;
    let mut block_cards = Vec::new();
    for (id, files) in &blocks {
        block_cards.push(orbe_scene::block_card(id, files, &cards).map_err(Error::Invalid)?);
    }
    let gaps = Gaps {
        parts: orbe_scene::missing_fields(&cards),
        blocks: BlockGaps {
            blocks: block_cards.len(),
            description: block_cards
                .iter()
                .filter(|b| b.description.is_none())
                .count(),
            categories: block_cards
                .iter()
                .filter(|b| b.categories.is_none())
                .count(),
        },
    };
    let mut items: Vec<Item> = cards.into_iter().map(Item::from).collect();
    items.extend(block_cards.into_iter().map(Item::from));
    items.sort_by(|a, b| a.name().cmp(b.name()));

    let mut output = BTreeMap::new();
    let mut rows = Vec::new();
    let mut listed = 0;
    for item in &items {
        let record = json(item);
        rows.push(item.row(&record));
        output.insert(format!("{}.json", item.address()), record);
        let folder = parts.get(item.name()).or(blocks.get(item.name()));
        for file in item.files() {
            listed += 1;
            let bytes = folder.and_then(|folder| folder.get(&file.path));
            output.insert(
                file.url.clone(),
                bytes.expect("a card lists its folder's files").clone(),
            );
        }
    }
    let index = Index {
        schema: INDEX_SCHEMA.to_string(),
        items: rows,
    };
    output.insert(format!("{INDEX}.json"), json(&index));
    output.insert(ITEM_SCHEMA.to_string(), json(&schema::<Item>()));
    output.insert(INDEX_SCHEMA.to_string(), json(&schema::<Index>()));
    let stored = output
        .keys()
        .filter(|path| path.starts_with("files/"))
        .count();
    let report = Report {
        parts: parts.len(),
        blocks: blocks.len(),
        models: 0,
        listed,
        stored,
        files: output.len(),
        bytes: output.values().map(|bytes| bytes.len() as u64).sum(),
        missing: gaps,
    };
    Ok(Registry {
        items,
        output,
        report,
    })
}

impl Registry {
    /// Writes every file below `out`, which must be missing or empty, so a registry never mixes
    /// with files of an earlier one.
    pub fn write(&self, out: &Path) -> Result<(), Error> {
        match fs::read_dir(out) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(Error::NotEmpty(out.to_path_buf()));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(out, error)),
        }
        for (rel, bytes) in &self.output {
            let path = out.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
            }
            fs::write(&path, bytes).map_err(|e| io_error(&path, e))?;
        }
        Ok(())
    }
}

/// The JSON Schema of `T` as the registry serves it.
pub fn schema<T: schemars::JsonSchema>() -> schemars::Schema {
    SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<T>()
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn json<T: Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("a record serializes");
    bytes.push(b'\n');
    bytes
}

/// The item folders in `src/<kind>/`, by id, each with every file below it. A missing
/// `src/<kind>/` holds none.
fn folders(src: &Path, kind: &str) -> Result<BTreeMap<String, Folder>, Error> {
    let dir = src.join(kind);
    let mut found = BTreeMap::new();
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(io_error(&dir, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| io_error(&dir, e))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| io_error(&path, e))?;
        if file_type.is_symlink() {
            return Err(linked(&path));
        }
        if !file_type.is_dir() {
            continue;
        }
        let id = utf8(&path, entry.file_name())?;
        if !orbe_scene::is_id(&id) {
            return Err(Error::Invalid(format!(
                "{kind}/{id} is not an id: kebab case, starting with a letter, at most 64 characters"
            )));
        }
        let mut files = Folder::new();
        walk(&path, "", &mut files)?;
        found.insert(id, files);
    }
    Ok(found)
}

fn walk(dir: &Path, prefix: &str, files: &mut Folder) -> Result<(), Error> {
    let entries = fs::read_dir(dir).map_err(|e| io_error(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_error(dir, e))?;
        let path = entry.path();
        let rel = format!("{prefix}{}", utf8(&path, entry.file_name())?);
        let kind = entry.file_type().map_err(|e| io_error(&path, e))?;
        if kind.is_symlink() {
            return Err(linked(&path));
        } else if kind.is_dir() {
            walk(&path, &format!("{rel}/"), files)?;
        } else {
            let bytes = fs::read(&path).map_err(|e| io_error(&path, e))?;
            files.insert(rel, bytes);
        }
    }
    Ok(())
}

fn utf8(path: &Path, name: std::ffi::OsString) -> Result<String, Error> {
    name.into_string()
        .map_err(|_| Error::Invalid(format!("{} is not a UTF-8 name", path.display())))
}

fn linked(path: &Path) -> Error {
    Error::Invalid(format!(
        "{} is a link, and a registry holds only folders and files",
        path.display()
    ))
}
