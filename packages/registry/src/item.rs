//! The records a registry serves: one per item version, and the index that lists them.

use orbe_scene::{BlockCard, Buy, Card, CardFile, Doc, NotSimulated, On, Param, Pin, Rating, Real};
use schemars::JsonSchema;
use serde::Serialize;

/// The URL of the file with this SHA-256, relative to the record that lists it.
pub fn file_url(sha256: &str) -> String {
    format!("files/{sha256}")
}

/// One record. `type` says which of the three it is.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
#[schemars(title = "Registry item")]
pub enum Item {
    Part(Box<PartItem>),
    Block(BlockItem),
    Model(ModelItem),
}

/// A part at one version: what its page shows and what `orbe add` copies.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct PartItem {
    pub name: String,
    pub version: String,
    pub title: String,
    /// The part file's `summary`. Absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The part file's `tags`. Absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    pub license: String,
    /// Each part this part is built from, as `<name>@<version>`.
    pub registry_dependencies: Vec<String>,
    /// Every file in the part's folder. `orbe add` writes them to `parts/<name>/<path>`.
    pub files: Vec<File>,
    /// The folder hash `parts.lock` records, as `sha256:<hex>`.
    pub hash: String,
    pub real: Real,
    pub pins: Vec<Pin>,
    pub params: Vec<Param>,
    pub ratings: Vec<Rating>,
    pub not_simulated: Vec<NotSimulated>,
    /// The part file's `docs`. A doc with no `url` is a name with no link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<Vec<Doc>>,
    /// The parts in this registry that are built from this one.
    pub used_by: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buy: Option<Vec<Buy>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<On>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pressable: bool,
}

/// A Block: a board, parts, firmware and tests that work together. A Block has no version.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct BlockItem {
    pub name: String,
    pub title: String,
    /// The Block file's `summary`. Absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The Block file's `tags`. Absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    pub license: String,
    /// Every part the scene runs, as `<name>@<version>`: the versions the Block was tested with.
    pub registry_dependencies: Vec<String>,
    /// Every file in the Block's folder. `orbe add` writes each one but `block.jsonc` and
    /// `README.md` to the project, at its `path`.
    pub files: Vec<File>,
    /// The part the Block's firmware runs on, one of `registryDependencies`.
    pub board: String,
}

/// An engine model, shipped as WebAssembly. Its own fields come with the device contract.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct ModelItem {
    pub name: String,
    pub version: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    pub license: String,
    pub registry_dependencies: Vec<String>,
    pub files: Vec<File>,
}

/// One file of an item.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct File {
    /// The `/`-separated path in the item's folder.
    pub path: String,
    /// Where the bytes are, relative to the record's URL.
    pub url: String,
    /// The SHA-256 of the bytes, as lowercase hex.
    pub sha256: String,
    pub bytes: u64,
    /// The SPDX expression that covers this file.
    pub license: String,
}

impl From<CardFile> for File {
    fn from(file: CardFile) -> File {
        File {
            url: file_url(&file.sha256),
            path: file.path,
            sha256: file.sha256,
            bytes: file.bytes,
            license: file.license,
        }
    }
}

impl From<Card> for Item {
    fn from(card: Card) -> Item {
        let Card {
            name,
            title,
            version,
            license,
            hash,
            description,
            categories,
            real,
            pins,
            params,
            ratings,
            not_simulated,
            docs,
            files,
            uses,
            used_by,
            buy,
            on,
            pressable,
        } = card;
        Item::Part(Box::new(PartItem {
            name,
            version,
            title,
            description,
            categories,
            license,
            registry_dependencies: addresses(uses),
            files: files.into_iter().map(File::from).collect(),
            hash,
            real,
            pins,
            params,
            ratings,
            not_simulated,
            docs,
            used_by,
            buy,
            on,
            pressable,
        }))
    }
}

impl From<BlockCard> for Item {
    fn from(card: BlockCard) -> Item {
        let BlockCard {
            name,
            title,
            description,
            categories,
            board,
            license,
            parts,
            files,
        } = card;
        Item::Block(BlockItem {
            name,
            title,
            description,
            categories,
            license,
            registry_dependencies: addresses(parts),
            files: files.into_iter().map(File::from).collect(),
            board,
        })
    }
}

fn addresses(versions: impl IntoIterator<Item = (String, String)>) -> Vec<String> {
    versions
        .into_iter()
        .map(|(name, version)| format!("{name}@{version}"))
        .collect()
}

impl Item {
    pub fn name(&self) -> &str {
        match self {
            Item::Part(item) => &item.name,
            Item::Block(item) => &item.name,
            Item::Model(item) => &item.name,
        }
    }

    pub fn version(&self) -> Option<&str> {
        match self {
            Item::Part(item) => Some(&item.version),
            Item::Block(_) => None,
            Item::Model(item) => Some(&item.version),
        }
    }

    /// What the URL template's `{name}` becomes for this record: `<name>@<version>`, or a
    /// Block's name alone.
    pub fn address(&self) -> String {
        match self.version() {
            Some(version) => format!("{}@{version}", self.name()),
            None => self.name().to_string(),
        }
    }

    pub fn files(&self) -> &[File] {
        match self {
            Item::Part(item) => &item.files,
            Item::Block(item) => &item.files,
            Item::Model(item) => &item.files,
        }
    }

    /// The index row for this record, served as `bytes`.
    pub fn row(&self, bytes: &[u8]) -> Row {
        let (kind, title, description, categories) = match self {
            Item::Part(item) => (Kind::Part, &item.title, &item.description, &item.categories),
            Item::Block(item) => (
                Kind::Block,
                &item.title,
                &item.description,
                &item.categories,
            ),
            Item::Model(item) => (
                Kind::Model,
                &item.title,
                &item.description,
                &item.categories,
            ),
        };
        Row {
            name: self.name().to_string(),
            kind,
            version: self.version().map(str::to_string),
            title: title.clone(),
            description: description.clone(),
            categories: categories.clone(),
            url: format!("{}.json", self.address()),
            sha256: crate::sha256(bytes),
            bytes: bytes.len() as u64,
        }
    }
}

/// The index: one row per record, by name and then version.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[schemars(title = "Registry index", deny_unknown_fields)]
pub struct Index {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub items: Vec<Row>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Part,
    Block,
    Model,
}

/// One record, with what search shows and where to fetch it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct Row {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    /// Absent for a Block, which has no version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    /// The record's URL, relative to the index's.
    pub url: String,
    /// The SHA-256 of the record's bytes, as lowercase hex.
    pub sha256: String,
    pub bytes: u64,
}
