//! Builds a registry from a folder that holds `registry.jsonc`, `parts/<id>/` and
//! `blocks/<id>/`: an index, one record per item version, every file once by its SHA-256, and
//! the JSON Schemas of both records and index. `docs/registry.md` is the contract a client
//! reads it by.

mod installed;
mod item;
pub mod pattern;

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use orbe_scene::{Diagnostic, Folder, Missing, Version};
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use item::{
    BlockItem, Dependency, File, Format, Head, Index, Item, Kind, PartItem, Row, file_url,
};

/// The contract's version, which every index and record carries.
pub const FORMAT: u32 = 1;
/// The address of the index.
pub const INDEX: &str = "index";
pub const ITEM_SCHEMA: &str = "item.schema.json";
pub const INDEX_SCHEMA: &str = "index.schema.json";
/// The file at the root of a registry's source that names the registry.
pub const MANIFEST: &str = "registry.jsonc";
/// A file over this many bytes is named in the build's warnings.
pub const LARGE: u64 = 1_000_000;
/// Ids no item takes in any registry: the index's address, and the names Windows keeps for
/// devices, which no folder there can have.
pub const RESERVED: [&str; 25] = [
    "index", "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6",
    "com7", "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8",
    "lpt9",
];

#[derive(Debug, PartialEq)]
pub enum Error {
    /// The source holds something a registry cannot: a part that does not read, a Block whose
    /// parts are missing, two items with one name.
    Invalid(String),
    /// A part file or a Block scene the reader `orbe check` uses would refuse, with those findings.
    Check(Vec<Diagnostic>),
    /// The output folder already holds files.
    NotEmpty(PathBuf),
    /// A file could not be read or written.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(message) | Error::Io(message) => f.write_str(message),
            Error::Check(found) => {
                for (i, diagnostic) in found.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{diagnostic}")?;
                }
                Ok(())
            }
            Error::NotEmpty(out) => write!(
                f,
                "{} is not empty, and a registry is written into an empty folder",
                out.display()
            ),
        }
    }
}

impl Error {
    /// What to try next. Empty when the message is the whole failure.
    pub fn hint(&self) -> &'static str {
        match self {
            Error::Invalid(message) if message.contains(" has no registry.jsonc,") => {
                "write registry.jsonc in that folder"
            }
            Error::Invalid(message) if message.starts_with("no folder at ") => {
                "name the folder that holds registry.jsonc, parts/ and blocks/"
            }
            Error::Invalid(_) => {
                "fix the part or Block it names. docs/registry.md says what a registry holds"
            }
            Error::NotEmpty(_) => "name a new folder, or empty this one first",
            Error::Check(_) | Error::Io(_) => "",
        }
    }
}

fn io_error(path: &Path, error: io::Error) -> Error {
    Error::Io(format!("{}: {error}", path.display()))
}

/// A registry, before it is written.
pub struct Registry {
    /// The items built from the source, by name and version.
    pub items: Vec<Item>,
    /// Every file of the registry by its `/`-separated path from the registry's root.
    pub output: BTreeMap<String, Vec<u8>>,
    pub report: Report,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub parts: usize,
    pub blocks: usize,
    /// Files the built records list, once per record that lists them.
    pub listed: usize,
    /// Files stored under `files/`, once per distinct content.
    pub stored: usize,
    /// Every file the registry holds: index, schemas, records and stored files.
    pub files: usize,
    pub bytes: u64,
    pub missing: Gaps,
    /// Files a publisher may not mean to publish.
    pub notices: Vec<Notice>,
}

impl Report {
    /// One sentence per kind of item that lacks something, then one per notice.
    pub fn warnings(&self) -> Vec<String> {
        let mut lines = self.missing.warnings();
        lines.extend(self.notices.iter().map(Notice::to_string));
        lines
    }
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

/// A file the build publishes that its author may not mean to.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Notice {
    /// The item, as `<id>@<version>`.
    pub item: String,
    pub path: String,
    pub bytes: u64,
    pub why: Why,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Why {
    /// A segment of its path starts with a dot.
    Hidden,
    /// It is over [`LARGE`] bytes.
    Large,
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Notice {
            item, path, bytes, ..
        } = self;
        match self.why {
            Why::Hidden => write!(
                f,
                "{item} publishes the hidden file `{path}`, since a file in an item's folder is published unless it is build output"
            ),
            Why::Large => write!(f, "{item} publishes `{path}`, {bytes} bytes, over 1 MB"),
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} parts and {} Blocks: {} records list {} files. {} files are stored by hash. {} files, {} bytes in all",
            self.parts,
            self.blocks,
            self.parts + self.blocks,
            self.listed,
            self.stored,
            self.files,
            self.bytes
        )
    }
}

