//! The catalog this binary was built with: the repo's `parts/`, embedded by `build.rs`. It is
//! the registry's index for the versions it holds, so a sealed copy can be compared with it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;

use crate::diagnostic::Findings;
use crate::project::{self, hash_files};
use crate::source::Sources;
use crate::syntax::{self, Node};

include!(concat!(env!("OUT_DIR"), "/catalog.rs"));

pub(crate) struct Original {
    pub(crate) version: String,
    pub(crate) hash: String,
    pub(crate) tree: Node,
    files: Vec<(&'static str, &'static [u8])>,
}

pub(crate) fn ids() -> impl Iterator<Item = String> {
    FILES
        .iter()
        .filter_map(|(path, _)| path.strip_suffix("/part.jsonc"))
        .map(str::to_string)
}

pub(crate) fn find(id: &str) -> Option<Original> {
    let prefix = format!("{id}/");
    let files: Vec<(&'static str, &'static [u8])> = FILES
        .iter()
        .filter_map(|(path, bytes)| path.strip_prefix(prefix.as_str()).map(|rel| (rel, *bytes)))
        .collect();
    let text = std::str::from_utf8(files.iter().find(|(rel, _)| *rel == "part.jsonc")?.1).ok()?;
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new("part.jsonc"), text.to_string());
    let tree = syntax::parse(&sources, file, &mut Findings::default())?;
    let version = tree.get("version")?.str()?.to_string();
    let hash = hash_files(files.iter().copied());
    Some(Original {
        version,
        hash,
        tree,
        files,
    })
}

impl Original {
    /// The files of `folder` that differ from this version's, for a message. `None` when
    /// every file matches.
    pub(crate) fn files_differing(&self, folder: &Path) -> Option<String> {
        let mut names: Vec<String> = Vec::new();
        for (rel, bytes) in &self.files {
            match fs::read(folder.join(rel)) {
                Ok(copy) if copy == *bytes => {}
                Ok(_) if *rel == "part.jsonc" => {
                    names.push("the comments or spacing of `part.jsonc`".into())
                }
                Ok(_) => names.push(format!("`{rel}`")),
                Err(_) => names.push(format!("`{rel}`, which is missing")),
            }
        }
        let mut extra = Vec::new();
        list(folder, folder, &mut extra);
        for rel in extra
            .iter()
            .filter(|rel| !self.files.iter().any(|(r, _)| r == rel))
        {
            names.push(format!("`{rel}`, which the catalog's copy does not have"));
        }
        (!names.is_empty()).then(|| names.join(", "))
    }
}

/// Three numbers. The first is the major version, including versions below 1.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version {
    pub(crate) major: u64,
    pub(crate) minor: u64,
    pub(crate) patch: u64,
}

impl Version {
    pub(crate) fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() || text.is_empty() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One published version of a part. `hash` is the folder hash `parts.lock` records.
#[derive(Clone, Debug)]
pub(crate) struct CatalogEntry {
    pub(crate) id: String,
    pub(crate) version: Version,
    pub(crate) files: BTreeMap<String, Vec<u8>>,
    pub(crate) hash: String,
    pub(crate) uses: BTreeMap<String, Version>,
    pub(crate) sealed: bool,
}

/// The parts `orbe add` can copy. The built-in catalog is this binary's `parts/`. Tests build
/// one with several versions of an id.
#[derive(Clone, Debug)]
pub(crate) struct Catalog {
    registry: String,
    entries: BTreeMap<(String, Version), CatalogEntry>,
}

impl Catalog {
    pub(crate) fn official() -> Catalog {
        Catalog {
            registry: project::REGISTRY.to_string(),
            entries: BTreeMap::new(),
        }
    }

