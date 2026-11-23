//! The registry a release serves: every part and Block from each release, oldest first.
//! A record at `id@version` is the one from the oldest release that contains it. A later
//! release does not replace those bytes. Each index row names the lowest orbe whose part
//! formats include that record's format.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use orbe_scene::{Missing, Version};

use crate::item::{Index, Item, Kind, Row};
use crate::{
    BlockGaps, Error, Gaps, INDEX, INDEX_SCHEMA, ITEM_SCHEMA, Notice, Registry, Report, json,
    validate_index,
};

/// One release tag: the orbe version it shipped, the part formats that orbe reads, and the
/// registry `orbe registry build` writes from that tag's `parts/`, `blocks/` and `registry.jsonc`.
pub struct Release {
    /// `0.2.0`, with no leading `v`.
    pub version: String,
    /// `PART_FORMATS` from that tag's `packages/scene/src/model.rs`.
    pub formats: Vec<u32>,
    pub registry: Registry,
}

/// The files the site serves for `releases`. The oldest release keeps a version's record.
pub fn publish(releases: &[Release]) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    if releases.is_empty() {
        return Err(Error::Invalid(
            "no release tag to publish. The registry is built from tags v0.2.0 and later".into(),
        ));
    }
    for release in releases {
        if Version::parse(&release.version).is_none() {
            return Err(Error::Invalid(format!(
                "`{}` is not an orbe version. A release is three numbers, such as 0.2.0",
                release.version
            )));
        }
    }
    let mut order: Vec<usize> = (0..releases.len()).collect();
    order.sort_by_key(|&index| Version::parse(&releases[index].version));
    let mut format_orbe: BTreeMap<u32, String> = BTreeMap::new();
    for &index in &order {
        let release = &releases[index];
        for format in &release.formats {
            format_orbe
                .entry(*format)
                .or_insert_with(|| release.version.clone());
        }
    }

    let mut output = BTreeMap::new();
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for &index in &order {
        let release = &releases[index];
        for kind in [Kind::Part, Kind::Block] {
            for item in &release.registry.items {
                if item.kind() != kind {
                    continue;
                }
                let key = (item.head().name.clone(), item.head().version.clone());
                if !seen.insert(key) {
                    continue;
                }
                let record_name = format!("{}.json", item.address());
                let Some(record) = release.registry.output.get(&record_name).cloned() else {
                    return Err(Error::Invalid(format!(
                        "{} has no record for {}",
                        release.version,
                        item.address()
                    )));
                };
                for file in &item.head().files {
                    let Some(bytes) = release.registry.output.get(&file.url) else {
                        return Err(Error::Invalid(format!(
                            "{} is missing {}",
                            item.address(),
                            file.url
                        )));
                    };
                    output
                        .entry(file.url.clone())
                        .or_insert_with(|| bytes.clone());
                }
                let mut row = item.row(&record, listed(&release.registry, &item.head().name));
                row.orbe = Some(orbe_for(
                    item,
                    &release.registry,
                    &release.version,
                    &format_orbe,
                    &rows,
                ));
                output.insert(record_name, record);
                rows.push(row);
            }
        }
    }
    rows.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| Version::parse(&a.version).cmp(&Version::parse(&b.version)))
    });
    let newest = &releases[*order.last().expect("releases is not empty")];
    copy_schema(&mut output, &newest.registry, ITEM_SCHEMA);
    copy_schema(&mut output, &newest.registry, INDEX_SCHEMA);
    let meta = index_of(&newest.registry)?;
    let index = Index {
        schema: meta.schema,
        format: meta.format,
        name: meta.name,
        homepage: meta.homepage,
        items: rows,
    };
    validate_index(&index).map_err(Error::Invalid)?;
    output.insert(format!("{INDEX}.json"), json(&index));
    Ok(output)
}

fn copy_schema(output: &mut BTreeMap<String, Vec<u8>>, registry: &Registry, name: &str) {
    if let Some(bytes) = registry.output.get(name) {
        output.insert(name.to_string(), bytes.clone());
    }
}