/// `registry.jsonc`: which registry the source builds.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[allow(dead_code, reason = "parsing it checks the format")]
    format: Format,
    name: String,
    #[serde(default)]
    homepage: Option<String>,
}

fn manifest(src: &Path) -> Result<Manifest, Error> {
    let path = src.join(MANIFEST);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(Error::Invalid(format!(
                "{} has no {MANIFEST}, which names the registry: {{ \"format\": 1, \"name\": \"@<id>\" }}",
                src.display()
            )));
        }
        Err(error) => return Err(io_error(&path, error)),
    };
    let manifest: Manifest =
        jsonc_parser::parse_to_serde_value(&text, &jsonc_parser::ParseOptions::default())
            .map_err(|error| Error::Invalid(format!("{}: {error}", path.display())))?;
    if !orbe_scene::is_registry_name(&manifest.name) {
        return Err(Error::Invalid(format!(
            "{} names `{}`, and a registry's name is `@` and an id, such as `@acme`",
            path.display(),
            manifest.name
        )));
    }
    if let Some(homepage) = &manifest.homepage
        && !homepage.starts_with("https://")
    {
        return Err(Error::Invalid(format!(
            "{} gives the homepage `{homepage}`, and a homepage is an https:// URL",
            path.display()
        )));
    }
    Ok(manifest)
}

