//! Part folders a caller hands in, such as the ones a browser fetched from a registry. A Block
//! on disk reads the `parts/` beside `blocks/` instead. Nothing here is built into the binary.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::diagnostic::Findings;
use crate::source::Sources;
use crate::syntax::{self, Node};

/// Part folders handed in, each file by `<id>/<path>`.
#[derive(Clone, Debug)]
pub(crate) struct Source {
    files: Arc<BTreeMap<String, Vec<u8>>>,
}

impl Default for Source {
    fn default() -> Source {
        Source::empty()
    }
}

impl Source {
    pub(crate) fn empty() -> Source {
        Source {
            files: Arc::new(BTreeMap::new()),
        }
    }

    pub(crate) fn given(parts: impl IntoIterator<Item = (String, Vec<u8>)>) -> Source {
        Source {
            files: Arc::new(parts.into_iter().collect()),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The bytes of `rel` in the handed-in copy of `id`.
    pub(crate) fn file(&self, id: &str, rel: &str) -> Option<&[u8]> {
        self.files.get(&format!("{id}/{rel}")).map(Vec::as_slice)
    }
}

/// Three numbers. The first is the major version, including versions below 1.0. Versions order
/// by number, so `0.10.0` is newer than `0.9.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub(crate) major: u64,
    pub(crate) minor: u64,
    pub(crate) patch: u64,
}

impl Version {
    /// The pattern of a version's text: three numbers of ASCII digits, none with a sign or a
    /// leading zero, so one version has one spelling.
    pub const PATTERN: &str = r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$";

