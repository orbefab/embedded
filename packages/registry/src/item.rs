//! The records a registry serves, one per item version, and the index that lists them.

use std::borrow::Cow;
use std::fmt;

use orbe_scene::{
    BlockCard, Buy, Card, CardFile, Doc, Holes, NotSimulated, Param, Pin, Rating, Real, Threshold,
};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::pattern;
use crate::{FORMAT, ITEM_SCHEMA};

/// The URL of the file with this SHA-256, relative to the record that lists it.
pub fn file_url(sha256: &str) -> String {
    format!("files/{sha256}")
}

/// The contract's version. Every index and record carries it as `"format": 1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Format;

impl Serialize for Format {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(FORMAT)
    }
}

impl<'de> Deserialize<'de> for Format {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Format, D::Error> {
        match u32::deserialize(deserializer)? {
            FORMAT => Ok(Format),
            other => Err(serde::de::Error::custom(format!(
                "format {other}, and this orbe reads format {FORMAT}"
            ))),
        }
    }
}

impl JsonSchema for Format {
    fn schema_name() -> Cow<'static, str> {
        "Format".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The contract's version. A client refuses a format it does not know.",
            "const": FORMAT
        })
    }
}

/// An item a record depends on: `[@<registry>/]<id>@<version>`. Without a registry it is in
/// the same registry as the record.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dependency {
    pub registry: Option<String>,
    pub id: String,
    pub version: orbe_scene::Version,
}

impl Dependency {
    pub fn parse(text: &str) -> Option<Dependency> {
        let (registry, rest) = match text.strip_prefix('@') {
            Some(scoped) => {
                let (registry, rest) = scoped.split_once('/')?;
                (Some(format!("@{registry}")), rest)
            }
            None => (None, text),
        };
        let (id, version) = rest.split_once('@')?;
        let named = registry.as_deref().is_none_or(orbe_scene::is_registry_name);
        (named && orbe_scene::is_id(id)).then_some(())?;
        Some(Dependency {
            registry,
            id: id.to_string(),
            version: orbe_scene::Version::parse(version)?,
        })
    }
}

impl fmt::Display for Dependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(registry) = &self.registry {
            write!(f, "{registry}/")?;
        }
        write!(f, "{}@{}", self.id, self.version)
    }
}

impl Serialize for Dependency {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Dependency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Dependency, D::Error> {
        let text = String::deserialize(deserializer)?;
        Dependency::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("`{text}` is not <id>@<version>")))
    }
}

impl JsonSchema for Dependency {
    fn schema_name() -> Cow<'static, str> {
        "Dependency".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "[@<registry>/]<id>@<version>. Without a registry it is in the same registry as the record.",
            "type": "string",
            "pattern": pattern::DEPENDENCY
        })
    }
}

/// One record. `type` says which of the two it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
#[schemars(title = "Registry item")]
pub enum Item {
    Part(Box<PartItem>),
    Block(Box<BlockItem>),
}

/// The fields every record has, whatever its type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Head {
    /// Where an editor finds the record's schema: `item.schema.json`, beside the record.
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: Format,
    #[schemars(pattern(pattern::ID))]
    pub name: String,
    #[schemars(pattern(pattern::VERSION))]
    pub version: String,
    #[schemars(length(min = 1))]
    pub title: String,
    /// The item file's `summary`. Absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub description: Option<String>,
    /// The item file's `tags`. Absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<String>")]
    pub categories: Option<Vec<String>>,
    /// The item's SPDX expression.
    #[schemars(length(min = 1))]
    pub license: String,
    /// Each item this one is built from or runs.
    pub registry_dependencies: Vec<Dependency>,
    /// Each published file in the item's folder, by path. Build output (`__pycache__`, `.pyc`,
    /// `.pytest_cache` and `.venv`) is left out, and so are `model.py` and `asset.json`.
    pub files: Vec<File>,
    /// The folder hash over `files`, as `sha256:<hex>`.
    #[schemars(pattern(pattern::HASH))]
    pub hash: String,
}

/// A part at one version: what its page shows and what `orbe add` copies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PartItem {
    #[serde(flatten)]
    pub head: Head,
    pub real: Real,
    pub pins: Vec<Pin>,
    pub params: Vec<Param>,
    pub ratings: Vec<Rating>,
    pub not_simulated: Vec<NotSimulated>,
    /// The part file's `docs`. A doc with no `url` is a name with no link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<Doc>")]
    pub docs: Option<Vec<Doc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<Buy>")]
    pub buy: Option<Vec<Buy>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Threshold")]
    pub lit: Option<Threshold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Threshold")]
    pub dim: Option<Threshold>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pressable: bool,
    /// A board's holes, such as a breadboard's: how deep each is in mm, and the holes each
    /// strip joins. Each hole is one of `pins`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Holes")]
    pub holes: Option<Holes>,
}

/// A Block at one version: a board, parts, firmware and tests that work together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlockItem {
    #[serde(flatten)]
    pub head: Head,
    /// The part the Block's firmware runs on, one of `registryDependencies`.
    #[schemars(pattern(pattern::ID))]
    pub board: String,
}

/// One file of an item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct File {
    /// The `/`-separated path in the item's folder.
    #[schemars(pattern(pattern::PATH))]
    pub path: String,
    /// Where the bytes are, relative to the record's URL: `files/<sha256>`.
    #[schemars(pattern(pattern::FILE_URL))]
    pub url: String,
    /// The SHA-256 of the bytes, as lowercase hex.
    #[schemars(pattern(pattern::SHA256))]
    pub sha256: String,
    pub bytes: u64,
    /// The SPDX expression that covers this file.
    #[schemars(length(min = 1))]
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

