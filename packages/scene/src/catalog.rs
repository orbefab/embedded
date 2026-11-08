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
    pub(crate) fn files(&self) -> &[(&'static str, &'static [u8])] {
        &self.files
    }

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
        assert!(catalog.newest("led-5mm").unwrap().sealed);
    }

    fn built_from(id: &str) -> BTreeSet<String> {
        find(id)
            .unwrap()
            .tree
            .get("parts")
            .and_then(crate::syntax::Node::props)
            .unwrap()
            .iter()
            .filter_map(|inst| inst.value.get("type").and_then(crate::syntax::Node::str))
            .map(str::to_string)
            .collect()
    }

    fn part_uses_the_parts_it_is_built_from(id: &str) {
        let catalog = Catalog::builtin();
        let part = catalog.newest(id).unwrap();
        assert!(!part.sealed, "{id}");
        let used: BTreeSet<String> = part.uses.keys().cloned().collect();
        assert_eq!(used, built_from(id), "{id}");
        for (child, version) in &part.uses {
            assert!(
                catalog.get(child, *version).is_some(),
                "{id} uses {child}@{version}, which the catalog lacks"
            );
        }
    }

    #[test]
    fn every_part_uses_the_parts_it_is_built_from() {
        for id in Catalog::builtin().ids() {
            let original = find(id).unwrap();
            let children = original
                .tree
                .get("parts")
                .and_then(crate::syntax::Node::props);
            if children.is_some_and(|props| !props.is_empty()) {
                part_uses_the_parts_it_is_built_from(id);
            }
        }
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

    /// A GLB's named nodes, sorted, read from its JSON chunk: bytes 20 to 20 + the u32 at
    /// byte 12.
    fn glb_node_names(id: &str, glb: &[u8]) -> Vec<String> {
        assert_eq!(&glb[..4], b"glTF", "{id}: the GLB's magic");
        assert_eq!(&glb[16..20], b"JSON", "{id}: the GLB's first chunk");
        let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: Value = serde_json::from_slice(&glb[20..20 + len])
            .unwrap_or_else(|e| panic!("{id}: the GLB's JSON chunk: {e}"));
        let mut names: Vec<String> = json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|node| node["name"].as_str())
            .filter(|name| !name.is_empty())
            .map(String::from)
            .collect();
        names.sort();
        names
    }

    fn numbers(id: &str, value: &Value) -> [f64; 3] {
        let list: Vec<f64> = value
            .as_array()
            .unwrap_or_else(|| panic!("{id}: {value} is not a list"))
            .iter()
            .map(|n| {
                n.as_f64()
                    .unwrap_or_else(|| panic!("{id}: {n} is not a number"))
            })
            .collect();
        list.try_into()
            .unwrap_or_else(|_| panic!("{id}: {value} is not three numbers"))
    }

    /// The asset step's record holds for every part with a body model: the hashes and sizes
    /// match the files, the anchors cover exactly the leads in `pins` and sit inside the box,
    /// and the GLB's named nodes are exactly the nodes the merge keeps: those `view` and `joints`
    /// name, and the `node` and `also` of each leg or socket anchor, each once.
    #[test]
    fn every_body_model_matches_its_asset_record() {
        let catalog = Catalog::builtin();
        for id in catalog.ids() {
            let entry = catalog.newest(id).unwrap();
            let glb = format!("{id}.glb");
            if !entry.files.contains_key(&glb) && !entry.files.contains_key("asset.json") {
                continue;
            }
            let asset_bytes = entry
                .files
                .get("asset.json")
                .unwrap_or_else(|| panic!("{id} is missing asset.json"));
            let asset: Value = serde_json::from_slice(asset_bytes)
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
                let bytes = entry.files.get(name).unwrap_or_else(|| {
                    panic!("{id}: asset.json lists {name}, which the folder lacks")
                });
                assert_eq!(record["bytes"], bytes.len(), "{id}: size of {name}");
                assert_eq!(
                    record["sha256"],
                    hex(&Sha256::digest(bytes)),
                    "{id}: sha256 of {name}"
                );
            }

            let part = find(id).unwrap().tree.to_value();
            let anchors: Vec<Value> = match entry.files.get(&anchors_name) {
                Some(bytes) => serde_json::from_slice::<Value>(bytes).unwrap()["anchors"]
                    .as_array()
                    .unwrap()
                    .clone(),
                None => Vec::new(),
            };
            let anchor_leads: Vec<u64> = anchors
                .iter()
                .map(|anchor| anchor["lead"].as_u64().unwrap())
                .collect();
            let lead_set: BTreeSet<u64> = anchor_leads.iter().copied().collect();
            assert_eq!(
                lead_set.len(),
                anchor_leads.len(),
                "{id}: a lead has two anchors"
            );
            assert_eq!(lead_set, leads(&part["pins"]), "{id}: anchors against pins");
            assert_eq!(
                asset["checks"]["anchors"]["anchors"],
                anchors.len(),
                "{id}: the anchors the asset step checked"
            );

            let [min, max] = ["min", "max"].map(|end| numbers(id, &asset["box"][end]));
            for anchor in &anchors {
                let at = numbers(id, &anchor["at"]);
                let inside = (0..3).all(|k| min[k] - 0.01 <= at[k] && at[k] <= max[k] + 0.01);
                assert!(
                    inside,
                    "{id}: lead {}'s anchor at {at:?} is outside the box",
                    anchor["lead"]
                );
                let axis = numbers(id, &anchor["axis"]);
                let length = axis.iter().map(|v| v * v).sum::<f64>().sqrt();
                assert!(
                    (length - 1.0).abs() < 1e-9,
                    "{id}: lead {}'s axis is not a unit vector",
                    anchor["lead"]
                );
            }

            let kept: Vec<String> = asset["kept_nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node.as_str().unwrap().to_string())
                .collect();
            let in_glb = glb_node_names(id, &entry.files[&glb]);
            assert_eq!(
                in_glb, kept,
                "{id}: the GLB's named nodes against kept_nodes"
            );

            let mut policy: BTreeSet<String> =
                named_nodes(&part).into_iter().map(String::from).collect();
            for anchor in &anchors {
                if !matches!(anchor["kind"].as_str(), Some("leg" | "socket")) {
                    continue;
                }
                let also = anchor["also"].as_array().unwrap().iter();
                for node in std::iter::once(&anchor["node"]).chain(also) {
                    let node = node.as_str().unwrap();
                    let times = in_glb.iter().filter(|name| *name == node).count();
                    assert_eq!(
                        times, 1,
                        "{id}: lead {}'s node `{node}` in the GLB",
                        anchor["lead"]
                    );
                    policy.insert(node.to_string());
                }
            }
            let policy: Vec<String> = policy.into_iter().collect();
            assert_eq!(kept, policy, "{id}: kept_nodes against the keep policy");
        }
    }
}
