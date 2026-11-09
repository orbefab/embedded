//! Builds a registry from a folder that holds `registry.jsonc`, `parts/<id>/` and
//! `blocks/<id>/`: an index, one record per item version, every file once by its SHA-256, and
//! the JSON Schemas of both records and index. `docs/registry.md` is the contract a client
//! reads it by.

mod item;
pub mod pattern;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use orbe_scene::{Folder, Missing, Version};
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
    /// parts are missing, two items with one name. Or `--previous` is not a registry this
    /// build can carry forward.
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
    /// The items built from the source, by name and version. Records carried from a previous
    /// build are in `output` and the index, not here.
    pub items: Vec<Item>,
    /// Every file of the registry by its `/`-separated path from the registry's root.
    pub output: BTreeMap<String, Vec<u8>>,
    pub report: Report,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub parts: usize,
    pub blocks: usize,
    /// Records carried from the previous build, at versions the source no longer holds.
    pub carried: usize,
    /// Files the built records list, once per record that lists them.
    pub listed: usize,
    /// Files stored under `files/`, once per distinct content, carried ones included.
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
                "{item} publishes the hidden file `{path}`, since every file in an item's folder is published"
            ),
            Why::Large => write!(f, "{item} publishes `{path}`, {bytes} bytes, over 1 MB"),
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} parts and {} Blocks: {} records list {} files",
            self.parts,
            self.blocks,
            self.parts + self.blocks,
            self.listed,
        )?;
        if self.carried > 0 {
            write!(f, ", and {} records are carried forward", self.carried)?;
        }
        write!(
            f,
            ". {} files are stored by hash. {} files, {} bytes in all",
            self.stored, self.files, self.bytes
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
/// a registry. Only folders there are items. Every file in an item's folder is one of its
/// files. With `previous`, a folder an earlier build wrote, every record it serves stays served:
/// a version the source still holds must build to the same bytes, and the others are carried
/// forward with their files. The error is a sentence that says what does not read.
pub fn build(src: &Path, previous: Option<&Path>) -> Result<Registry, Error> {
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
    items.sort_by(|a, b| a.head().name.cmp(&b.head().name));

    let mut output = BTreeMap::new();
    let mut rows = Vec::new();
    let mut listed = 0;
    let mut notices = Vec::new();
    for item in &items {
        let record = json(item);
        rows.push(item.row(&record));
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
    let carried = match previous {
        Some(dir) => carry(dir, &manifest.name, &items, &mut output, &mut rows)?,
        None => 0,
    };
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
        carried,
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

/// What a carried record must hold for the build to check it: its identity and its files.
#[derive(Deserialize)]
struct Served {
    #[serde(rename = "type")]
    kind: Kind,
    name: String,
    version: String,
    files: Vec<File>,
}

/// Adds every record the registry in `dir` serves, and its files, to `output` and `rows`, after
/// checking each against its hashes. A record the new build also wrote must be byte for byte
/// the same. Returns how many records were carried.
fn carry(
    dir: &Path,
    name: &str,
    items: &[Item],
    output: &mut BTreeMap<String, Vec<u8>>,
    rows: &mut Vec<Row>,
) -> Result<usize, Error> {
    let read = |rel: &str| {
        let path = dir.join(rel);
        fs::read(&path).map_err(|e| io_error(&path, e))
    };
    let invalid = |what: String| Error::Invalid(format!("--previous {}: {what}", dir.display()));
    let index: Index = serde_json::from_slice(&read(&format!("{INDEX}.json"))?)
        .map_err(|e| invalid(format!("index.json is not a registry index: {e}")))?;
    if index.name != name {
        return Err(invalid(format!(
            "it is the registry `{}`, and {MANIFEST} names `{name}`",
            index.name
        )));
    }
    let mut seen = BTreeSet::new();
    let mut carried = 0;
    for row in index.items {
        let at = item::address(&row.name, &row.version);
        if !orbe_scene::is_id(&row.name) || Version::parse(&row.version).is_none() {
            return Err(invalid(format!(
                "index.json lists `{at}`, which is not an id and a version"
            )));
        }
        if !seen.insert(at.clone()) {
            return Err(invalid(format!("index.json lists `{at}` twice")));
        }
        if row.url != item::record_url(&row.name, &row.version) {
            return Err(invalid(format!(
                "index.json gives `{at}` the URL `{}`, and a record's URL is `{at}.json`",
                row.url
            )));
        }
        let bytes = read(&row.url)?;
        if sha256(&bytes) != row.sha256 || bytes.len() as u64 != row.bytes {
            return Err(invalid(format!(
                "{} does not match the hash and size its index row gives",
                row.url
            )));
        }
        let served: Served = serde_json::from_slice(&bytes)
            .map_err(|e| invalid(format!("{} is not a record: {e}", row.url)))?;
        if (served.kind, &served.name, &served.version) != (row.kind, &row.name, &row.version) {
            return Err(invalid(format!(
                "{} is not the {} its index row names",
                row.url, row.kind
            )));
        }
        if let Some(built) = items.iter().find(|item| item.head().name == row.name)
            && built.kind() != row.kind
        {
            return Err(invalid(format!(
                "`{}` is a {} there and a {} in this build, and one name names one item",
                row.name,
                row.kind,
                built.kind()
            )));
        }
        if let Some(rebuilt) = output.get(&row.url) {
            if *rebuilt != bytes {
                return Err(invalid(format!(
                    "`{at}` builds to a record other than the one served at that version. A served version never changes, so give the change a new version"
                )));
            }
            continue;
        }
        for file in &served.files {
            if let Some(problem) = path_problem(&file.path) {
                return Err(invalid(format!(
                    "{} lists `{}`, which has {problem}",
                    row.url, file.path
                )));
            }
            let hex = file.sha256.len() == 64
                && file
                    .sha256
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            if !hex || file.url != file_url(&file.sha256) {
                return Err(invalid(format!(
                    "{} lists `{}` at `{}`, and a file's URL is `files/<sha256>`",
                    row.url, file.path, file.url
                )));
            }
            let stored = read(&file.url)?;
            if sha256(&stored) != file.sha256 || stored.len() as u64 != file.bytes {
                return Err(invalid(format!(
                    "{} does not match the hash and size {} gives",
                    file.url, row.url
                )));
            }
            output.insert(file.url.clone(), stored);
        }
        output.insert(row.url.clone(), bytes);
        rows.push(row);
        carried += 1;
    }
    Ok(carried)
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