/// Bare dependencies on `(id, version)` pairs, which the cards took from versions they checked.
fn dependencies(versions: impl IntoIterator<Item = (String, String)>) -> Vec<Dependency> {
    versions
        .into_iter()
        .map(|(id, version)| Dependency {
            registry: None,
            version: orbe_scene::Version::parse(&version)
                .expect("a card names only versions in the canonical form"),
            id,
        })
        .collect()
}

impl From<Card> for Item {
    /// A record leaves out the card's `used_by`: a served record never changes, and the parts
    /// that use a part grow with the registry.
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
            used_by: _,
            buy,
            lit,
            dim,
            pressable,
            holes,
        } = card;
        Item::Part(Box::new(PartItem {
            head: Head {
                schema: ITEM_SCHEMA.to_string(),
                format: Format,
                name,
                version,
                title,
                description,
                categories,
                license,
                registry_dependencies: dependencies(uses),
                files: files.into_iter().map(File::from).collect(),
                hash,
            },
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
        }))
    }
}

impl From<BlockCard> for Item {
    fn from(card: BlockCard) -> Item {
        let BlockCard {
            name,
            version,
            hash,
            title,
            description,
            categories,
            board,
            license,
            parts,
            files,
        } = card;
        Item::Block(Box::new(BlockItem {
            head: Head {
                schema: ITEM_SCHEMA.to_string(),
                format: Format,
                name,
                version,
                title,
                description,
                categories,
                license,
                registry_dependencies: dependencies(parts),
                files: files.into_iter().map(File::from).collect(),
                hash,
            },
            board,
        }))
    }
}

impl Item {
    pub fn head(&self) -> &Head {
        match self {
            Item::Part(item) => &item.head,
            Item::Block(item) => &item.head,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Item::Part(_) => Kind::Part,
            Item::Block(_) => Kind::Block,
        }
    }

    /// `<name>@<version>`, which names the record's file, `<name>@<version>.json`.
    pub fn address(&self) -> String {
        address(&self.head().name, &self.head().version)
    }

    /// The index row for this record, served as `bytes`. `listed` is false when search and the
    /// parts list leave the part out. A Block is offered.
    pub fn row(&self, bytes: &[u8], listed: bool) -> Row {
        let head = self.head();
        let (maker, number) = match self {
            Item::Part(part) => (part.real.maker.clone(), part.real.number.clone()),
            Item::Block(_) => (None, None),
        };
        Row {
            name: head.name.clone(),
            kind: self.kind(),
            version: head.version.clone(),
            title: head.title.clone(),
            description: head.description.clone(),
            categories: head.categories.clone(),
            maker,
            number,
            url: record_url(&head.name, &head.version),
            sha256: crate::sha256(bytes),
            bytes: bytes.len() as u64,
            listed,
            orbe: None,
        }
    }
}

pub(crate) fn address(name: &str, version: &str) -> String {
    format!("{name}@{version}")
}

pub(crate) fn record_url(name: &str, version: &str) -> String {
    format!("{}.json", address(name, version))
}

/// The index: which registry this is, and one row per record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "Registry index")]
pub struct Index {
    /// Where an editor finds the index's schema: `index.schema.json`, beside the index.
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: Format,
    /// The registry's name, such as `@orbefab`.
    #[schemars(pattern(pattern::REGISTRY))]
    pub name: String,
    /// The registry's page for people. Absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(with = "String", pattern(pattern::HTTPS))]
    pub homepage: Option<String>,
    /// One row per record, by name and then by version, oldest first.
    pub items: Vec<Row>,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Part,
    Block,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Part => "part",
            Kind::Block => "Block",
        })
    }
}

/// One record, with what search shows and where to fetch it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Row {
    #[schemars(pattern(pattern::ID))]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    #[schemars(pattern(pattern::VERSION))]
    pub version: String,
    #[schemars(length(min = 1))]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(with = "String")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(with = "Vec<String>")]
    pub categories: Option<Vec<String>>,
    /// The part's maker. Absent on a Block, and on a part that names none.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(with = "String")]
    pub maker: Option<String>,
    /// The part's number. Absent on a Block, and on a part that names none.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(with = "String")]
    pub number: Option<String>,
    /// Whether search and the parts list offer this part. Absent means they do. Present only
    /// as `false`, when they leave the part out.
    #[serde(default = "listed_when_absent", skip_serializing_if = "is_offered")]
    pub listed: bool,
    /// The record's URL relative to the index's: `<name>@<version>.json`.
    #[schemars(pattern(pattern::RECORD_URL))]
    pub url: String,
    /// The SHA-256 of the record's bytes, as lowercase hex.
    #[schemars(pattern(pattern::SHA256))]
    pub sha256: String,
    pub bytes: u64,
    /// The lowest orbe that reads this record, such as `0.2.0`. Absent when the registry was
    /// built from a working tree: every orbe may try the record, and the part reader still
    /// refuses a format it does not know. A registry built from release tags sets it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(with = "String", pattern(pattern::VERSION))]
    pub orbe: Option<String>,
}

fn listed_when_absent() -> bool {
    true
}

fn is_offered(listed: &bool) -> bool {
    *listed
}