/// Reads `src`, which holds `registry.jsonc` and `parts/<id>/` and `blocks/<id>/` folders, into
/// a registry. Only folders there are items. Every published file in an item's folder is one of
/// its files. Build output is left out, and so are `model.py` and `asset.json`. The error is a
/// sentence that says what does not read, or the findings `orbe check` would print for an
/// installed copy of a part or a Block.
pub fn build(src: &Path) -> Result<Registry, Error> {
    match fs::metadata(src) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(Error::Invalid(format!("no folder at {}", src.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(Error::Invalid(format!("no folder at {}", src.display())));
        }
        Err(error) => return Err(io_error(src, error)),
    }
    let manifest = manifest(src)?;
    let parts = folders(src, "parts")?;
    let blocks = folders(src, "blocks")?;
    if parts.is_empty() && blocks.is_empty() {
        return Err(Error::Invalid(format!(
            "{} holds no part in parts/ and no Block in blocks/",
            src.display()
        )));
    }
    if let Some(id) = parts.keys().find(|id| blocks.contains_key(*id)) {
        return Err(Error::Invalid(format!(
            "`{id}` is both a part and a Block, and one name names one item"
        )));
    }
    // The reader `orbe check` starts with. A file it refuses is not also compiled.
    let refused = reader_findings(&parts, &blocks);
    if !refused.is_empty() {
        return Err(Error::Check(refused));
    }
    let cards = orbe_scene::cards(&parts).map_err(Error::Invalid)?;
    let mut block_cards = Vec::new();
    for (id, files) in &blocks {
        block_cards.push(orbe_scene::block_card(id, files, &cards).map_err(Error::Invalid)?);
    }
    // The rest of `orbe check`: load the folders and compile, once per part and once per Block.
    match installed::refusal(&parts, &blocks, &cards) {
        Some(installed::Refusal::Outside(lines)) => return Err(Error::Invalid(lines.join("\n"))),
        Some(installed::Refusal::Check(found)) => return Err(Error::Check(found)),
        None => {}
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
    let offered: BTreeMap<String, bool> = cards
        .iter()
        .map(|card| (card.name.clone(), card.listed))
        .collect();
    let mut items: Vec<Item> = cards.into_iter().map(Item::from).collect();
    items.extend(block_cards.into_iter().map(Item::from));
    items.sort_by(|a, b| a.head().name.cmp(&b.head().name));

    let mut output = BTreeMap::new();
    let mut rows = Vec::new();
    let mut listed = 0;
    let mut notices = Vec::new();
    for item in &items {
        let record = json(item);
        let show = offered.get(&item.head().name).copied().unwrap_or(true);
        rows.push(item.row(&record, show));
        output.insert(format!("{}.json", item.address()), record);
        let name = &item.head().name;
        let folder = parts.get(name).or(blocks.get(name));
        for file in &item.head().files {
            listed += 1;
            notices.extend(notice(item, file));
            let bytes = folder.and_then(|folder| folder.get(&file.path));
            output.insert(
                file.url.clone(),
                bytes.expect("a card lists its folder's files").clone(),
            );
        }
    }
    rows.sort_by_key(|row| (row.name.clone(), Version::parse(&row.version)));
    let index = Index {
        schema: INDEX_SCHEMA.to_string(),
        format: Format,
        name: manifest.name,
        homepage: manifest.homepage,
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
        listed,
        stored,
        files: output.len(),
        bytes: output.values().map(|bytes| bytes.len() as u64).sum(),
        missing: gaps,
        notices,
    };
    Ok(Registry {
        items,
        output,
        report,
    })
}

/// Findings the part-file reader and the scene reader report, in the order `orbe check` prints.
fn reader_findings(
    parts: &BTreeMap<String, Folder>,
    blocks: &BTreeMap<String, Folder>,
) -> Vec<Diagnostic> {
    let mut found = Vec::new();
    for (id, files) in parts {
        found.extend(orbe_scene::part_findings(id, files));
    }
    for (id, files) in blocks {
        found.extend(orbe_scene::block_findings(id, files));
    }
    found.sort_by(|a, b| {
        (&a.file, a.line, a.column, a.code, &a.message)
            .cmp(&(&b.file, b.line, b.column, b.code, &b.message))
    });
    found.dedup();
    found
}

fn notice(item: &Item, file: &File) -> Vec<Notice> {
    let at = |why| Notice {
        item: item.address(),
        path: file.path.clone(),
        bytes: file.bytes,
        why,
    };
    let mut found = Vec::new();
    if file.path.split('/').any(|segment| segment.starts_with('.')) {
        found.push(at(Why::Hidden));
    }
    if file.bytes > LARGE {
        found.push(at(Why::Large));
    }
    found
}

impl Registry {
    /// Writes every file into `out`, which must be missing or empty, so a registry never mixes
    /// with files of an earlier one. The files go into a sibling folder first, which becomes
    /// `out` once all are written, so a failed write leaves no partial registry to serve.
    pub fn write(&self, out: &Path) -> Result<(), Error> {
        let exists = match fs::read_dir(out) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(Error::NotEmpty(out.to_path_buf()));
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(io_error(out, error)),
        };
        let out = std::path::absolute(out).map_err(|e| io_error(out, e))?;
        let (Some(parent), Some(name)) = (out.parent(), out.file_name()) else {
            return Err(Error::Io(format!("{} names no folder", out.display())));
        };
        fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        let stage = parent.join(format!(
            ".{}.partial-{}",
            name.to_string_lossy(),
            std::process::id()
        ));
        fs::create_dir(&stage).map_err(|e| io_error(&stage, e))?;
        let written = self.write_into(&stage).and_then(|()| {
            if exists {
                fs::remove_dir(&out).map_err(|e| io_error(&out, e))?;
            }
            fs::rename(&stage, &out).map_err(|e| io_error(&out, e))
        });
        if written.is_err() {
            let _ = fs::remove_dir_all(&stage);
        }
        written
    }

    fn write_into(&self, dir: &Path) -> Result<(), Error> {
        for (rel, bytes) in &self.output {
            let path = dir.join(rel);
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

/// Whether Windows keeps `stem` for a device, so no file or folder there can be named it, with
/// or without an extension.
fn is_device(stem: &str) -> bool {
    let stem = stem.to_ascii_lowercase();
    RESERVED[1..].contains(&stem.as_str())
}

/// Why `path`, a file's `/`-separated path in its item's folder, is not one a registry can
/// serve to every platform. `None` when it is.
pub fn path_problem(path: &str) -> Option<&'static str> {
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Some("an empty, `.` or `..` segment");
        }
        if segment
            .chars()
            .any(|c| c.is_control() || r#"\:*?"<>|"#.contains(c))
        {
            return Some("a backslash, a control character or one of :*?\"<>|");
        }
        if segment.ends_with(['.', ' ']) {
            return Some("a name that ends in a dot or a space");
        }
        if is_device(segment.split('.').next().unwrap_or(segment)) {
            return Some("a name Windows keeps for a device");
        }
    }
    None
}

/// The item folders in `src/<kind>/`, by id, each with every published file below it.
/// Build output is left out. A missing `src/<kind>/` holds none.
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
        if RESERVED.contains(&id.as_str()) {
            return Err(Error::Invalid(format!(
                "{kind}/{id}: `{id}` is reserved in every registry, for the index or by Windows, so no item takes it"
            )));
        }
        if !orbe_scene::is_id(&id) {
            return Err(Error::Invalid(format!(
                "{kind}/{id} is not an id: kebab case, starting with a letter, at most 64 characters"
            )));
        }
        let mut files = Folder::new();
        walk(&path, "", &mut files)?;
        let mut folded: BTreeMap<String, &str> = BTreeMap::new();
        for rel in files.keys() {
            if let Some(problem) = path_problem(rel) {
                return Err(Error::Invalid(format!(
                    "{kind}/{id}/{rel} has {problem}, so not every platform can hold it"
                )));
            }
            if let Some(other) = folded.insert(rel.to_lowercase(), rel) {
                return Err(Error::Invalid(format!(
                    "{kind}/{id} holds `{other}` and `{rel}`, which are one file on a disk that ignores case"
                )));
            }
        }
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
        if !orbe_scene::is_published(&rel) {
            continue;
        }
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

/// The file at the root of a registry source that records each part and Block a release served.
/// One line per item, `<id> <version> sha256:<hash>`, sorted by id. The hash is the SHA-256 of
/// the record [`build`] writes for that id and version. It is empty until the first release,
/// because nothing has been served, and `scripts/released.sh` rewrites it at a tag. A line stays
/// once written, including when its folder is gone.
pub const RELEASED: &str = "released.lock";

struct RecordPin {
    version: String,
    /// SHA-256 of the record bytes, as `sha256:<hex>`, the index row's digest with that prefix.
    hash: String,
    /// `parts/<id>/part.jsonc` or `blocks/<id>/block.jsonc`, where a bump is written.
    version_file: String,
}

fn record_hash(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256(bytes))
}

/// The record [`build`] would write for each part and Block in `src`, by id. An empty source
/// has none, so a rewrite can keep lines when every folder is gone. The hash is of the record
/// bytes, not the folder: the record already carries the folder hash, and a folder hash cannot
/// see the part versions a Block's record names.
fn record_pins(src: &Path) -> Result<BTreeMap<String, RecordPin>, Error> {
    if folders(src, "parts")?.is_empty() && folders(src, "blocks")?.is_empty() {
        return Ok(BTreeMap::new());
    }
    let registry = build(src)?;
    let mut pins = BTreeMap::new();
    for item in &registry.items {
        let name = &item.head().name;
        let bytes = registry
            .output
            .get(&format!("{}.json", item.address()))
            .expect("build stores each item's record");
        pins.insert(
            name.clone(),
            RecordPin {
                version: item.head().version.clone(),
                hash: record_hash(bytes),
                version_file: match item.kind() {
                    Kind::Part => format!("parts/{name}/part.jsonc"),
                    Kind::Block => format!("blocks/{name}/block.jsonc"),
                },
            },
        );
    }
    Ok(pins)
}

/// `released.lock` for `src`. Each current item is added, or its line advanced when its version
/// is newer. A line at the same version keeps the hash already recorded, so a same-version edit
/// is not written over. A line whose folder is gone stays. A version never moves backwards.
pub fn released_lock(src: &Path) -> Result<String, Error> {
    let pins = record_pins(src)?;
    let mut lines = match load_released(src)? {
        Some(text) => parse_released(&text)?,
        None => BTreeMap::new(),
    };
    for (id, pin) in &pins {
        let parsed = Version::parse(&pin.version).ok_or_else(|| {
            Error::Invalid(format!(
                "`{id}` has version `{}`, and a version is three numbers, such as `0.1.0`",
                pin.version
            ))
        })?;
        if let Some(line) = lines.get(id) {
            if parsed < line.version {
                return Err(Error::Invalid(format!(
                    "{id} is {}, and {RELEASED} records {}, which is newer. A released version does not go backwards",
                    pin.version, line.spelled
                )));
            }
            if parsed == line.version {
                continue;
            }
        }
        lines.insert(
            id.clone(),
            ReleasedLine {
                version: parsed,
                spelled: pin.version.clone(),
                hash: pin.hash.clone(),
            },
        );
    }
    let mut text = String::new();
    for (id, line) in &lines {
        text.push_str(&format!("{id} {} {}\n", line.spelled, line.hash));
    }
    Ok(text)
}

struct ReleasedLine {
    version: Version,
    spelled: String,
    hash: String,
}

/// The text of `src/released.lock`, or `None` when the file is missing.
fn load_released(src: &Path) -> Result<Option<String>, Error> {
    let path = src.join(RELEASED);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(&path, error)),
    }
}

