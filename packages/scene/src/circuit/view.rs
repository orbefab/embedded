//! What a viewer draws of a scene that reads: each instance with its placement and params, the
//! 3D fields of each part file, every wire, and the pins whose nets reach no simulated pin. The
//! field names are those of `packages/viewer/src/protocol.ts`.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Circuit, ROOT};
use crate::diagnostic::Findings;
use crate::model::{ParamKind, PinDef};
use crate::params::number;
use crate::project::{hash_files, hex};
use crate::source::FileId;
use crate::syntax;

#[derive(Clone, Debug, Serialize)]
pub struct View {
    /// Over every file the reader read.
    pub scene_hash: String,
    /// Over the netlist the engine opens. Absent on a view read without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit_hash: Option<String>,
    pub instances: Vec<Instance>,
    /// Keyed by the part's id, or the path a file names it by.
    pub parts: BTreeMap<String, PartData>,
    pub wires: Vec<Wire>,
    pub not_simulated: Vec<PinWhy>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Instance {
    pub name: String,
    pub part: String,
    /// Each number param as a number in its SI unit, and each choice as its name.
    pub params: BTreeMap<String, Value>,
    pub children: Vec<Instance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_simulated: Option<String>,
    /// `None` only for a scene instance the scene's `layout` leaves out.
    pub placement: Option<Placement>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Placement {
    pub at: [f64; 3],
    pub rotate: [f64; 3],
}

#[derive(Clone, Debug, Serialize)]
pub struct PartData {
    /// The GLB as a URL path from the project folder, such as `/parts/led-5mm/led-5mm.glb`.
    pub model: Option<String>,
    /// `<id>.anchors.json`'s `anchors`, unchanged.
    pub anchors: Vec<Value>,
    pub leads: BTreeMap<String, Vec<u64>>,
    pub labels: BTreeMap<String, Vec<String>>,
    pub view: Value,
    pub joints: BTreeMap<String, Joint>,
    pub controls: Controls,
    pub readings: Readings,
    pub tints: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_simulated: Vec<Value>,
    /// A board's strips, each the holes one clip joins, by hole pin.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub strips: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Joint {
    pub node: String,
    pub slide: [f64; 3],
    /// In mm.
    pub range: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Controls {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub press: Option<Press>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Press {
    pub joint: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Readings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brightness: Option<Full>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Full {
    /// In amperes.
    pub full: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Wire {
    pub ends: [String; 2],
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PinWhy {
    /// The ref hover shows for the pin: the outermost label that reaches it, else its own pin.
    pub r#ref: String,
    pub why: String,
}

impl Circuit<'_> {
    pub(super) fn view(&self, netlist: Option<&str>) -> View {
        let sources = &self.project.sources;
        let trees: BTreeMap<FileId, Value> = std::iter::once(self.project.scene.file)
            .chain(self.project.parts.iter().map(|p| p.file))
            .map(|file| {
                let tree = syntax::parse(sources, file, &mut Findings::default())
                    .map_or(Value::Null, |node| node.to_value());
                (file, tree)
            })
            .collect();
        let parents = self.parents();
        let mut parts = BTreeMap::new();
        for inst in 1..self.insts.len() {
            if let Some(part) = self.part(inst) {
                parts
                    .entry(part.label.clone())
                    .or_insert_with(|| self.part_data(part, &trees[&part.file]));
            }
        }
        let mut wires = written_wires(&trees[&self.project.scene.file], "");
        for inst in 1..self.insts.len() {
            if let Some(part) = self.part(inst) {
                let prefix = format!("{}.", self.insts[inst].dotted());
                wires.extend(written_wires(&trees[&part.file], &prefix));
            }
        }
        for seated in self.seated.values() {
            wires.extend(seated.contacts.iter().map(|c| Wire {
                ends: [
                    c.leaf.clone(),
                    format!("{}.{}", self.insts[c.board].dotted(), c.hole),
                ],
                kind: "contact".into(),
                color: None,
            }));
        }
        let mut idle: BTreeMap<String, String> = BTreeMap::new();
        for (&key, why) in &self.unsimulated {
            if !self.pins[key].label {
                let shown = self.hover(&parents, self.pins[key].inst, &self.pins[key].name);
                idle.entry(shown).or_insert_with(|| why.clone());
            }
        }
        View {
            scene_hash: hash_files(
                sources
                    .files()
                    .iter()
                    .map(|f| (f.display.as_str(), f.text.as_bytes())),
            ),
            circuit_hash: netlist
                .map(|text| format!("sha256:{}", hex(&Sha256::digest(text.as_bytes())))),
            instances: self.instances(ROOT, &trees),
            parts,
            wires,
            not_simulated: idle
                .into_iter()
                .map(|(r#ref, why)| PinWhy { r#ref, why })
                .collect(),
        }
    }

    pub(super) fn parents(&self) -> Vec<usize> {
        let mut parents = vec![ROOT; self.insts.len()];
        for (id, inst) in self.insts.iter().enumerate() {
            for &child in inst.children.values() {
                parents[child] = id;
            }
        }
        parents
    }

    /// The children of `parent` in the order their file lists them.
    fn instances(&self, parent: usize, trees: &BTreeMap<FileId, Value>) -> Vec<Instance> {
        let owner = match self.part(parent) {
            Some(part) => &trees[&part.file],
            None => &trees[&self.project.scene.file],
        };
        let mut ids: Vec<usize> = self.insts[parent].children.values().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| {
                let inst = &self.insts[id];
                let part = self.part(id)?;
                let name = inst.path.last().cloned().unwrap_or_default();
                let seated = self.seated.get(&id).map(|s| s.placement);
                let placement = match (
                    seated.or_else(|| placement(&owner["layout"][&name])),
                    parent,
                ) {
                    (None, ROOT) => None,
                    (placed, _) => Some(placed.unwrap_or(Placement {
                        at: [0.0; 3],
                        rotate: [0.0; 3],
                    })),
                };
                let params = inst
                    .args
                    .iter()
                    .filter_map(|(key, arg)| {
                        let value = match &part.param(key)?.kind {
                            ParamKind::Number { unit, range } => {
                                si(&number(*unit, *range, &arg.node).ok()?)?.into()
                            }
                            ParamKind::Choice(_) => Value::String(arg.node.str()?.to_string()),
                            ParamKind::File { .. } | ParamKind::Through => return None,
                        };
                        Some((key.clone(), value))
                    })
                    .collect();
                Some(Instance {
                    name,
                    part: part.label.clone(),
                    params,
                    children: self.instances(id, trees),
                    not_simulated: inst.dropped.clone(),
                    placement,
                })
            })
            .collect()
    }

    fn part_data(&self, part: &crate::model::Part, tree: &Value) -> PartData {
        let root = self.project.sources.root();
        let id = &part.label;
        let (files, store) = (self.project.files, self.project.store);
        let model = (self.catalog(part) && store.has(files, root, id, &format!("{id}.glb")))
            .then(|| format!("/parts/{id}/{id}.glb"));
        let anchors = self.anchors_of(part);
        let mut leads = BTreeMap::new();
        let mut labels = BTreeMap::new();
        for pin in &part.pins {
            let written = &tree["pins"][&pin.name];
            match &pin.def {
                PinDef::Leads => {
                    let numbers = written.as_array().into_iter().flatten();
                    leads.insert(
                        pin.name.clone(),
                        numbers.filter_map(Value::as_u64).collect(),
                    );
                }
                PinDef::Label(refs) => {
                    labels.insert(
                        pin.name.clone(),
                        refs.iter().map(|r| r.text.clone()).collect(),
                    );
                }
            }
        }
        let joints = tree["joints"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(name, joint)| Some((name.clone(), self::joint(joint)?)))
            .collect();
        let tints = tree["params"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(param, decl)| {
                let tints: BTreeMap<String, String> = decl["choices"]
                    .as_object()?
                    .iter()
                    .filter_map(|(choice, set)| {
                        Some((choice.clone(), set["tint"].as_str()?.into()))
                    })
                    .collect();
                (!tints.is_empty()).then(|| (param.clone(), tints))
            })
            .collect();
        PartData {
            model,
            anchors,
            leads,
            labels,
            view: match &tree["view"] {
                Value::Null => Value::Object(serde_json::Map::new()),
                view => view.clone(),
            },
            joints,
            controls: Controls {
                press: tree["controls"]["press"]["joint"]
                    .as_str()
                    .map(|joint| Press {
                        joint: joint.to_string(),
                    }),
            },
            readings: Readings {
                brightness: tree["readings"]["brightness"]["full"]["value"]
                    .as_f64()
                    .map(|full| Full { full }),
            },
            tints,
            not_simulated: tree["not_simulated"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            strips: part
                .holes
                .as_ref()
                .map(|h| h.strips.clone())
                .unwrap_or_default(),
        }
    }

    /// Whether a part is a catalog copy, whose folder holds its GLB and anchors.
    fn catalog(&self, part: &crate::model::Part) -> bool {
        let root = self.project.sources.root();
        let file = &self.project.sources.file(part.file).path;
        file == &root.join("parts").join(&part.label).join("part.jsonc")
    }

    /// `<id>.anchors.json`'s `anchors`, unchanged, or none for a part of the user's own.
    pub(super) fn anchors_of(&self, part: &crate::model::Part) -> Vec<Value> {
        if !self.catalog(part) {
            return Vec::new();
        }
        let root = self.project.sources.root();
        let id = &part.label;
        let (files, store) = (self.project.files, self.project.store);
        anchors(store.read(files, root, id, &format!("{id}.anchors.json")))
    }

    /// The ref hover shows for pin `pin` of `inst`: up through each parent's label that lists
    /// it, as far as one goes.
    pub(super) fn hover(&self, parents: &[usize], inst: usize, pin: &str) -> String {
        let (mut at, mut name) = (inst, pin.to_string());
        while parents[at] != ROOT {
            let parent = parents[at];
            let inner = format!(
                "{}.{name}",
                self.insts[at].path.last().map_or("", String::as_str)
            );
            let label = self.part(parent).and_then(|part| {
                part.pins.iter().find(|p| {
                    matches!(&p.def, PinDef::Label(refs) if refs.iter().any(|r| r.text == inner))
                })
            });
            let Some(label) = label else { break };
            at = parent;
            name.clone_from(&label.name);
        }
        format!("@{}.{name}", self.insts[at].dotted())
    }
}

/// A number param's value in its SI unit. The reader writes a duration in picoseconds.
fn si(written: &Value) -> Option<f64> {
    let value = written["value"].as_f64()?;
    Some(match written["unit"].as_str() {
        Some("ps") => value / 1e12,
        _ => value,
    })
}

pub(super) fn placement(written: &Value) -> Option<Placement> {
    let three = |value: &Value| -> Option<[f64; 3]> {
        let numbers: Vec<f64> = value
            .as_array()?
            .iter()
            .map(Value::as_f64)
            .collect::<Option<_>>()?;
        match numbers[..] {
            [x, y] => Some([x, y, 0.0]),
            [x, y, z] => Some([x, y, z]),
            _ => None,
        }
    };
    let at = three(&written["at"])?;
    let rotate = match &written["rotate"] {
        Value::Null => [0.0; 3],
        rotate => three(rotate)?,
    };
    Some(Placement { at, rotate })
}

/// A sliding joint, with `range` read as mm. A turning joint the viewer does not draw yet.
fn joint(written: &Value) -> Option<Joint> {
    let slide: Vec<f64> = written["slide"]
        .as_array()?
        .iter()
        .map(Value::as_f64)
        .collect::<Option<_>>()?;
    let value = written["range"]["value"].as_f64()?;
    let range = match written["range"]["unit"].as_str()? {
        "mm" => value,
        "m" => value * 1000.0,
        _ => return None,
    };
    Some(Joint {
        node: written["node"].as_str()?.to_string(),
        slide: slide.try_into().ok()?,
        range,
    })
}

fn anchors(file: Option<Vec<u8>>) -> Vec<Value> {
    file.and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|file| file["anchors"].as_array().cloned())
        .unwrap_or_default()
}

/// A file's `wires`, each end prefixed with the path to the part that holds them.
fn written_wires(tree: &Value, prefix: &str) -> Vec<Wire> {
    tree["wires"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|wire| {
            let end = |k: usize| Some(format!("{prefix}{}", wire.get(k)?.as_str()?));
            let hint = wire.get(2);
            Some(Wire {
                ends: [end(0)?, end(1)?],
                kind: hint
                    .and_then(|h| h["kind"].as_str())
                    .unwrap_or("jumper")
                    .to_string(),
                color: hint.and_then(|h| h["color"].as_str()).map(String::from),
            })
        })
        .collect()
}