fn index_of(registry: &Registry) -> Result<Index, Error> {
    let Some(bytes) = registry.output.get(&format!("{INDEX}.json")) else {
        return Err(Error::Invalid(
            "a release registry has no index.json".into(),
        ));
    };
    serde_json::from_slice(bytes)
        .map_err(|error| Error::Invalid(format!("a release index does not parse: {error}")))
}

fn listed(registry: &Registry, name: &str) -> bool {
    index_of(registry)
        .ok()
        .and_then(|index| {
            index
                .items
                .into_iter()
                .find(|row| row.name == name)
                .map(|row| row.listed)
        })
        .unwrap_or(true)
}

fn orbe_for(
    item: &Item,
    registry: &Registry,
    publishing: &str,
    format_orbe: &BTreeMap<u32, String>,
    rows: &[Row],
) -> String {
    if item.kind() == Kind::Part {
        return part_format(registry, item)
            .and_then(|format| format_orbe.get(&format).cloned())
            .unwrap_or_else(|| publishing.to_string());
    }
    let mut best = None;
    for dep in &item.head().registry_dependencies {
        if dep.registry.is_some() {
            continue;
        }
        let version = dep.version.to_string();
        let Some(row) = rows
            .iter()
            .find(|row| row.name == dep.id && row.version == version)
        else {
            continue;
        };
        let Some(text) = &row.orbe else {
            continue;
        };
        let Some(parsed) = Version::parse(text) else {
            continue;
        };
        if best.is_none_or(|have: Version| parsed > have) {
            best = Some(parsed);
        }
    }
    best.map(|version| version.to_string())
        .unwrap_or_else(|| publishing.to_string())
}

/// A registry `orbe registry build` wrote into `dir`.
pub fn read_built(dir: &Path) -> Result<Registry, Error> {
    let mut output = BTreeMap::new();
    read_tree(dir, dir, &mut output)?;
    let mut items = Vec::new();
    for (path, bytes) in &output {
        if path.contains('/') || !path.ends_with(".json") || !path.contains('@') {
            continue;
        }
        let item: Item = serde_json::from_slice(bytes)
            .map_err(|error| Error::Invalid(format!("{path} is not a registry record: {error}")))?;
        items.push(item);
    }
    if items.is_empty() {
        return Err(Error::Invalid(format!("{} holds no record", dir.display())));
    }
    Ok(Registry {
        items,
        output,
        report: Report {
            parts: 0,
            blocks: 0,
            listed: 0,
            stored: 0,
            files: 0,
            bytes: 0,
            missing: Gaps {
                parts: Missing {
                    parts: 0,
                    description: 0,
                    categories: 0,
                    docs: 0,
                    datasheet_url: 0,
                    buy: 0,
                },
                blocks: BlockGaps {
                    blocks: 0,
                    description: 0,
                    categories: 0,
                },
            },
            notices: Vec::<Notice>::new(),
        },
    })
}

fn read_tree(root: &Path, dir: &Path, output: &mut BTreeMap<String, Vec<u8>>) -> Result<(), Error> {
    let entries = fs::read_dir(dir)
        .map_err(|error| Error::Io(format!("cannot read {}: {error}", dir.display())))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| Error::Io(format!("cannot read {}: {error}", dir.display())))?;
        let path = entry.path();
        if path.is_dir() {
            read_tree(root, &path, output)?;
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|error| Error::Io(error.to_string()))?;
        let key = rel.to_string_lossy().replace('\\', "/");
        let bytes = fs::read(&path)
            .map_err(|error| Error::Io(format!("cannot read {}: {error}", path.display())))?;
        output.insert(key, bytes);
    }
    Ok(())
}

fn part_format(registry: &Registry, item: &Item) -> Option<u32> {
    let file = item
        .head()
        .files
        .iter()
        .find(|file| file.path == "part.jsonc")?;
    let bytes = registry.output.get(&file.url)?;
    let text = std::str::from_utf8(bytes).ok()?;
    let value: serde_json::Value =
        jsonc_parser::parse_to_serde_value(text, &jsonc_parser::ParseOptions::default())
            .ok()
            .flatten()?;
    value
        .get("format")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}