/// The lines of `text`. A blank line is skipped.
fn parse_released(text: &str) -> Result<BTreeMap<String, ReleasedLine>, Error> {
    let mut lines = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let at = index + 1;
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [id, version, hash] = fields.as_slice() else {
            return Err(Error::Invalid(format!(
                "{RELEASED}:{at} is `{line}`, and a line is `<id> <version> sha256:<hash>`"
            )));
        };
        let Some(parsed) = Version::parse(version) else {
            return Err(Error::Invalid(format!(
                "{RELEASED}:{at} gives `{id}` the version `{version}`, and a version is three numbers, such as `0.1.0`"
            )));
        };
        let hex = hash
            .strip_prefix("sha256:")
            .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()));
        let lower = hash.bytes().all(|b| !b.is_ascii_uppercase());
        if !hex || !lower {
            return Err(Error::Invalid(format!(
                "{RELEASED}:{at} gives `{id}` the hash `{hash}`, and a hash is `sha256:` and 64 lowercase hex digits"
            )));
        }
        if lines
            .insert(
                (*id).to_string(),
                ReleasedLine {
                    version: parsed,
                    spelled: (*version).to_string(),
                    hash: (*hash).to_string(),
                },
            )
            .is_some()
        {
            return Err(Error::Invalid(format!(
                "{RELEASED} lists `{id}` twice, and it has one line per item"
            )));
        }
    }
    Ok(lines)
}