    /// The repo's `parts/`, one version of each id, embedded at build time.
    pub(crate) fn builtin() -> Catalog {
        let mut groups: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
        for (path, bytes) in FILES {
            let Some((id, rel)) = path.split_once('/') else {
                continue;
            };
            groups
                .entry(id.to_string())
                .or_default()
                .insert(rel.to_string(), bytes.to_vec());
        }
        let mut catalog = Catalog::official();
        for (id, files) in groups {
            let entry = entry_from_files(&id, files).expect("the built-in catalog parses");
            catalog.insert(entry);
        }
        catalog
    }

    pub(crate) fn registry(&self) -> &str {
        &self.registry
    }

    pub(crate) fn insert(&mut self, entry: CatalogEntry) {
        self.entries
            .insert((entry.id.clone(), entry.version), entry);
    }

    pub(crate) fn get(&self, id: &str, version: Version) -> Option<&CatalogEntry> {
        self.entries.get(&(id.to_string(), version))
    }

    fn of_id(&self, id: &str) -> impl Iterator<Item = &CatalogEntry> {
        self.entries
            .range(
                (
                    id.to_string(),
                    Version {
                        major: 0,
                        minor: 0,
                        patch: 0,
                    },
                )..,
            )
            .take_while(move |(key, _)| key.0 == id)
            .map(|(_, entry)| entry)
    }

    pub(crate) fn newest(&self, id: &str) -> Option<&CatalogEntry> {
        self.of_id(id).max_by_key(|entry| entry.version)
    }

    pub(crate) fn newest_in_major(&self, id: &str, major: u64) -> Option<&CatalogEntry> {
        self.of_id(id)
            .filter(|entry| entry.version.major == major)
            .max_by_key(|entry| entry.version)
    }

    pub(crate) fn ids(&self) -> BTreeSet<&str> {
        self.entries.keys().map(|(id, _)| id.as_str()).collect()
    }

    pub(crate) fn versions_of(&self, id: &str) -> Vec<Version> {
        self.of_id(id).map(|entry| entry.version).collect()
    }
}

pub(crate) fn entry_from_files(
    id: &str,
    files: BTreeMap<String, Vec<u8>>,
) -> Result<CatalogEntry, String> {
    let bytes = files
        .get("part.jsonc")
        .ok_or_else(|| format!("`{id}` has no part.jsonc"))?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| format!("`{id}` part.jsonc is not UTF-8"))?
        .to_string();
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new("part.jsonc"), text);
    let mut findings = Findings::default();
    let tree = syntax::parse(&sources, file, &mut findings).ok_or_else(|| {
        let messages: Vec<String> = findings
            .into_diagnostics(&sources)
            .iter()
            .map(|d| d.message.clone())
            .collect();
        format!("`{id}` part.jsonc did not parse: {}", messages.join("; "))
    })?;
    let version = tree
        .get("version")
        .and_then(Node::str)
        .and_then(Version::parse)
        .ok_or_else(|| format!("`{id}` has no version of the form `0.1.0`"))?;
    let uses = match tree.get("uses") {
        None => BTreeMap::new(),
        Some(node) => {
            let props = node
                .props()
                .ok_or_else(|| format!("`{id}` uses is not an object"))?;
            let mut uses = BTreeMap::new();
            for prop in props {
                let text = prop
                    .value
                    .str()
                    .ok_or_else(|| format!("`{id}` uses `{}` is not a version", prop.key))?;
                let version = Version::parse(text).ok_or_else(|| {
                    format!(
                        "`{id}` uses `{}` at `{text}`, which is not a version",
                        prop.key
                    )
                })?;
                uses.insert(prop.key.clone(), version);
            }
            uses
        }
    };
    let hash = hash_files(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    Ok(CatalogEntry {
        id: id.to_string(),
        version,
        sealed: project::is_sealed(&tree),
        files,
        hash,
        uses,
    })
}

