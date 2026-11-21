//! One tree of instances from the files, then pins joined into nets, and netlist v0.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::{Map, Value, json};

use orbe_engine::blob::Blobs;
use orbe_engine::world::Quantity;

use crate::diagnostic::{Code, Findings, Reported};
use crate::model::{Body, Comp, ParamDecl, ParamKind, Part, PinDef, Ref, Scene, passed};
use crate::name::did_you_mean;
use crate::params::{Arg, Cover, blocked, convert, cover, dotted, set, tokens, within};
use crate::project::{Owner, Project};
use crate::source::Span;
use crate::syntax::Node;
use crate::units::Unit;

mod board;
mod engine;
mod lint;
mod map;
mod plug;
mod view;

pub use board::{BoardFacts, BoardLed, Gap, Supply, board_facts, one_part};
pub use map::{Amount, EnginePart, MapPart, Reading, RefMap};
pub use view::{Instance, PartData, PinWhy, Placement, View, Wire};

const ROOT: usize = 0;
const NAME_LIMIT: usize = 64;
const GRID_PS: u64 = 1_000_000_000;
/// A shared hole whose strip still has room. The hole after `such as` is one an agent can
/// write at the finding, and each hint in one check names a different hole.
const HOLE_HINT: &str = "a hole holds one lead; move one of them to a free hole in the same strip";
/// Every hole in the strip already holds a lead. Naming a destination would be a guess:
/// the strip it picks can join another net, and the sentence would not be true on every board.
const FULL_STRIP_HINT: &str = "a hole holds one lead, and every hole in this strip is taken; free one first, for example by moving one of its leads to an empty strip and wiring that strip back to the hole it left";
/// Free holes are still empty. An earlier hint in this check named each of them, so calling
/// the strip full would be false.
const ALREADY_NAMED: &str = "a hole holds one lead; every free hole in this strip is already named by another hint in this check";

struct Inst {
    path: Vec<String>,
    /// `None` for the scene, and for an instance whose type did not resolve.
    part: Option<usize>,
    children: BTreeMap<String, usize>,
    args: BTreeMap<String, Arg>,
    /// Params that only pass a value on and get none, because a param above them is left
    /// unset at a null default: each with the `{ "param": ... }` where that unset param is
    /// passed down, and its name.
    unset: BTreeMap<String, (Span, String)>,
    /// Required params that have no value yet, when only a placement is read: a scene's
    /// instance left them out, or a param above passes one of them on. Each body such a
    /// param writes is not whole.
    waiting: BTreeSet<String>,
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
    /// A hole of a board, such as a breadboard's, which never names a net.
    hole: bool,
}

struct Circuit<'p> {
    project: &'p Project<'p>,
    out: &'p mut Findings,
    insts: Vec<Inst>,
    keys: BTreeMap<(usize, String), usize>,
    pins: Vec<PinInfo>,
    parent: Vec<usize>,
    /// The longest name past the limit at each span, and whether a `nets` entry wrote it.
    long: BTreeMap<Span, (String, bool)>,
    /// Each instance's model after its params are applied.
    built: BTreeMap<usize, (Value, SetBy)>,
    /// Instances with a finding about their own params or their part file, with its proof.
    /// Their engine findings may only repeat it, so they wait for the next run.
    failed: BTreeMap<usize, Reported>,
    /// Each pin whose net, before the scene's wires join, reaches no engine pin and has a
    /// reason, with the reason `orbe check` gives for a wire to it.
    unsimulated: BTreeMap<usize, String>,
    /// Each scene instance the scene plugs into a board, by its id.
    seated: BTreeMap<usize, plug::Seated>,
    /// Only the placement of the scene's parts is read, before a scene gives them a value for
    /// each required param and a ground. See [`crate::read_placed`].
    placing: bool,
    /// The bytes each blob param read, under their SHA-256.
    blobs: Blobs,
}

/// What [`Circuit::args`] gives an instance, field by field of [`Inst`].
struct Taken {
    args: BTreeMap<String, Arg>,
    unset: BTreeMap<String, (Span, String)>,
    waiting: BTreeSet<String>,
}

/// The value that set a model field, by the field's JSON pointer.
type SetBy = BTreeMap<String, Setter>;

/// A voltage source the parts' own wires keep on two nets: its instance, its `p` and `n` pins,
/// and a name for each net.
struct HeldApart {
    inst: usize,
    p: usize,
    n: usize,
    names: [String; 2],
}

/// A wire end that names a breadboard hole some other lead already holds.
struct Doubled {
    key: usize,
    span: Span,
    /// The part whose file writes the wire, or the scene.
    owner: usize,
    lead: String,
    other: String,
    /// The other ends of this same wire.
    own: Vec<usize>,
}

struct HoleEnd<'a> {
    key: usize,
    span: Span,
    owner: usize,
    lead: &'a str,
    own: &'a [usize],
}

#[derive(Clone)]
struct Setter {
    span: Span,
    /// The param the file sets, such as `btn1.bounce`.
    site: String,
    /// The value as written, such as `5ns`.
    shown: String,
    /// A number param's unit.
    unit: Option<Unit>,
    choice: bool,
    /// An instance set it, or a choice an instance picked, and not a default.
    explicit: bool,
}

impl Setter {
    fn new(decl: &ParamDecl, arg: &Arg) -> Setter {
        Setter {
            span: arg.node.span,
            site: arg.site.clone(),
            shown: shown(&arg.node),
            unit: match decl.kind {
                ParamKind::Number { unit, .. } => Some(unit),
                _ => None,
            },
            choice: matches!(decl.kind, ParamKind::Choice(_)),
            explicit: arg.explicit,
        }
    }
}

/// The deepest setter of `pointer` or of an object around it.
fn setter_of<'s>(set_by: &'s SetBy, pointer: &str) -> Option<&'s Setter> {
    set_by
        .iter()
        .filter(|(p, _)| within(pointer, p))
        .max_by_key(|(p, _)| p.len())
        .map(|(_, s)| s)
}

/// An engine part as the reader writes it: its instance, its name in a model map, its params,
/// and its fields.
struct Written {
    inst: usize,
    body: Option<String>,
    name: String,
    set_by: SetBy,
    fields: Value,
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
    pub refs: RefMap,
    pub view: View,
    /// The bytes of each blob the netlist names, read from the blob params' files.
    pub blobs: Blobs,
}