/// The lines of `src/released.lock`. A blank line is skipped. A missing file is an error, so
/// deleting the file does not turn the check off.
fn read_released(src: &Path) -> Result<BTreeMap<String, ReleasedLine>, Error> {
    match load_released(src)? {
        Some(text) => parse_released(&text),
        None => Err(Error::Invalid(format!(
            "{} is missing. It is empty until the first tag, and scripts/released.sh writes it when you tag a release",
            src.join(RELEASED).display()
        ))),
    }
}

/// The items in `src` whose record differs from `released.lock` at the version that file
/// records. The record is the one [`build`] writes. A line that names no folder is left, and an
/// item with no line passes. A folder at a newer version than its line passes. The same version
/// with another record asks for a bump.
pub fn released_changes(src: &Path) -> Result<Vec<String>, Error> {
    let lines = read_released(src)?;
    let mut changes = Vec::new();
    for (id, pin) in record_pins(src)? {
        let Some(line) = lines.get(&id) else {
            continue;
        };
        let version = Version::parse(&pin.version).ok_or_else(|| {
            Error::Invalid(format!(
                "`{id}` has version `{}`, and a version is three numbers, such as `0.1.0`",
                pin.version
            ))
        })?;
        if version > line.version {
            continue;
        }
        if version == line.version && pin.hash == line.hash {
            continue;
        }
        if version == line.version {
            changes.push(format!(
                "`{id}@{}` builds to a record other than the one {RELEASED} records. A served version never changes, so bump its version in {}",
                pin.version, pin.version_file
            ));
        } else {
            changes.push(format!(
                "{id} is {}, and {RELEASED} records {}, which is newer. A released version does not go backwards",
                pin.version, line.spelled
            ));
        }
    }
    Ok(changes)
}
