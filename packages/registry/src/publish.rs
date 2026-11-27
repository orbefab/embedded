//! The registry a release serves: every part and Block from each release, oldest first.
//! A record at `id@version` is the one from the oldest release that contains it. A later
//! release does not replace those bytes. Each index row names the lowest orbe that reads the
//! record and every record it depends on, all the way down, and takes `listed` from the
//! newest release that contains that name. A part's own is the lowest orbe whose part formats
//! include its format. A format no release reads names the next release, the patch after the
//! newest one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use orbe_scene::{Missing, Version};

use crate::item::{Index, Item, Kind};
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
    let mut listed_by_name: BTreeMap<String, bool> = BTreeMap::new();
    for &index in &order {
        let release = &releases[index];
        for format in &release.formats {
            format_orbe
                .entry(*format)
                .or_insert_with(|| release.version.clone());
        }
        for row in index_of(&release.registry)?.items {
            listed_by_name.insert(row.name, row.listed);
        }
    }
    let newest = &releases[*order.last().expect("releases is not empty")];
    let unread = next_patch(&newest.version);

    let mut output = BTreeMap::new();
    let mut rows = Vec::new();
    let mut owns = Vec::new();
    let mut deps = Vec::new();
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
                let listed = listed_by_name
                    .get(&item.head().name)
                    .copied()
                    .unwrap_or(true);
                rows.push(item.row(&record, listed));
                owns.push(own_orbe(
                    item,
                    &release.registry,
                    &release.version,
                    &unread,
                    &format_orbe,
                ));
                deps.push(
                    item.head()
                        .registry_dependencies
                        .iter()
                        .filter(|dep| dep.registry.is_none())
                        .map(|dep| (dep.id.clone(), dep.version.to_string()))
                        .collect::<Vec<_>>(),
                );
                output.insert(record_name, record);
            }
        }
    }
    let at: BTreeMap<(String, String), usize> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| ((row.name.clone(), row.version.clone()), i))
        .collect();
    for (i, row) in rows.iter_mut().enumerate() {
        let mut stack = vec![i];
        let mut reached = BTreeSet::from([i]);
        let mut best = owns[i].own;
        while let Some(next) = stack.pop() {
            for dep in &deps[next] {
                let Some(&child) = at.get(dep) else {
                    continue;
                };
                if !reached.insert(child) {
                    continue;
                }
                if let Some(version) = owns[child].own
                    && best.is_none_or(|have| version > have)
                {
                    best = Some(version);
                }
                stack.push(child);
            }
        }
        row.orbe = Some(best.unwrap_or(owns[i].fallback).to_string());
    }
    rows.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| Version::parse(&a.version).cmp(&Version::parse(&b.version)))
    });
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

/// One line for each item in `tree` that a release already published at the same version with
/// other files or other dependencies. The site keeps serving the bytes of the oldest release
/// that holds a version, so a changed item reaches nobody until its version is raised.
pub fn unbumped(releases: &[Release], tree: &Registry) -> Vec<String> {
    let mut order: Vec<&Release> = releases.iter().collect();
    order.sort_by_key(|release| Version::parse(&release.version));
    let mut served: BTreeMap<(String, String), (&str, &Item)> = BTreeMap::new();
    for release in order {
        for item in &release.registry.items {
            let key = (item.head().name.clone(), item.head().version.clone());
            served
                .entry(key)
                .or_insert((release.version.as_str(), item));
        }
    }
    let mut lines = Vec::new();
    for item in &tree.items {
        let head = item.head();
        let Some((tag, published)) = served.get(&(head.name.clone(), head.version.clone())) else {
            continue;
        };
        let was = published.head();
        let mut changed = Vec::new();
        if was.hash != head.hash {
            changed.push("files");
        }
        if was.registry_dependencies != head.registry_dependencies {
            changed.push("dependencies");
        }
        if changed.is_empty() {
            continue;
        }
        let (folder, file) = match item.kind() {
            Kind::Part => ("parts", "part.jsonc"),
            Kind::Block => ("blocks", "block.jsonc"),
        };
        lines.push(format!(
            "`{}` {} has other {} than v{tag} published at that version, and the site keeps serving v{tag}'s. raise the version in {folder}/{}/{file}",
            head.name,
            head.version,
            changed.join(" and "),
            head.name,
        ));
    }
    lines
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

/// The orbe an item needs by itself. A part needs the lowest orbe that reads its format. A
/// Block needs only what its parts need, and falls back to the release that published it when
/// it depends on no part in this registry. A part with no readable format needs the release
/// that published it.
struct Own {
    own: Option<Version>,
    fallback: Version,
}

fn own_orbe(
    item: &Item,
    registry: &Registry,
    publishing: &str,
    unread: &str,
    format_orbe: &BTreeMap<u32, String>,
) -> Own {
    let fallback = Version::parse(publishing).expect("publish checked each release version");
    if item.kind() == Kind::Block {
        return Own {
            own: None,
            fallback,
        };
    }
    let own = match part_format(registry, item) {
        Some(format) => {
            let text = format_orbe.get(&format).map_or(unread, String::as_str);
            Version::parse(text).unwrap_or(fallback)
        }
        None => fallback,
    };
    Own {
        own: Some(own),
        fallback,
    }
}

/// The patch after `version`. A format no release reads names this, so the newest
/// published orbe does not install that part.
fn next_patch(version: &str) -> String {
    let Some((head, patch)) = version.rsplit_once('.') else {
        return version.to_string();
    };
    let Ok(patch) = patch.parse::<u64>() else {
        return version.to_string();
    };
    format!("{head}.{}", patch + 1)
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