fn list(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for path in fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
    {
        if path.is_dir() {
            list(root, &path, out);
        } else {
            out.push(crate::source::display(root, &path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{folder_hash, hex};
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    #[test]
    fn the_builtin_catalog_matches_the_parts_tree() {
        let catalog = Catalog::builtin();
        let parts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts");
        let mut ids = Vec::new();
        for entry in fs::read_dir(&parts).unwrap() {
            ids.push(entry.unwrap().file_name().into_string().unwrap());
        }
        let want: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(catalog.ids(), want);
        for id in ids {
            let entry = catalog.newest(&id).unwrap();
            assert_eq!(entry.version.to_string(), "0.1.0");
            assert_eq!(entry.hash, folder_hash(&parts.join(&id)).unwrap(), "{id}");
        }
        let nano = catalog
            .get("nano-board", Version::parse("0.1.0").unwrap())
            .unwrap();
        assert_eq!(nano.uses.len(), 14);
        assert!(!nano.sealed);
        assert!(catalog.newest("led-5mm").unwrap().sealed);
    }

    fn leads(pins: &Value) -> BTreeSet<u64> {
        let lists = pins.as_object().into_iter().flat_map(|pins| pins.values());
        lists
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_u64)
            .collect()
    }

    fn named_nodes(part: &Value) -> Vec<&str> {
        let view = &part["view"];
        let entries = ["glow", "tint"]
            .iter()
            .filter_map(|key| view[key].as_array())
            .flatten()
            .map(|entry| &entry["node"]);
        let recoloured = view["recolour"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry["nodes"].as_array())
            .flatten();
        let joints = part["joints"]
            .as_object()
            .into_iter()
            .flat_map(|joints| joints.values())
            .map(|joint| &joint["node"]);
        entries
            .chain(recoloured)
            .chain(joints)
            .map(|node| node.as_str().unwrap())
            .collect()
    }

    /// The asset step's record holds for every part with a body model: the hashes and sizes
    /// match the files, the anchors cover exactly the leads in `pins`, and the merge kept every
    /// node `view` and `joints` name.
    #[test]
    fn every_body_model_matches_its_asset_record() {
        let catalog = Catalog::builtin();
        let mut checked = 0;
        for id in catalog.ids() {
            let entry = catalog.newest(id).unwrap();
            let glb = format!("{id}.glb");
            if !entry.files.contains_key(&glb) {
                continue;
            }
            checked += 1;
            let asset: Value = serde_json::from_slice(&entry.files["asset.json"])
                .unwrap_or_else(|e| panic!("{id}: asset.json: {e}"));
            let anchors_name = format!("{id}.anchors.json");
            let mut want = vec![glb.as_str()];
            if entry.files.contains_key(&anchors_name) {
                want.push(&anchors_name);
            }
            let listed = asset["files"].as_object().unwrap();
            let mut names: Vec<&str> = listed.keys().map(String::as_str).collect();
            names.sort();
            want.sort();
            assert_eq!(names, want, "{id}: asset.json lists these files");
            for (name, record) in listed {
                let bytes = &entry.files[name];
                assert_eq!(record["bytes"], bytes.len(), "{id}: size of {name}");
                assert_eq!(
                    record["sha256"],
                    hex(&Sha256::digest(bytes)),
                    "{id}: sha256 of {name}"
                );
            }

            let part = find(id).unwrap().tree.to_value();
            let anchors: Vec<u64> = match entry.files.get(&anchors_name) {
                Some(bytes) => serde_json::from_slice::<Value>(bytes).unwrap()["anchors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|anchor| anchor["lead"].as_u64().unwrap())
                    .collect(),
                None => Vec::new(),
            };
            let anchor_set: BTreeSet<u64> = anchors.iter().copied().collect();
            assert_eq!(
                anchor_set.len(),
                anchors.len(),
                "{id}: a lead has two anchors"
            );
            assert_eq!(
                anchor_set,
                leads(&part["pins"]),
                "{id}: anchors against pins"
            );

            let kept: BTreeSet<&str> = asset["kept_nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node.as_str().unwrap())
                .collect();
            for node in named_nodes(&part) {
                assert!(kept.contains(node), "{id}: `{node}` is not a kept node");
            }
        }
        assert_eq!(checked, 18);
    }
}