/// With `lint`, the lint rules' warnings on a circuit that checks go there. With `placing`,
/// only the placement of the scene's parts is read, as [`crate::read_placed`] says. With
/// `paths`, each path a file param takes goes there.
pub(crate) fn compile(
    project: &Project,
    out: &mut Findings,
    lint: Option<&mut Findings>,
    placing: bool,
    paths: Option<&mut Vec<FilePath>>,
) -> Option<Netlist> {
    let scene = &project.scene;
    let mut circuit = Circuit::new(project, out);
    circuit.placing = placing;
    circuit.instantiate(ROOT, Owner::Scene);
    if let Some(paths) = paths {
        *paths = circuit.file_paths();
    }
    circuit.make_pins();
    circuit.join_inside();
    circuit.plug();
    circuit.check_scene_wires(scene);
    circuit.apply_models();
    let nets = circuit.name_nets(scene);
    circuit.check_flat_names();
    circuit.report_long();
    let ground = circuit.ground(scene, &nets);
    let netlist = circuit.write(scene, &nets, ground)?;
    if let Some(warnings) = lint {
        circuit.lint(&nets, &netlist, warnings);
    }
    Some(netlist)
}

/// A path a file param takes in a scene or a part file it reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePath {
    /// The file that writes the path, from the project root.
    pub file: String,
    pub line: usize,
    pub column: usize,
    /// The param, such as `mcu.firmware`.
    pub site: String,
    /// The path as written, relative to the folder of `file` unless it is absolute.
    pub path: String,
}

impl Circuit<'_> {
    /// Each path a file param of a simulated instance takes: the value the instance gets, or
    /// else its part's default. These are the paths [`compile`] reads.
    fn file_paths(&self) -> Vec<FilePath> {
        let project = self.project;
        let mut paths = Vec::new();
        for inst in &self.insts[1..] {
            if inst.dropped.is_some() {
                continue;
            }
            let Some(part) = inst.part.map(|p| &project.parts[p]) else {
                continue;
            };
            for decl in &part.params {
                if !matches!(decl.kind, ParamKind::File(_)) {
                    continue;
                }
                let (node, site) = match (inst.args.get(&decl.name), &decl.default) {
                    (Some(arg), _) => (&arg.node, arg.site.clone()),
                    (None, Some(default)) => (default, format!("{}.{}", inst.dotted(), decl.name)),
                    (None, None) => continue,
                };
                let Some(path) = node.str() else {
                    continue;
                };
                let (line, column) = project.sources.line_col(node.span);
                paths.push(FilePath {
                    file: project.sources.file(node.span.file).display.clone(),
                    line,
                    column,
                    site,
                    path: path.to_string(),
                });
            }
        }
        paths.sort_by(|a, b| (&a.file, a.line, a.column).cmp(&(&b.file, b.line, b.column)));
        paths.dedup();
        paths
    }
}

/// What a viewer draws of the scene, read only as far as its instances, their params, its
/// layout and the pins no simulated pin reaches. It checks no param, firmware or ground and
/// writes no netlist, so its findings are left out.
pub(crate) fn view_only(project: &Project) -> View {
    let mut out = Findings::default();
    read_far(project, &mut out).view(None)
}

/// Instances, their pins, the joins inside each part, plugs, and which pins no simulated
/// pin reaches. [`view_only`] and a board's facts both stop here.
fn read_far<'p>(project: &'p Project<'p>, out: &'p mut Findings) -> Circuit<'p> {
    let mut circuit = Circuit::new(project, out);
    circuit.instantiate(ROOT, Owner::Scene);
    circuit.make_pins();
    circuit.join_inside();
    circuit.plug();
    circuit.check_scene_wires(&project.scene);
    circuit
}