    /// The version `text` spells, when it is in the one spelling [`Version::PATTERN`] allows.
    pub fn parse(text: &str) -> Option<Version> {
        let number = |part: Option<&str>| {
            let part = part?;
            let canonical = part == "0"
                || (part.starts_with(|c: char| matches!(c, '1'..='9'))
                    && part.bytes().all(|b| b.is_ascii_digit()));
            if canonical { part.parse().ok() } else { None }
        };
        let mut parts = text.split('.');
        let version = Version {
            major: number(parts.next())?,
            minor: number(parts.next())?,
            patch: number(parts.next())?,
        };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The file `rel` of the folder `id`, parsed. The error is a sentence that names the file.
pub(crate) fn jsonc(
    id: &str,
    files: &BTreeMap<String, Vec<u8>>,
    rel: &str,
) -> Result<Node, String> {
    let bytes = files
        .get(rel)
        .ok_or_else(|| format!("`{id}` has no {rel}"))?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| format!("`{id}` {rel} is not UTF-8"))?
        .to_string();
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new(rel), text);
    let mut findings = Findings::default();
    syntax::parse(&sources, file, &mut findings).ok_or_else(|| {
        let messages: Vec<String> = findings
            .into_diagnostics(&sources)
            .iter()
            .map(|d| d.message.clone())
            .collect();
        format!("`{id}` {rel} did not parse: {}", messages.join("; "))
    })
}

/// A part's `version`, which has to be three numbers.
pub(crate) fn part_version(id: &str, tree: &Node) -> Result<Version, String> {
    tree.get("version")
        .and_then(Node::str)
        .and_then(Version::parse)
        .ok_or_else(|| {
            format!("`{id}` has no version of the form `0.1.0`: three numbers, with no sign and no leading zero")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::hex;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn parts_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
    }

    fn load_folder(id: &str) -> BTreeMap<String, Vec<u8>> {
        let dir = parts_root().join(id);
        let mut files = BTreeMap::new();
        for path in crate::published::published_paths(&dir).unwrap() {
            let rel = path
                .strip_prefix(&dir)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            files.insert(rel, fs::read(&path).unwrap());
        }
        files
    }

    fn part_ids() -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(parts_root())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|id| parts_root().join(id).join("part.jsonc").is_file())
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn a_version_has_one_spelling_and_orders_by_number() {
        for good in ["0.1.0", "1.2.3", "10.0.0", "0.0.0"] {
            assert_eq!(Version::parse(good).unwrap().to_string(), good);
        }
        for bad in [
            "+01.2.3",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "+1.2.3",
            "-1.2.3",
            "1.2",
            "1.2.3.4",
            "",
            "1..3",
            "1.2.x",
            " 1.2.3",
            "1.2.3 ",
            "١.2.3",
            "99999999999999999999.0.0",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(Version::parse("0.10.0") > Version::parse("0.9.0"));
    }

    #[test]
    fn build_output_is_not_a_published_file() {
        let dir = tempfile::tempdir().unwrap();
        let lamp = dir.path().join("lamp");
        fs::create_dir(&lamp).unwrap();
        fs::write(lamp.join("part.jsonc"), "{}\n").unwrap();
        fs::write(lamp.join("notes.txt"), b"kept\n").unwrap();
        fs::write(lamp.join("model.py"), b"print('lamp')\n").unwrap();
        fs::write(lamp.join("asset.json"), b"{}\n").unwrap();
        fs::write(lamp.join("a.pyc"), b"bytecode").unwrap();
        fs::create_dir(lamp.join("__pycache__")).unwrap();
        fs::write(
            lamp.join("__pycache__").join("part.cpython-312.pyc"),
            b"cached",
        )
        .unwrap();
        fs::create_dir(lamp.join(".pytest_cache")).unwrap();
        fs::write(lamp.join(".pytest_cache").join("v"), b"cache").unwrap();
        fs::create_dir(lamp.join(".venv")).unwrap();
        fs::write(lamp.join(".venv").join("pyvenv.cfg"), b"home = /tmp\n").unwrap();
        let rels: Vec<String> = crate::published::published_paths(dir.path())
            .unwrap()
            .iter()
            .map(|path| {
                path.strip_prefix(dir.path())
                    .unwrap()
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();
        assert_eq!(
            rels,
            vec!["lamp/notes.txt".to_string(), "lamp/part.jsonc".to_string()]
        );
    }

    #[test]
    fn every_part_uses_the_parts_it_is_built_from() {
        let ids = part_ids();
        let known: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
        for id in &ids {
            let files = load_folder(id);
            let tree = jsonc(id, &files, "part.jsonc").unwrap();
            let children = tree
                .get("parts")
                .and_then(crate::syntax::Node::props)
                .unwrap_or_default();
            if children.is_empty() {
                continue;
            }
            let built: BTreeSet<String> = children
                .iter()
                .filter_map(|inst| inst.value.get("type").and_then(crate::syntax::Node::str))
                .filter(|ty| !ty.starts_with('.') && !ty.starts_with('@'))
                .map(str::to_string)
                .collect();
            let used: BTreeSet<String> = tree
                .get("uses")
                .and_then(crate::syntax::Node::props)
                .unwrap_or_default()
                .iter()
                .map(|prop| prop.key.clone())
                .collect();
            assert_eq!(used, built, "{id}");
            for prop in tree
                .get("uses")
                .and_then(crate::syntax::Node::props)
                .unwrap_or_default()
            {
                assert!(known.contains(prop.key.as_str()), "{id} uses {}", prop.key);
                let child = jsonc(&prop.key, &load_folder(&prop.key), "part.jsonc").unwrap();
                assert_eq!(
                    child.get("version").and_then(crate::syntax::Node::str),
                    prop.value.str(),
                    "{id} uses {}",
                    prop.key
                );
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
        let entries = ["glow", "tint", "screen"]
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

    /// Every catalog part has a body model built from `model.py`, and the asset step's record
    /// holds for it: the hashes and sizes match the files, the anchors cover exactly the leads in
    /// `pins` and sit inside the box, and the GLB's named nodes are exactly the nodes the merge
    /// keeps: those `view` and `joints` name, and the `node` and `also` of each leg or socket
    /// anchor, each once.
    #[test]
    fn every_body_model_matches_its_asset_record() {
        for id in part_ids() {
            let id = id.as_str();
            let files = load_folder(id);
            let glb = format!("{id}.glb");
            let script_path = parts_root().join(id).join("model.py");
            assert!(
                script_path.is_file(),
                "{id} is missing model.py. Add parts/{id}/model.py and run scripts/model.sh {id}"
            );
            assert!(
                files.contains_key(&glb),
                "{id} is missing {glb}. Run scripts/model.sh {id}"
            );
            let asset_path = parts_root().join(id).join("asset.json");
            let asset_bytes = fs::read(&asset_path)
                .unwrap_or_else(|error| panic!("{id} is missing asset.json: {error}"));
            let asset: Value = serde_json::from_slice(&asset_bytes)
                .unwrap_or_else(|e| panic!("{id}: asset.json: {e}"));
            let anchors_name = format!("{id}.anchors.json");
            let mut want = vec![glb.as_str()];
            if files.contains_key(&anchors_name) {
                want.push(&anchors_name);
            }
            let listed = asset["files"].as_object().unwrap();
            let mut names: Vec<&str> = listed.keys().map(String::as_str).collect();
            names.sort();
            want.sort();
            assert_eq!(names, want, "{id}: asset.json lists these files");
            for (name, record) in listed {
                let bytes = files.get(name).unwrap_or_else(|| {
                    panic!("{id}: asset.json lists {name}, which the folder lacks")
                });
                assert_eq!(record["bytes"], bytes.len(), "{id}: size of {name}");
                assert_eq!(
                    record["sha256"],
                    hex(&Sha256::digest(bytes)),
                    "{id}: sha256 of {name}"
                );
            }
            let script = fs::read(&script_path).unwrap();
            let recorded = asset["source"]["sha256"].as_str().unwrap_or("");
            assert_eq!(
                recorded,
                hex(&Sha256::digest(script)),
                "{id}: model.py does not match asset.json. Run scripts/model.sh {id}"
            );

            let part = jsonc(id, &files, "part.jsonc").unwrap().to_value();
            let anchors: Vec<Value> = match files.get(&anchors_name) {
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
            let in_glb = glb_node_names(id, &files[&glb]);
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
                if anchor["node"].is_null() {
                    assert_eq!(
                        anchor["kind"], "socket",
                        "{id}: lead {} has no node, which only a socket cut into the body may",
                        anchor["lead"]
                    );
                }
                let also = anchor["also"].as_array().unwrap().iter();
                for node in std::iter::once(&anchor["node"]).chain(also) {
                    let Some(node) = node.as_str() else { continue };
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
