//! One tree of instances from the files, then pins joined into nets, and netlist v0.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::diagnostic::{Code, Findings};
use crate::model::{Comp, ParamKind, Part, PinDef, Ref, Scene};
use crate::name::did_you_mean;
use crate::params::{Arg, convert, set};
use crate::project::{Owner, Project};
use crate::source::Span;
use crate::syntax::Node;

const ROOT: usize = 0;
const NAME_LIMIT: usize = 64;
const GRID_PS: u64 = 1_000_000_000;

struct Inst {
    path: Vec<String>,
    /// `None` for the scene, and for an instance whose type did not resolve.
    part: Option<usize>,
    children: BTreeMap<String, usize>,
    args: BTreeMap<String, Arg>,
    /// Why this instance or one above it is left out.
    dropped: Option<String>,
    /// The name of the scene's instance this one sits in, for a finding about its flat name.
    top_span: Span,
}

impl Inst {
    fn flat(&self) -> String {
        self.path.join("__")
    }

    fn dotted(&self) -> String {
        self.path.join(".")
    }
}

struct PinInfo {
    inst: usize,
    name: String,
    label: bool,
    reason: Option<String>,
    engine: bool,
}

struct Circuit<'p> {
    project: &'p Project,
    out: &'p mut Findings,
    insts: Vec<Inst>,
    keys: BTreeMap<(usize, String), usize>,
    pins: Vec<PinInfo>,
    parent: Vec<usize>,
    long: BTreeMap<Span, String>,
}

/// For each param that set a model field, by the field's dotted path, its value's span and
/// the param the file sets.
type SetBy = BTreeMap<String, (Span, String)>;

/// Where an engine part came from: its instance, its name in a model map, and its params.
struct Written {
    inst: usize,
    body: Option<String>,
    set_by: SetBy,
}

/// What the reader made of a scene that reads.
pub struct Netlist {
    /// Netlist v0, pretty-printed.
    pub json: String,
    /// The number of engine parts.
    pub parts: usize,
    /// Net names, ground first, then by name.
    pub nets: Vec<String>,
    pub ground: String,
    /// Catalog copies with children and no model that differ from `parts.lock`.
    pub modified: Vec<String>,
}

pub(crate) fn compile(project: &Project, out: &mut Findings) -> Option<Netlist> {
    let scene = &project.scene;
    let root = Inst {
        path: Vec::new(),
        part: None,
        children: BTreeMap::new(),
        args: BTreeMap::new(),
        dropped: None,
        top_span: Span::start_of(scene.file),
    };
    let mut circuit = Circuit {
        project,
        out,
        insts: vec![root],
        keys: BTreeMap::new(),
        pins: Vec::new(),
        parent: Vec::new(),
        long: BTreeMap::new(),
    };
    circuit.instantiate(ROOT, Owner::Scene);
    circuit.make_pins();
    circuit.join_inside();
    circuit.check_scene_wires(scene);
    if !circuit.out.is_empty() {
        return None;
    }
    let nets = circuit.name_nets(scene);
    let ground = circuit.ground(scene, &nets)?;
    let netlist = circuit.write(scene, &nets, &ground)?;
    circuit.out.is_empty().then_some(netlist)
}