impl<'p> Circuit<'p> {
    fn new(project: &'p Project<'p>, out: &'p mut Findings) -> Circuit<'p> {
        let root = Inst {
            path: Vec::new(),
            part: None,
            children: BTreeMap::new(),
            args: BTreeMap::new(),
            unset: BTreeMap::new(),
            waiting: BTreeSet::new(),
            dropped: None,
            top_span: Span::start_of(project.scene.file),
        };
        Circuit {
            project,
            out,
            insts: vec![root],
            keys: BTreeMap::new(),
            pins: Vec::new(),
            parent: Vec::new(),
            long: BTreeMap::new(),
            built: BTreeMap::new(),
            failed: BTreeMap::new(),
            unsimulated: BTreeMap::new(),
            seated: BTreeMap::new(),
            placing: false,
            blobs: Blobs::new(),
        }
    }

    fn part(&self, inst: usize) -> Option<&'p Part> {
        let project: &'p Project<'p> = self.project;
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
                unset: BTreeMap::new(),
                waiting: BTreeSet::new(),
                dropped,
                top_span,
            };
            let before = self.out.len();
            if let Some(part) = part {
                let taken = self.args(parent, owner, &dir, &inst, instance, &project.parts[part]);
                (inst.args, inst.unset, inst.waiting) = (taken.args, taken.unset, taken.waiting);
            }
            self.insts.push(inst);
            let id = self.insts.len() - 1;
            let part_reported = part.and_then(|p| project.parts[p].reported);
            if let Some(reported) = self.out.since(before).or(part_reported) {
                self.failed.insert(id, reported);
            }
            self.insts[parent]
                .children
                .insert(instance.name.clone(), id);
            if let Some(part) = part {
                self.instantiate(id, Owner::Part(part));
            }
        }
    }

    /// The values an instance's params take, each passed-through value replaced by the one
    /// its parent got. A parent's default is passed on as a default, and a parent param left
    /// unset with a null default leaves the child's param unset too, through any number of
    /// params that only pass it on. Returns the values, the params left unset that way, and the
    /// params still waiting for a value when only a placement is read.
    fn args(
        &mut self,
        parent: usize,
        owner: Owner,
        dir: &std::path::Path,
        inst: &Inst,
        instance: &crate::model::Instance,
        part: &Part,
    ) -> Taken {
        let project = self.project;
        let mut args = BTreeMap::new();
        let mut unset = BTreeMap::new();
        let mut waiting = BTreeSet::new();
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
                    format!("{hint}{list}"),
                );
                continue;
            }
            let Some(name) = passed(&prop.value) else {
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
                self.out.push(Code::Param, prop.value.span, "a scene sets a value; only a part passes its own param through with { \"param\": ... }", "set the value here, such as \"220\" or \"1ms\"".into());
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
                    "pass a param this part declares, or set the value directly".into(),
                );
                continue;
            };
            if self.insts[parent].waiting.contains(name) {
                waiting.insert(prop.key.clone());
                continue;
            }
            let left_unset = self.insts[parent]
                .unset
                .get(name)
                .cloned()
                .or_else(|| decl.opens().then(|| (prop.value.span, name.to_string())));
            // A null set where the default is null is that default, so it passes on as unset.
            let passed = self.insts[parent]
                .args
                .get(name)
                .filter(|arg| !(decl.opens() && arg.node.is_null()));
            if let Some(arg) = passed {
                args.insert(prop.key.clone(), arg.clone());
            } else if let Some((span, origin)) = left_unset {
                let child = part
                    .param(&prop.key)
                    .expect("an unknown param is skipped above");
                match (&child.default, &child.kind) {
                    (Some(_), _) => {}
                    (None, ParamKind::Through) => {
                        unset.insert(prop.key.clone(), (span, origin));
                    }
                    (None, _) => self.out.push(
                        Code::Param,
                        span,
                        format!(
                            "`{}` needs param `{}`, and `{origin}` passes nothing while it is unset",
                            inst.dotted(),
                            prop.key
                        ),
                        format!(
                            "give `{origin}` a default other than null, or set `{origin}` where this part is used"
                        ),
                    ),
                }
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
                if self.placing && matches!(owner, Owner::Scene) {
                    waiting.insert(decl.name.clone());
                    continue;
                }
                let names: Vec<&str> = part.params.iter().map(|p| p.name.as_str()).collect();
                self.out.push(
                    Code::Param,
                    instance.name_span,
                    format!("`{}` needs param `{}`", inst.dotted(), decl.name),
                    format!("{} takes {}", part.label, names.join(", ")),
                );
            }
        }
        Taken {
            args,
            unset,
            waiting,
        }
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
                    hole: part.holes.is_some(),
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
        match self.lookup(inst, r) {
            Ok(key) => key,
            Err((code, message, hint)) => {
                self.out.push(code, r.span, message, hint);
                None
            }
        }
    }

    /// [`resolve`] without a finding. A part wire was already resolved while the part was
    /// joined, so claiming it later must not report that miss again.
    fn lookup(&self, inst: usize, r: &Ref) -> Result<Option<usize>, (Code, String, String)> {
        let mut at = inst;
        for (i, seg) in r.segs.iter().enumerate() {
            if at != ROOT && self.insts[at].part.is_none() {
                return Ok(None);
            }
            let prefix = r.segs[..i].join(".");
            let last = i + 1 == r.segs.len();
            if last && let Some(&key) = self.keys.get(&(at, seg.clone())) {
                return Ok(Some(key));
            }
            if !last && let Some(&child) = self.insts[at].children.get(seg) {
                at = child;
                continue;
            }
            return Err(self.unresolved(at, seg, &prefix, last));
        }
        Ok(None)
    }

    fn unresolved(&self, at: usize, seg: &str, prefix: &str, last: bool) -> (Code, String, String) {
        let children: Vec<&str> = self.insts[at].children.keys().map(String::as_str).collect();
        if last && let Some(&child) = self.insts[at].children.get(seg) {
            let pins = self.part(child).map(Part::pin_list).unwrap_or_default();
            let full = if prefix.is_empty() {
                seg.to_string()
            } else {
                format!("{prefix}.{seg}")
            };
            return (
                Code::UnknownPin,
                format!("`{full}` is a part, not a pin"),
                format!("name one of its pins; its pins: {pins}"),
            );
        }
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

    /// Joins labels to their child pins and every part's own wires. A part marked
    /// `not_simulated` contributes neither: its parent's wires still reach its pins.
    fn join_inside(&mut self) {
        for inst in 1..self.insts.len() {
            if self.insts[inst].dropped.is_some() {
                continue;
            }
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
            for strip in part.holes.iter().flat_map(|h| &h.strips) {
                let keys: Vec<usize> = strip
                    .iter()
                    .filter_map(|hole| self.keys.get(&(inst, hole.clone())).copied())
                    .collect();
                for pair in keys.windows(2) {
                    self.union(pair[0], pair[1]);
                }
            }
        }
    }

    /// The holes of the strip `key` sits in, when `key` is a hole of a board.
    fn strip_of(&self, key: usize) -> Option<Vec<String>> {
        let inst = self.pins[key].inst;
        let name = self.pins[key].name.clone();
        let holes = self.part(inst)?.holes.as_ref()?;
        holes
            .strips
            .iter()
            .find(|strip| strip.iter().any(|hole| hole == &name))
            .cloned()
    }

    /// `board.hole` for the hole pin `key`.
    fn hole_label(&self, key: usize) -> String {
        let pin = &self.pins[key];
        format!("{}.{}", self.insts[pin.inst].dotted(), pin.name)
    }

    /// The nearest hole of `key`'s strip that no lead names yet, as `board.hole`.
    ///
    /// `held` is only the leads that are really there, so a hole named here stays free for a
    /// later real lead to conflict with. `reserved` is what an earlier hint in this check
    /// already named, and the hole returned is recorded there. `own` is the other ends of the
    /// wire being hinted, so the hint never names one of them.
    fn free_hole(
        &self,
        key: usize,
        held: &BTreeMap<usize, String>,
        reserved: &mut BTreeSet<usize>,
        own: &[usize],
    ) -> Option<String> {
        let strip = self.strip_of(key)?;
        let inst = self.pins[key].inst;
        let at = strip.iter().position(|hole| hole == &self.pins[key].name)?;
        let board = self.insts[inst].dotted();
        let mut best: Option<(usize, usize)> = None;
        for (index, hole) in strip.iter().enumerate() {
            let Some(&hole_key) = self.keys.get(&(inst, hole.clone())) else {
                continue;
            };
            if held.contains_key(&hole_key)
                || reserved.contains(&hole_key)
                || own.contains(&hole_key)
            {
                continue;
            }
            let distance = index.abs_diff(at);
            if best.is_none_or(|(nearest, chosen)| {
                distance < nearest
                    || (distance == nearest && hole.as_str() < strip[chosen].as_str())
            }) {
                best = Some((distance, index));
            }
        }
        let (_, index) = best?;
        let hole = &strip[index];
        let hole_key = self.keys[&(inst, hole.clone())];
        reserved.insert(hole_key);
        Some(format!("{board}.{hole}"))
    }

    /// Holes of `key`'s strip that no lead holds yet.
    fn open_holes(&self, key: usize, held: &BTreeMap<usize, String>) -> Vec<usize> {
        let Some(strip) = self.strip_of(key) else {
            return Vec::new();
        };
        let inst = self.pins[key].inst;
        strip
            .iter()
            .filter_map(|hole| self.keys.get(&(inst, hole.clone())).copied())
            .filter(|hole_key| !held.contains_key(hole_key))
            .collect()
    }

    /// `full`, a `board.hole` from the scene, written the way `owner`'s file writes a ref.
    /// A part's own wire says `bb.b1`. A scene wire to that same hole says `kit.bb.b1`.
    fn as_written(&self, owner: usize, full: &str) -> String {
        if owner == ROOT {
            return full.to_string();
        }
        let prefix = format!("{}.", self.insts[owner].dotted());
        full.strip_prefix(&prefix).unwrap_or(full).to_string()
    }

    /// How a finding names a wire. A part's refs are relative to that part, so the lead says
    /// which part they belong to.
    fn wire_lead(&self, owner: usize, ends: &[Ref; 2]) -> String {
        let end = |r: &Ref| {
            if owner == ROOT {
                format!("`{}`", r.text)
            } else {
                format!("`{}.{}`", self.insts[owner].dotted(), r.text)
            }
        };
        format!("the wire from {} to {}", end(&ends[0]), end(&ends[1]))
    }

    /// A breadboard hole holds one lead. Records a wire end that names a hole, or keeps the
    /// conflict when a plugged leg or an earlier end already names it. The hint is written
    /// once every lead is recorded, so two hints cannot name one hole. Two pins of one part
    /// in one strip is a different fault, reported later as `scene.shorted`.
    fn note_end(
        &self,
        end: HoleEnd<'_>,
        held: &mut BTreeMap<usize, String>,
        doubled: &mut Vec<Doubled>,
    ) {
        if self.strip_of(end.key).is_none() {
            return;
        }
        let Some(other) = held.get(&end.key).cloned() else {
            held.insert(end.key, end.lead.to_string());
            return;
        };
        doubled.push(Doubled {
            key: end.key,
            span: end.span,
            owner: end.owner,
            lead: end.lead.to_string(),
            other,
            own: end.own.to_vec(),
        });
    }

    /// Each part's own wires, after the legs are seated. A pin that is not a hole, such as a
    /// header pin, is not a breadboard hole and can still take more than one wire.
    fn note_part_wires(&self, held: &mut BTreeMap<usize, String>, doubled: &mut Vec<Doubled>) {
        let mut wires = Vec::new();
        for inst in 1..self.insts.len() {
            if self.insts[inst].dropped.is_some() {
                continue;
            }
            let Some(part) = self.part(inst) else {
                continue;
            };
            for wire in &part.comp.wires {
                wires.push((inst, wire.ends.clone()));
            }
        }
        for (inst, ends) in wires {
            let keys: Vec<Option<usize>> = ends
                .iter()
                .map(|r| self.lookup(inst, r).ok().flatten())
                .collect();
            let lead = self.wire_lead(inst, &ends);
            for (i, end) in ends.iter().enumerate() {
                let Some(key) = keys[i] else { continue };
                let own: Vec<usize> = keys
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .filter_map(|(_, key)| *key)
                    .collect();
                self.note_end(
                    HoleEnd {
                        key,
                        span: end.span,
                        owner: inst,
                        lead: &lead,
                        own: &own,
                    },
                    held,
                    doubled,
                );
            }
        }
    }

    fn report_doubled(&mut self, held: &BTreeMap<usize, String>, doubled: Vec<Doubled>) {
        let mut reserved = BTreeSet::new();
        for hole in doubled {
            let hint = match self.free_hole(hole.key, held, &mut reserved, &hole.own) {
                Some(free) => {
                    let free = self.as_written(hole.owner, &free);
                    format!("{HOLE_HINT}, such as {free}")
                }
                None if self.open_holes(hole.key, held).is_empty() => FULL_STRIP_HINT.to_string(),
                None => ALREADY_NAMED.to_string(),
            };
            let name = self.hole_label(hole.key);
            self.out.push(
                Code::Layout,
                hole.span,
                format!(
                    "{} names {name}, which {} already holds",
                    hole.lead, hole.other
                ),
                hint,
            );
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
        for key in 0..self.pins.len() {
            if let Some((false, Some(reason))) = sets.get(&self.find(key)) {
                self.unsimulated.insert(key, reason.clone());
            }
        }
        let mut held: BTreeMap<usize, String> = BTreeMap::new();
        for seated in self.seated.values() {
            for contact in &seated.contacts {
                held.insert(contact.hole_key, format!("`{}`", contact.shown));
            }
        }
        let mut doubled = Vec::new();
        self.note_part_wires(&mut held, &mut doubled);
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
                        "wire to a pin the engine simulates".into(),
                    );
                }
            }
            let lead = self.wire_lead(ROOT, &wire.ends);
            for (i, (r, key)) in wire.ends.iter().zip(&ends).enumerate() {
                let Some(key) = *key else { continue };
                let own: Vec<usize> = ends
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .filter_map(|(_, key)| *key)
                    .collect();
                self.note_end(
                    HoleEnd {
                        key,
                        span: r.span,
                        owner: ROOT,
                        lead: &lead,
                        own: &own,
                    },
                    &mut held,
                    &mut doubled,
                );
            }
            if let [Some(a), Some(b)] = ends[..] {
                joins.push((a, b, wire));
            }
        }
        self.report_doubled(&held, doubled);
        let mut supplies = self.held_apart();
        for (a, b, wire) in joins {
            self.union(a, b);
            let shorted: Vec<HeldApart> = supplies
                .extract_if(.., |supply| self.find(supply.p) == self.find(supply.n))
                .collect();
            for supply in &shorted {
                self.short(supply, wire);
            }
        }
    }

    /// Each voltage source whose two pins the parts' own wires keep on two nets, with a name
    /// for each of those nets. A scene wire that joins them is a short: no solve can hold two
    /// pins of one net apart. A net a diode feeds from the source's `p` net, such as the Nano's
    /// 5V rail behind its USB Schottky, is held apart from `n` the same way, since a wire from it
    /// to `n` puts the source across the diode alone.
    fn held_apart(&mut self) -> Vec<HeldApart> {
        let mut sources = Vec::new();
        let mut diodes = Vec::new();
        for inst in 1..self.insts.len() {
            let Some(part) = self.part(inst) else {
                continue;
            };
            if self.insts[inst].dropped.is_some() {
                continue;
            }
            for body in &part.bodies {
                let pin = |name: &str| {
                    body.pins
                        .iter()
                        .find(|(engine, _)| *engine == name)
                        .map(|(_, pin)| self.keys[&(inst, pin.clone())])
                };
                match body.kind.as_str() {
                    "voltage_source" => {
                        if let (Some(p), Some(n)) = (pin("p"), pin("n")) {
                            sources.push((inst, p, n));
                        }
                    }
                    "diode" => {
                        if let (Some(anode), Some(cathode)) = (pin("anode"), pin("cathode")) {
                            diodes.push((anode, cathode));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for (inst, p, n) in sources {
            if self.find(p) == self.find(n) {
                continue;
            }
            let mut held = vec![p];
            let mut more = true;
            while more {
                more = false;
                for &(anode, cathode) in &diodes {
                    let fed = held.iter().any(|&net| self.find(net) == self.find(anode));
                    let known = held.iter().any(|&net| self.find(net) == self.find(cathode));
                    if fed && !known && self.find(cathode) != self.find(n) {
                        held.push(cathode);
                        more = true;
                    }
                }
            }
            for p in held {
                out.push(HeldApart {
                    inst,
                    p,
                    n,
                    names: [self.side_name(p), self.side_name(n)],
                });
            }
        }
        out
    }

    /// A name for the net `key` is on before the scene's wires join anything: the smallest pin
    /// of a scene part on it, such as `nano.5V`, else `key` itself.
    fn side_name(&mut self, key: usize) -> String {
        let root = self.find(key);
        let mut top = Vec::new();
        for k in 0..self.pins.len() {
            let inst = &self.insts[self.pins[k].inst];
            if inst.path.len() != 1 || self.pins[k].hole || inst.dropped.is_some() {
                continue;
            }
            let name = format!("{}.{}", inst.dotted(), self.pins[k].name);
            if self.find(k) == root {
                top.push(name);
            }
        }
        top.into_iter().min().unwrap_or_else(|| {
            format!(
                "{}.{}",
                self.insts[self.pins[key].inst].dotted(),
                self.pins[key].name
            )
        })
    }

    fn short(&mut self, supply: &HeldApart, wire: &crate::model::Wire) {
        let [p, n] = &supply.names;
        let source = self.insts[supply.inst].dotted();
        let [a, b] = &wire.ends;
        let ends = [a.text.as_str(), b.text.as_str()];
        let message = if ends == [p.as_str(), n.as_str()] || ends == [n.as_str(), p.as_str()] {
            format!(
                "the wire from `{}` to `{}` shorts the supply `{source}`, which holds the two apart",
                a.text, b.text
            )
        } else {
            format!(
                "the wire from `{}` to `{}` joins `{p}` to `{n}`, which the supply `{source}` holds apart, so it shorts that supply",
                a.text, b.text
            )
        };
        self.out.push(
            Code::Engine("net.short"),
            a.span,
            message,
            format!(
                "remove the wire; a load from `{p}` to `{n}` needs a part that limits its current, such as a resistor"
            ),
        );
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
    /// smallest label, else the smallest member pin, each flattened. A hole never names a net,
    /// so a circuit on a breadboard has the names it has wired directly.
    fn name_nets(&mut self, scene: &Scene) -> BTreeMap<usize, String> {
        let mut named: BTreeMap<usize, String> = BTreeMap::new();
        let mut from_scene = BTreeSet::new();
        let mut engine: BTreeMap<usize, bool> = BTreeMap::new();
        for key in 0..self.pins.len() {
            let root = self.find(key);
            *engine.entry(root).or_default() |= self.pins[key].engine;
        }
        for (name, span, r) in &scene.nets {
            self.note_long(*span, name, true);
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
                    "name a pin the engine simulates".into(),
                );
            } else if let Some(other) = named.get(&root) {
                self.out.push(
                    Code::Duplicate,
                    *span,
                    format!("`{name}` names the net `{other}` already names"),
                    "give a net one name".into(),
                );
            } else {
                named.insert(root, name.clone());
                from_scene.insert(root);
            }
        }
        let mut labels: BTreeMap<usize, String> = BTreeMap::new();
        let mut members: BTreeMap<usize, String> = BTreeMap::new();
        for key in 0..self.pins.len() {
            if !self.kept(key) || self.pins[key].hole {
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
            if from_scene.contains(root) {
                continue;
            }
            let span = self.insts[self.pins[*root].inst].top_span;
            self.note_long(span, name, false);
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
        let hint = "name one in the scene, as \"ground\": \"nano.GND\"";
        let roots: Vec<(Option<&String>, &String)> = grounds
            .iter()
            .map(|(k, text)| (k.map(|k| self.find(k)).and_then(|r| nets.get(&r)), text))
            .collect();
        let Some((first, first_text)) = roots.first().copied() else {
            if !self.placing && self.insts[1..].iter().all(|i| i.part.is_some()) {
                self.out.push(
                    Code::Ground,
                    span,
                    "nothing in the scene names a ground",
                    hint.into(),
                );
            }
            return None;
        };
        let Some(first) = first else {
            self.out.push(
                Code::Ground,
                span,
                format!("the ground `{first_text}` is on no simulated net"),
                hint.into(),
            );
            return None;
        };
        if let Some((_, other)) = roots.iter().find(|(net, _)| *net != Some(first)) {
            self.out.push(
                Code::Ground,
                span,
                format!("the grounds `{first_text}` and `{other}` sit on different nets"),
                hint.into(),
            );
            return None;
        }
        Some(first.clone())
    }

    /// Writes netlist v0 and has the engine check it, even when the reader has findings, so
    /// one run reports both. A part the engine's schema refuses is left out with the finding
    /// that says why, and the rest are checked again.
    fn write(
        &mut self,
        scene: &Scene,
        nets: &BTreeMap<usize, String>,
        ground: Option<String>,
    ) -> Option<Netlist> {
        let mut parts = Vec::new();
        for inst in 1..self.insts.len() {
            if self.insts[inst].dropped.is_some() {
                continue;
            }
            let Some(part) = self.part(inst) else {
                continue;
            };
            let Some((model, set_by)) = self.built.get(&inst).cloned() else {
                continue;
            };
            let waiting = self.waiting_bodies(inst, part);
            for body in part
                .bodies
                .iter()
                .filter(|body| !waiting.contains(&body.name))
            {
                let mut flat = self.insts[inst].flat();
                let mut fields = model.clone();
                let mut set_here = SetBy::new();
                if let Some(name) = &body.name {
                    flat = format!("{flat}__{name}");
                    fields = model.get(name).cloned().unwrap_or(Value::Null);
                    let prefix = format!("/{name}");
                    set_here = set_by
                        .iter()
                        .filter(|(p, _)| within(p, &prefix))
                        .map(|(p, s)| (p[prefix.len()..].to_string(), s.clone()))
                        .collect();
                } else {
                    set_here.clone_from(&set_by);
                }
                let mut pins = Map::new();
                for (engine_pin, part_pin) in &body.pins {
                    let key = self.keys.get(&(inst, part_pin.clone())).copied();
                    if let Some(net) = key.map(|k| self.find(k)).and_then(|root| nets.get(&root)) {
                        pins.insert(engine_pin.to_string(), Value::String(net.clone()));
                    }
                }
                let mut object = match fields {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                object.remove("pins");
                object.insert("name".into(), Value::String(flat.clone()));
                object.insert("kind".into(), Value::String(body.kind.clone()));
                object.insert("pins".into(), Value::Object(pins));
                let mut object = Value::Object(object);
                whole_numbers(&mut object);
                parts.push(Written {
                    inst,
                    body: body.name.clone(),
                    name: flat,
                    set_by: set_here,
                    fields: object,
                });
            }
        }
        let mut names: Vec<String> = nets.values().cloned().collect();
        // A placement with no simulated net has no ground for the engine to take, and no part
        // with an engine pin for it to check.
        if self.placing && names.is_empty() {
            return None;
        }
        names.sort();
        let ground_name = ground
            .clone()
            .or_else(|| names.first().cloned())
            .unwrap_or_default();
        names.retain(|n| *n != ground_name);
        names.insert(0, ground_name.clone());
        let mut head = json!({
            "version": 0,
            "seed": scene.seed,
            "analysis": { "transient": { "stop_ps": Scene::STOP_PS, "max_step_ps": scene.max_step_ps, "grid_ps": GRID_PS } },
            "ground": ground_name,
            "nets": names,
        });
        if let Some(world) = world(scene, parts.iter().map(|part| &part.fields)) {
            head["world"] = world;
        }
        let head = Head(head);
        let clean = self.out.is_empty();
        let mut left_out: BTreeMap<usize, Reported> = BTreeMap::new();
        let text = loop {
            let kept: Vec<usize> = (0..parts.len())
                .filter(|i| !left_out.contains_key(i))
                .collect();
            let document = head.with(kept.iter().map(|&i| &parts[i].fields));
            let text = serde_json::to_string_pretty(&document).expect("a JSON value");
            let Err(diagnostics) = orbe_engine::netlist::parse_with(text.as_bytes(), &self.blobs)
            else {
                break text;
            };
            let quiet = !clean || !left_out.is_empty();
            let mut refused = false;
            for diagnostic in &diagnostics {
                let value = serde_json::to_value(diagnostic).unwrap_or_default();
                let index = value["data"]["path"]
                    .as_str()
                    .and_then(|p| p.strip_prefix("parts["))
                    .and_then(|p| p.split(']').next())
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| kept.get(i).copied());
                if diagnostic.code() == "netlist.syntax"
                    && let Some(i) = index
                {
                    let reported = match self.failed.get(&parts[i].inst) {
                        Some(reported) => *reported,
                        None => self.schema_finding(&parts[i], &head, &value),
                    };
                    left_out.insert(i, reported);
                    refused = true;
                    continue;
                }
                let origin = index.or_else(|| {
                    value["refs"]
                        .as_array()
                        .and_then(|r| r.first())
                        .and_then(Value::as_str)
                        .and_then(|r| r.strip_prefix('#'))
                        .map(|r| r.split('.').next().unwrap_or(r))
                        .and_then(|name| parts.iter().position(|w| w.name == name))
                });
                self.engine_finding(diagnostic, &value, origin.map(|i| &parts[i]), quiet);
            }
            if !refused {
                break text;
            }
        };
        if !self.out.is_empty() || ground.is_none() {
            return None;
        }
        let view = self.view(Some(&text));
        let mut refs = self.map(nets);
        refs.quiet_floating = self.quiet_floating(nets);
        Some(Netlist {
            json: text,
            parts: parts.len() - left_out.len(),
            nets: names,
            ground: ground_name,
            modified: self.project.modified.clone(),
            refs,
            view,
            blobs: std::mem::take(&mut self.blobs),
        })
    }

    /// Params are converted even when a wire is wrong, so one run reports both.
    fn apply_models(&mut self) {
        for inst in 1..self.insts.len() {
            if self.insts[inst].dropped.is_some() {
                continue;
            }
            let Some(part) = self.part(inst) else {
                continue;
            };
            let Some(model_node) = part.model.clone() else {
                continue;
            };
            let before = self.out.len();
            let (model, set_by) = self.model(inst, part, &model_node);
            if let Some(reported) = self.out.since(before) {
                self.failed.entry(inst).or_insert(reported);
            }
            // Hold back each body a waiting param writes. That field has no value yet, so
            // the body is not whole. A pointer into a body writes it, and a param that sets
            // several writes each of them. A param a parent passed down is already in
            // `waiting`, and its own pointers decide.
            // A sibling stays. Each of its engine pins sits on a pin of the part, and the
            // nets were named from every body before one is held back, so the engine checks
            // the sibling as its own part. It does not take a pin net from the body that waits.
            let every_body_waits = part
                .bodies
                .iter()
                .all(|body| self.body_waits(inst, part, body));
            if !every_body_waits {
                self.built.insert(inst, (model, set_by));
            }
        }
    }

    /// The bodies a waiting param writes, by the name `Body::name` uses. `None` is the
    /// one engine part of a model that is not a map of named parts.
    fn waiting_bodies(&self, inst: usize, part: &Part) -> BTreeSet<Option<String>> {
        part.bodies
            .iter()
            .filter(|body| self.body_waits(inst, part, body))
            .map(|body| body.name.clone())
            .collect()
    }

    /// Whether `body` is not whole: a param still waiting for a value writes it.
    fn body_waits(&self, inst: usize, part: &Part, body: &Body) -> bool {
        let waiting = &self.insts[inst].waiting;
        if waiting.is_empty() {
            return false;
        }
        part.params.iter().any(|decl| {
            waiting.contains(&decl.name)
                && decl
                    .pointers()
                    .iter()
                    .copied()
                    .any(|pointer| pointer_writes_body(pointer, body))
        })
    }

    /// Flat part names are checked even when a wire is wrong.
    fn check_flat_names(&mut self) {
        for inst in 1..self.insts.len() {
            if self.insts[inst].dropped.is_some() {
                continue;
            }
            let Some(part) = self.part(inst) else {
                continue;
            };
            if part.model.is_none() {
                continue;
            }
            for body in &part.bodies {
                let mut flat = self.insts[inst].flat();
                if let Some(name) = &body.name {
                    flat = format!("{flat}__{name}");
                }
                self.note_long(self.insts[inst].top_span, &flat, false);
            }
        }
    }

    /// Keeps the longest name past the limit at `span`. `from_nets` is a `nets` entry.
    fn note_long(&mut self, span: Span, name: &str, from_nets: bool) {
        if name.len() > NAME_LIMIT
            && self
                .long
                .get(&span)
                .is_none_or(|(kept, _)| kept.len() < name.len())
        {
            self.long.insert(span, (name.to_string(), from_nets));
        }
    }

    fn report_long(&mut self) {
        for (span, (name, from_nets)) in std::mem::take(&mut self.long) {
            let (message, hint) = if from_nets {
                (
                    format!(
                        "net name `{name}` is {} bytes, past the engine's {NAME_LIMIT}",
                        name.len()
                    ),
                    "shorten this name; a net name is at most 64 bytes",
                )
            } else {
                (
                    format!(
                        "the flat engine name `{name}` is {} bytes, past the engine's {NAME_LIMIT}",
                        name.len()
                    ),
                    "shorten this part's name; names inside it join onto it with __",
                )
            };
            self.out.push(Code::NameTooLong, span, message, hint.into());
        }
    }

    /// The part's model with every param set. Nothing here depends on the order params are
    /// declared or applied: each write is kept or dropped by what the other writes declare,
    /// the objects that end up on follow from the kept writes, and then each kept write goes
    /// in once, shallowest first.
    ///
    /// A param left unset at a null default only fills a field the model leaves out. A value
    /// an instance set beats any default, and a default that fills a required param beats any
    /// other default. Of two defaults, one that writes an object keeps what it wrote, and a
    /// deeper one fills only what it left out. Of two set values, or two defaults that fill
    /// required params, the deeper wins, unless the object around it is set off. A param whose default is null
    /// turns on the objects its value passes through, unless a write that wins over it turns
    /// one off. Then a value an instance set that did not land is a finding at that value.
    fn model(&mut self, inst: usize, part: &Part, model_node: &Node) -> (Value, SetBy) {
        let mut model = model_node.to_value();
        let dir = self
            .project
            .sources
            .file(part.file)
            .path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        let mut writes: Vec<Write> = Vec::new();
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
            // `convert` takes null only for a param whose default is null, so a null set by an
            // instance is that default and ranks as unset.
            let rank = match (arg.node.is_null(), arg.explicit, decl.default.is_none()) {
                (true, _, _) => Rank::Unset,
                (false, true, _) => Rank::Set,
                (false, false, true) => Rank::Required,
                (false, false, false) => Rank::Default,
            };
            match convert(self.project.files, decl, &arg, &mut self.blobs) {
                Ok(sets) => {
                    let setter = Setter::new(decl, &arg);
                    let opens = decl.opens() && !arg.node.is_null();
                    writes.extend(sets.into_iter().map(|(pointer, value)| Write {
                        pointer,
                        value,
                        opens,
                        rank,
                        checked: !decl.sets_refused,
                        param: decl.name.clone(),
                        setter: setter.clone(),
                    }));
                }
                Err((code, message, hint)) => self.out.push(
                    code,
                    arg.node.span,
                    format!("`{}`: {message}", arg.site),
                    hint,
                ),
            }
        }
        let mut kept: Vec<(&Write, bool)> = writes
            .iter()
            .filter_map(|w| match fate(w, &writes) {
                Fate::Dropped => None,
                Fate::Lands { opens } => Some((w, opens)),
            })
            .collect();
        kept.sort_by_key(|(w, _)| w.pointer.matches('/').count());
        // The objects that end up on, decided before any write goes in, so a write whose way
        // a deeper write turns on lands whatever order they come in.
        let on: BTreeSet<String> = kept
            .iter()
            .filter(|(_, opens)| *opens)
            .flat_map(|(w, _)| around(&w.pointer))
            .collect();
        for (w, _) in &kept {
            if w.rank == Rank::Unset && model.pointer(&w.pointer).is_some() {
                continue;
            }
            let open = around(&w.pointer).iter().all(|at| {
                on.contains(at)
                    || model
                        .pointer(at)
                        .is_some_and(|v| v.is_object() || v.is_array())
            });
            set(&mut model, &w.pointer, w.value.clone(), open);
        }
        let mut set_by = SetBy::new();
        let mut closed: Vec<(&str, &Setter)> = Vec::new();
        for (w, _) in &kept {
            let Some(landed) = model.pointer(&w.pointer) else {
                continue;
            };
            if landed.is_null() != w.value.is_null() {
                continue;
            }
            set_by.insert(w.pointer.clone(), w.setter.clone());
            if w.value.is_null() && w.rank != Rank::Unset {
                closed.push((&w.pointer, &w.setter));
            }
        }
        let mut told = BTreeSet::new();
        for w in writes
            .iter()
            .filter(|w| w.rank == Rank::Set && w.checked && !w.value.is_null())
        {
            let Some((at, found)) = blocked(&model, &w.pointer) else {
                continue;
            };
            if !told.insert(w.setter.span) {
                continue;
            }
            let site = &w.setter.site;
            let field = dotted(&at);
            let closer = closed.iter().find(|(c, _)| within(&at, c));
            let (message, hint) = match (found, closer) {
                (Some(value), _) => {
                    let by = setter_of(&set_by, &at)
                        .map(|s| format!("; `{}` writes it as {}", s.site, s.shown))
                        .unwrap_or_default();
                    (
                        format!("`{site}` has no effect: `{field}` is {value}, not an object{by}"),
                        format!("make `{field}` an object, or remove this value"),
                    )
                }
                (None, Some((_, closer))) => (
                    format!(
                        "`{site}` has no effect while `{}` is {}, which turns `{field}` off",
                        closer.site, closer.shown
                    ),
                    format!(
                        "remove this value, or give `{}` a value that keeps `{field}` on",
                        closer.site
                    ),
                ),
                (None, None) => match part.opener(&at).filter(|o| o.name != w.param) {
                    Some(opener) => {
                        let example = match &opener.kind {
                            ParamKind::Number { unit, .. } => {
                                format!(", such as {}", unit.example())
                            }
                            _ => String::new(),
                        };
                        let set_null = self.insts[inst]
                            .args
                            .get(&opener.name)
                            .filter(|a| a.explicit && a.node.is_null());
                        match set_null {
                            Some(off) => (
                                format!(
                                    "`{site}` has no effect while `{}` is null, which leaves `{field}` off",
                                    off.site
                                ),
                                format!(
                                    "give `{}` a value{example}, or remove this value",
                                    off.site
                                ),
                            ),
                            None => (
                                format!(
                                    "`{site}` has no effect while `{field}` is off; set `{}` too",
                                    opener.name
                                ),
                                format!("set `{}`{example}, or remove this value", opener.name),
                            ),
                        }
                    }
                    None => (
                        format!("`{site}` has no effect while `{field}` is off"),
                        "remove this value".into(),
                    ),
                },
            };
            self.out.push(Code::Param, w.setter.span, message, hint);
        }
        (model, set_by)
    }
}

/// Where a param's value comes from, weakest first. Each beats the ones before it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    /// Left unset at a null default, or set to that null: it fills a field the model leaves
    /// out, and turns nothing off.
    Unset,
    /// A declaration's default, or a parent's default passed down into a param that has one.
    Default,
    /// A parent's default passed down into a param that has none.
    Required,
    /// Set by an instance, or passed down from a value an instance set.
    Set,
}

/// One value a param writes into a model.
struct Write {
    pointer: String,
    value: Value,
    /// It turns on the objects on its way.
    opens: bool,
    rank: Rank,
    /// The part file's check passed its pointers. A refused one has its own finding.
    checked: bool,
    param: String,
    setter: Setter,
}

enum Fate {
    Dropped,
    /// It goes in; `opens` when it may turn on the objects on its way.
    Lands {
        opens: bool,
    },
}

/// What another param's write around `w` decides for it. A write that holds `w`'s field drops
/// `w` unless `w` ranks higher, or both rank alike above a default, where the deeper wins. A
/// write that turns `w`'s object off drops `w` when both rank alike above a default, and keeps
/// `w` from turning it on unless `w` ranks higher.
fn fate(w: &Write, writes: &[Write]) -> Fate {
    let mut opens = w.opens;
    for around in writes.iter().filter(|a| {
        a.param != w.param
            && a.rank != Rank::Unset
            && a.pointer != w.pointer
            && within(&w.pointer, &a.pointer)
    }) {
        let deeper_wins = w.rank == around.rank && w.rank > Rank::Default;
        match cover(&around.value, &w.pointer[around.pointer.len()..]) {
            Cover::Gap => {}
            Cover::Value if w.rank > around.rank || deeper_wins => {}
            Cover::Value => return Fate::Dropped,
            Cover::Off if deeper_wins => return Fate::Dropped,
            Cover::Off if w.rank > around.rank => {}
            Cover::Off => opens = false,
        }
    }
    Fate::Lands { opens }
}

/// Whether `pointer` writes `body`. This is what [`set`] writes, and [`tokens`] is how `set`
/// reads the pointer. One engine part is the whole model, so a pointer `tokens` accepts writes
/// it. In a model of named parts, the first token is the body `set` walks into. A pointer
/// `tokens` refuses writes nothing.
fn pointer_writes_body(pointer: &str, body: &Body) -> bool {
    let Ok(tokens) = tokens(pointer) else {
        return false;
    };
    match &body.name {
        None => true,
        Some(name) => tokens.first().copied() == Some(name.as_str()),
    }
}

/// The pointers of the objects `pointer` passes through, outermost first: `/a` and `/a/b` for
/// `/a/b/c`.
fn around(pointer: &str) -> Vec<String> {
    pointer
        .match_indices('/')
        .skip(1)
        .map(|(i, _)| pointer[..i].to_string())
        .collect()
}

/// Netlist v0 without its parts.
pub(super) struct Head(Value);

impl Head {
    /// The netlist with these parts.
    pub(super) fn with<'v>(&self, parts: impl IntoIterator<Item = &'v Value>) -> Value {
        let mut document = self.0.clone();
        document["parts"] = Value::Array(parts.into_iter().cloned().collect());
        document
    }
}

/// A param's value as the file wrote it, such as `5ns` or `220`.
fn shown(node: &Node) -> String {
    if let Some(text) = node.str().or(node.raw_number()) {
        return text.to_string();
    }
    match (
        node.get("value").and_then(Node::raw_number),
        node.get("unit").and_then(Node::str),
    ) {
        (Some(value), Some(unit)) => format!("{value} {unit}"),
        _ => node.what().to_string(),
    }
}

/// The netlist's `world`: each quantity the scene sets, and each one a part's model reads, at
/// its default unless the scene sets it, in table order. `None` when there is none, so a scene
/// with no world writes the netlist it wrote before there was one.
fn world<'a>(scene: &Scene, models: impl Iterator<Item = &'a Value>) -> Option<Value> {
    let mut read = BTreeSet::new();
    for model in models {
        reads(model, &mut read);
    }
    read.extend(scene.world.iter().map(|ambient| ambient.quantity));
    if read.is_empty() {
        return None;
    }
    let level = |quantity: Quantity, value: f64| json!({ "value": value, "unit": quantity.unit(), "source": "user" });
    let mut world = Map::new();
    for quantity in read {
        let set = scene
            .world
            .iter()
            .find(|ambient| ambient.quantity == quantity);
        let field = match set {
            None => level(quantity, quantity.row().default),
            Some(ambient) if ambient.steps.is_empty() => level(quantity, ambient.initial),
            Some(ambient) => json!({
                "kind": "steps",
                "initial": level(quantity, ambient.initial),
                "events": ambient.steps.iter().map(|&(at, to)| json!({
                    "t_ps": at,
                    "level": level(quantity, to),
                })).collect::<Vec<_>>(),
            }),
        };
        world.insert(quantity.name().into(), field);
    }
    Some(Value::Object(world))
}

/// Each quantity a `{"kind": "world", "quantity": ...}` field inside `value` reads, and each
/// one a device kind inside it reads.
fn reads(value: &Value, out: &mut BTreeSet<Quantity>) {
    match value {
        Value::Object(map) => {
            let kind = map.get("kind").and_then(Value::as_str);
            if kind == Some("world")
                && let Some(quantity) = map
                    .get("quantity")
                    .and_then(Value::as_str)
                    .and_then(Quantity::parse)
            {
                out.insert(quantity);
            }
            if let Some(kind) = kind {
                out.extend(orbe_engine::device::reads(kind));
            }
            map.values().for_each(|inner| reads(inner, out));
        }
        Value::Array(items) => items.iter().for_each(|inner| reads(inner, out)),
        _ => {}
    }
}

/// A finite whole number goes in as a JSON integer and any other number as a float. A field the
/// engine reads as an integer then gets one, and a float field reads either.
fn whole_numbers(value: &mut Value) {
    match value {
        Value::Number(n) => {
            let Some(f) = n.as_f64().filter(|f| n.is_f64() && f.fract() == 0.0) else {
                return;
            };
            if (0.0..18_446_744_073_709_551_616.0).contains(&f) {
                *value = Value::from(f as u64);
            } else if (-9_223_372_036_854_775_808.0..0.0).contains(&f) {
                *value = Value::from(f as i64);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(whole_numbers),
        Value::Object(map) => map.values_mut().for_each(whole_numbers),
        _ => {}
    }
}