impl<'p> Circuit<'p> {
    fn part(&self, inst: usize) -> Option<&'p Part> {
        let project: &'p Project = self.project;
        self.insts[inst].part.map(|p| &project.parts[p])
    }

    fn instantiate(&mut self, parent: usize, owner: Owner) {
        let project = self.project;
        let comp: &Comp = project.comp(owner);
        let dir = project
            .sources
            .file(project.file(owner))
            .path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        for (index, instance) in comp.parts.iter().enumerate() {
            let part = project.types.get(&(owner, index)).copied();
            let mut path = self.insts[parent].path.clone();
            path.push(instance.name.clone());
            let dropped = instance
                .not_simulated
                .clone()
                .or_else(|| self.insts[parent].dropped.clone());
            let top_span = if parent == ROOT {
                instance.name_span
            } else {
                self.insts[parent].top_span
            };
            let mut inst = Inst {
                path,
                part,
                children: BTreeMap::new(),
                args: BTreeMap::new(),
                dropped,
                top_span,
            };
            if let Some(part) = part {
                inst.args = self.args(parent, owner, &dir, &inst, instance, &project.parts[part]);
            }
            self.insts.push(inst);
            let id = self.insts.len() - 1;
            self.insts[parent]
                .children
                .insert(instance.name.clone(), id);
            if let Some(part) = part {
                self.instantiate(id, Owner::Part(part));
            }
        }
    }

    /// The values an instance's params take, each passed-through value replaced by the one
    /// its parent got.
    fn args(
        &mut self,
        parent: usize,
        owner: Owner,
        dir: &std::path::Path,
        inst: &Inst,
        instance: &crate::model::Instance,
        part: &Part,
    ) -> BTreeMap<String, Arg> {
        let project = self.project;
        let mut args = BTreeMap::new();
        for prop in &instance.params {
            if part.param(&prop.key).is_none() {
                let names: Vec<&str> = part.params.iter().map(|p| p.name.as_str()).collect();
                let list = if names.is_empty() {
                    "it takes no params".to_string()
                } else {
                    format!("its params: {}", names.join(", "))
                };
                let hint = did_you_mean(&prop.key, names.iter().copied(), "");
                self.out.push(
                    Code::Param,
                    prop.key_span,
                    format!(
                        "`{}` is a {} and has no param `{}`",
                        inst.dotted(),
                        part.label,
                        prop.key
                    ),
                    Some(format!("{hint}{list}")),
                );
                continue;
            }
            let through = prop
                .value
                .props()
                .filter(|p| p.len() == 1 && p[0].key == "param")
                .and_then(|p| p[0].value.str());
            let Some(name) = through else {
                let site = format!("{}.{}", inst.dotted(), prop.key);
                args.insert(
                    prop.key.clone(),
                    Arg {
                        node: prop.value.clone(),
                        site,
                        dir: dir.to_path_buf(),
                        explicit: true,
                    },
                );
                continue;
            };
            let Owner::Part(owner_part) = owner else {
                self.out.push(Code::Param, prop.value.span, "a scene sets a value; only a part passes its own param through with { \"param\": ... }", None);
                continue;
            };
            let owner_part = &project.parts[owner_part];
            let Some(decl) = owner_part.param(name) else {
                self.out.push(
                    Code::Param,
                    prop.value.span,
                    format!(
                        "`{}` declares no param `{name}` to pass through",
                        owner_part.label
                    ),
                    None,
                );
                continue;
            };
            if let Some(arg) = self.insts[parent].args.get(name) {
                args.insert(prop.key.clone(), arg.clone());
            } else if let Some(default) = &decl.default {
                let site = format!("{}.{name}", self.insts[parent].dotted());
                let dir = project
                    .sources
                    .file(owner_part.file)
                    .path
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_default();
                args.insert(
                    prop.key.clone(),
                    Arg {
                        node: default.clone(),
                        site,
                        dir,
                        explicit: false,
                    },
                );
            }
        }
        for decl in part.params.iter().filter(|d| d.default.is_none()) {
            if !instance.params.iter().any(|p| p.key == decl.name) {
                let names: Vec<&str> = part.params.iter().map(|p| p.name.as_str()).collect();
                self.out.push(
                    Code::Param,
                    instance.name_span,
                    format!("`{}` needs param `{}`", inst.dotted(), decl.name),
                    Some(format!("{} takes {}", part.label, names.join(", "))),
                );
            }
        }
        args
    }

    fn make_pins(&mut self) {
        for inst in 0..self.insts.len() {
            let Some(part) = self.part(inst) else {
                continue;
            };
            let mut found = Vec::new();
            for pin in &part.pins {
                let listed = part
                    .not_simulated
                    .iter()
                    .find(|(_, pins)| pins.contains(&pin.name))
                    .map(|(why, _)| why.clone());
                let reason = self.insts[inst].dropped.clone().or(listed);
                found.push(PinInfo {
                    inst,
                    name: pin.name.clone(),
                    label: matches!(pin.def, PinDef::Label(_)),
                    reason,
                    engine: false,
                });
            }
            if self.insts[inst].dropped.is_none() {
                for body in &part.bodies {
                    for (_, part_pin) in &body.pins {
                        if let Some(info) = found.iter_mut().find(|p| &p.name == part_pin) {
                            info.engine = true;
                        }
                    }
                }
            }
            for info in found {
                self.keys.insert((inst, info.name.clone()), self.pins.len());
                self.parent.push(self.pins.len());
                self.pins.push(info);
            }
        }
    }

    fn find(&mut self, key: usize) -> usize {
        let mut root = key;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut at = key;
        while self.parent[at] != root {
            let next = self.parent[at];
            self.parent[at] = root;
            at = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a.max(b)] = a.min(b);
        }
    }

    /// The pin a ref names from `inst`, walking one child per segment.
    fn resolve(&mut self, inst: usize, r: &Ref) -> Option<usize> {
        let mut at = inst;
        for (i, seg) in r.segs.iter().enumerate() {
            if at != ROOT && self.insts[at].part.is_none() {
                return None;
            }
            let prefix = r.segs[..i].join(".");
            let last = i + 1 == r.segs.len();
            if last && let Some(&key) = self.keys.get(&(at, seg.clone())) {
                return Some(key);
            }
            if !last && let Some(&child) = self.insts[at].children.get(seg) {
                at = child;
                continue;
            }
            let (code, message, hint) = self.unresolved(at, seg, &prefix, last);
            self.out.push(code, r.span, message, Some(hint));
            return None;
        }
        None
    }

    fn unresolved(&self, at: usize, seg: &str, prefix: &str, last: bool) -> (Code, String, String) {
        let children: Vec<&str> = self.insts[at].children.keys().map(String::as_str).collect();
        if at == ROOT {
            let hint = did_you_mean(seg, children.iter().copied(), "");
            return (
                Code::UnknownRef,
                format!("no part named `{seg}`"),
                format!("{hint}parts: {}", children.join(", ")),
            );
        }
        let part = self.part(at).expect("a resolved instance");
        let owner = if prefix.is_empty() {
            part.label.clone()
        } else {
            format!("`{prefix}`")
        };
        if last && children.contains(&seg) {
            let full = if prefix.is_empty() {
                seg.to_string()
            } else {
                format!("{prefix}.{seg}")
            };
            return (
                Code::UnknownPin,
                format!("`{full}` is a part, not a pin"),
                "name one of its pins, such as `<part>.<pin>`".into(),
            );
        }
        if last {
            let dotted = if prefix.is_empty() {
                String::new()
            } else {
                format!("{prefix}.")
            };
            let hint = did_you_mean(seg, part.pins.iter().map(|p| p.name.as_str()), &dotted);
            let name = if prefix.is_empty() {
                "it".to_string()
            } else {
                prefix.to_string()
            };
            return (
                Code::UnknownPin,
                format!("{owner} has no pin `{seg}`"),
                format!(
                    "{hint}{name} is a {}; its pins: {}",
                    part.label,
                    part.pin_list()
                ),
            );
        }
        let hint = did_you_mean(seg, children.iter().copied(), "");
        (
            Code::UnknownRef,
            format!("{owner} has no part `{seg}`"),
            format!("{hint}its parts: {}", children.join(", ")),
        )
    }

    /// Joins labels to their child pins and every part's own wires.
    fn join_inside(&mut self) {
        for inst in 1..self.insts.len() {
            let Some(part) = self.part(inst) else {
                continue;
            };
            let labels: Vec<(String, Vec<Ref>)> = part
                .pins
                .iter()
                .filter_map(|p| match &p.def {
                    PinDef::Label(refs) => Some((p.name.clone(), refs.clone())),
                    PinDef::Leads => None,
                })
                .collect();
            let wires: Vec<[Ref; 2]> = part.comp.wires.iter().map(|w| w.ends.clone()).collect();
            for (label, refs) in labels {
                let key = self.keys[&(inst, label)];
                for r in &refs {
                    if let Some(target) = self.resolve(inst, r) {
                        self.union(key, target);
                    }
                }
            }
            for [a, b] in wires {
                if let (Some(a), Some(b)) = (self.resolve(inst, &a), self.resolve(inst, &b)) {
                    self.union(a, b);
                }
            }
        }
    }

    /// Each scene wire end is checked against the net it reaches inside its part, before the
    /// scene's own wires join anything. Then the scene's wires join.
    fn check_scene_wires(&mut self, scene: &Scene) {
        let mut sets: BTreeMap<usize, (bool, Option<String>)> = BTreeMap::new();
        let mut members: Vec<(String, usize)> = (0..self.pins.len())
            .map(|k| (self.flat_pin(k), k))
            .collect();
        members.sort();
        for (_, key) in members {
            let root = self.find(key);
            let entry = sets.entry(root).or_insert((false, None));
            entry.0 |= self.pins[key].engine;
            if entry.1.is_none() {
                entry.1 = self.pins[key].reason.clone();
            }
        }
        let mut joins = Vec::new();
        for wire in &scene.comp.wires {
            let ends: Vec<Option<usize>> =
                wire.ends.iter().map(|r| self.resolve(ROOT, r)).collect();
            for (r, key) in wire.ends.iter().zip(&ends) {
                let Some(key) = key else { continue };
                let root = self.find(*key);
                if let Some((false, Some(reason))) = sets.get(&root) {
                    self.out.push(
                        Code::NotSimulated,
                        r.span,
                        format!("{}: {reason}", r.text),
                        Some("wire to a pin the engine simulates".into()),
                    );
                }
            }
            if let [Some(a), Some(b)] = ends[..] {
                joins.push((a, b));
            }
        }
        for (a, b) in joins {
            self.union(a, b);
        }
    }

    fn flat_pin(&self, key: usize) -> String {
        format!(
            "{}__{}",
            self.insts[self.pins[key].inst].flat(),
            self.pins[key].name
        )
    }

    fn kept(&self, key: usize) -> bool {
        self.insts[self.pins[key].inst].dropped.is_none()
    }

    /// Each net with an engine pin, by its root, with its name: a `nets` name, else the
    /// smallest label, else the smallest member pin, each flattened.
    fn name_nets(&mut self, scene: &Scene) -> BTreeMap<usize, String> {
        let mut named: BTreeMap<usize, String> = BTreeMap::new();
        let mut engine: BTreeMap<usize, bool> = BTreeMap::new();
        for key in 0..self.pins.len() {
            let root = self.find(key);
            *engine.entry(root).or_default() |= self.pins[key].engine;
        }
        for (name, span, r) in &scene.nets {
            let Some(key) = self.resolve(ROOT, r) else {
                continue;
            };
            let root = self.find(key);
            if !engine.get(&root).copied().unwrap_or_default() {
                self.out.push(
                    Code::NotSimulated,
                    r.span,
                    format!(
                        "net `{name}` names `{}`, whose net has no simulated pin",
                        r.text
                    ),
                    None,
                );
            } else if let Some(other) = named.get(&root) {
                self.out.push(
                    Code::Duplicate,
                    *span,
                    format!("`{name}` names the net `{other}` already names"),
                    Some("give a net one name".into()),
                );
            } else {
                named.insert(root, name.clone());
            }
        }
        let mut labels: BTreeMap<usize, String> = BTreeMap::new();
        let mut members: BTreeMap<usize, String> = BTreeMap::new();
        for key in 0..self.pins.len() {
            if !self.kept(key) {
                continue;
            }
            let root = self.find(key);
            if !engine[&root] {
                continue;
            }
            let flat = self.flat_pin(key);
            let pool = if self.pins[key].label {
                &mut labels
            } else {
                &mut members
            };
            let slot = pool.entry(root).or_insert_with(|| flat.clone());
            if flat < *slot {
                *slot = flat;
            }
        }
        for (root, name) in labels.into_iter().chain(members) {
            named.entry(root).or_insert(name);
        }
        for (root, name) in &named {
            let span = self.insts[self.pins[*root].inst].top_span;
            self.too_long(span, name);
        }
        named
    }

    /// Every `ground` in the tree must land on one net. The scene's own, when it has one, wins.
    fn ground(&mut self, scene: &Scene, nets: &BTreeMap<usize, String>) -> Option<String> {
        let mut grounds: Vec<(Option<usize>, String)> = Vec::new();
        if let Some(r) = &scene.comp.ground {
            let key = self.resolve(ROOT, r)?;
            grounds.push((Some(key), r.text.clone()));
        } else {
            for inst in 1..self.insts.len() {
                if self.insts[inst].dropped.is_some() {
                    continue;
                }
                let Some(r) = self.part(inst).and_then(|p| p.comp.ground.clone()) else {
                    continue;
                };
                let key = self.resolve(inst, &r);
                grounds.push((key, format!("{}.{}", self.insts[inst].dotted(), r.text)));
            }
        }
        let span = scene.span;
        let hint = Some("name one in the scene, as \"ground\": \"nano.GND\"".to_string());
        let roots: Vec<(Option<&String>, &String)> = grounds
            .iter()
            .map(|(k, text)| (k.map(|k| self.find(k)).and_then(|r| nets.get(&r)), text))
            .collect();
        let Some((first, first_text)) = roots.first().copied() else {
            self.out.push(
                Code::Ground,
                span,
                "nothing in the scene names a ground",
                hint,
            );
            return None;
        };
        let Some(first) = first else {
            self.out.push(
                Code::Ground,
                span,
                format!("the ground `{first_text}` is on no simulated net"),
                hint,
            );
            return None;
        };
        if let Some((_, other)) = roots.iter().find(|(net, _)| *net != Some(first)) {
            self.out.push(
                Code::Ground,
                span,
                format!("the grounds `{first_text}` and `{other}` sit on different nets"),
                hint,
            );
            return None;
        }
        Some(first.clone())
    }

    fn write(
        &mut self,
        scene: &Scene,
        nets: &BTreeMap<usize, String>,
        ground: &str,
    ) -> Option<Netlist> {
        let mut parts = Vec::new();
        let mut origins: BTreeMap<String, Written> = BTreeMap::new();
        for inst in 1..self.insts.len() {
            if self.insts[inst].dropped.is_some() {
                continue;
            }
            let Some(part) = self.part(inst) else {
                continue;
            };
            let Some(model_node) = &part.model else {
                continue;
            };
            let (model, set_by) = self.model(inst, part, model_node);
            for body in &part.bodies {
                let mut flat = self.insts[inst].flat();
                let mut fields = model.clone();
                let mut set_here = BTreeMap::new();
                if let Some(name) = &body.name {
                    flat = format!("{flat}__{name}");
                    fields = model.get(name).cloned().unwrap_or(Value::Null);
                    let prefix = format!("{name}.");
                    set_here = set_by
                        .iter()
                        .filter_map(|(f, o)| {
                            f.strip_prefix(&prefix).map(|f| (f.to_string(), o.clone()))
                        })
                        .collect();
                } else {
                    set_here.clone_from(&set_by);
                }
                self.too_long(self.insts[inst].top_span, &flat);
                let mut pins = Map::new();
                for (engine_pin, part_pin) in &body.pins {
                    let key = self.keys[&(inst, part_pin.clone())];
                    let root = self.find(key);
                    pins.insert(engine_pin.to_string(), Value::String(nets[&root].clone()));
                }
                let mut object = match fields {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                object.remove("pins");
                object.insert("name".into(), Value::String(flat.clone()));
                object.insert("kind".into(), Value::String(body.kind.clone()));
                object.insert("pins".into(), Value::Object(pins));
                parts.push(Value::Object(object));
                origins.insert(
                    flat,
                    Written {
                        inst,
                        body: body.name.clone(),
                        set_by: set_here,
                    },
                );
            }
        }
        let mut names: Vec<String> = nets.values().filter(|n| *n != ground).cloned().collect();
        names.sort();
        names.insert(0, ground.to_string());
        let document = json!({
            "version": 0,
            "seed": scene.seed,
            "analysis": { "transient": { "stop_ps": Scene::STOP_PS, "max_step_ps": scene.max_step_ps, "grid_ps": GRID_PS } },
            "ground": ground,
            "nets": names,
            "parts": parts,
        });
        let text = serde_json::to_string_pretty(&document).expect("a JSON value");
        for (span, name) in std::mem::take(&mut self.long) {
            self.out.push(
                Code::NameTooLong,
                span,
                format!(
                    "the flat engine name `{name}` is {} bytes, past the engine's {NAME_LIMIT}",
                    name.len()
                ),
                Some("shorten this part's name; names inside it join onto it with __".into()),
            );
        }
        if !self.out.is_empty() {
            return None;
        }
        if let Err(diagnostics) = sim_core::netlist::parse(text.as_bytes()) {
            for diagnostic in diagnostics {
                self.engine_finding(&diagnostic, &origins);
            }
            return None;
        }
        Some(Netlist {
            json: text,
            parts: parts.len(),
            nets: names,
            ground: ground.to_string(),
            modified: self.project.modified.clone(),
        })
    }

    /// Keeps the longest flat name past the limit under each of the scene's instances.
    fn too_long(&mut self, span: Span, name: &str) {
        if name.len() > NAME_LIMIT && self.long.get(&span).is_none_or(|n| n.len() < name.len()) {
            self.long.insert(span, name.to_string());
        }
    }

    /// An engine finding placed at the param that set the field, or else at the field in its
    /// part file.
    fn engine_finding(
        &mut self,
        diagnostic: &sim_core::diagnostic::Diagnostic,
        origins: &BTreeMap<String, Written>,
    ) {
        let value = serde_json::to_value(diagnostic).unwrap_or_default();
        let message = value["message"].as_str().unwrap_or_default().to_string();
        let field = value["data"]["field"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let part = value["refs"]
            .as_array()
            .and_then(|r| r.first())
            .and_then(Value::as_str)
            .and_then(|r| r.strip_prefix('#'))
            .map(|r| r.split('.').next().unwrap_or(r).to_string());
        let found = part.as_ref().and_then(|p| origins.get(p));
        let Some(Written { inst, body, set_by }) = found else {
            self.out.push(
                Code::Engine(diagnostic.code()),
                Span::start_of(self.project.scene.file),
                message,
                None,
            );
            return;
        };
        if let Some((_, (span, site))) = set_by.iter().find(|(f, _)| {
            !field.is_empty() && (f.starts_with(&field) || field.starts_with(f.as_str()))
        }) {
            self.out
                .push(Code::Param, *span, format!("`{site}`: {message}"), None);
            return;
        }
        let part = self.part(*inst).expect("an engine part has a part");
        let model = part.model.as_ref().expect("an engine part has a model");
        let within = match body {
            Some(name) if field.is_empty() => name.clone(),
            Some(name) => format!("{name}.{field}"),
            None => field.clone(),
        };
        let span = if within.is_empty() {
            model.span
        } else {
            model.at_field(&within).map_or(model.span, |n| n.span)
        };
        self.out.push(
            Code::Engine(diagnostic.code()),
            span,
            format!("{}: {message}", self.insts[*inst].dotted()),
            None,
        );
    }

    /// The part's model with every param set: an instance's values first, then the defaults.
    /// A default never creates an object its pointer passes through, so a switch whose
    /// `bounce` is off keeps `"bounce": null` and drops its default `max_bounces`.
    fn model(&mut self, inst: usize, part: &Part, model_node: &Node) -> (Value, SetBy) {
        let mut model = model_node.to_value();
        let mut set_by = BTreeMap::new();
        let dir = self
            .project
            .sources
            .file(part.file)
            .path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        for explicit_pass in [true, false] {
            for decl in &part.params {
                if matches!(decl.kind, ParamKind::Through) {
                    continue;
                }
                let arg = match (self.insts[inst].args.get(&decl.name), &decl.default) {
                    (Some(arg), _) => arg.clone(),
                    (None, Some(default)) => Arg {
                        node: default.clone(),
                        site: format!("{}.{}", self.insts[inst].dotted(), decl.name),
                        dir: dir.clone(),
                        explicit: false,
                    },
                    (None, None) => continue,
                };
                if arg.explicit != explicit_pass {
                    continue;
                }
                let sets = match convert(decl, &arg) {
                    Ok(sets) => sets,
                    Err((code, message, hint)) => {
                        self.out.push(
                            code,
                            arg.node.span,
                            format!("`{}`: {message}", arg.site),
                            hint,
                        );
                        continue;
                    }
                };
                for (pointer, value) in sets {
                    let create = arg.explicit && !value.is_null();
                    match set(&mut model, &pointer, value, create) {
                        Ok(true) => {
                            set_by.insert(
                                pointer.trim_start_matches('/').replace('/', "."),
                                (arg.node.span, arg.site.clone()),
                            );
                        }
                        Ok(false) => {}
                        Err(reason) => self.out.push(
                            Code::Param,
                            arg.node.span,
                            format!("`{}`: {reason}", arg.site),
                            None,
                        ),
                    }
                }
            }
        }
        (model, set_by)
    }
}
