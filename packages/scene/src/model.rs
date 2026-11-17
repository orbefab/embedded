//! The files' model: what a scene or a part file says, decoded from the syntax tree. Each
//! decoder reports what one file alone can show, and keeps going after a finding.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use orbe_engine::netlist::{MIN_GAP_PS, engine_kinds};
use orbe_engine::world::Quantity;

use crate::diagnostic::{Code, Findings, Reported};
use crate::name::{NameKind, and_list, did_you_mean, problem};
use crate::params::{Cover, Step, cover, dotted, pointer_of, step, tokens, within};
use crate::shape::{self, ParamShape, schema, unknown_key};
use crate::source::{FileId, Sources, Span};
use crate::syntax::{Json, Node, Prop};
use crate::units::{self, Unit};

const SCENE_KEYS: &[&str] = &[
    "format", "$schema", "parts", "wires", "layout", "ground", "nets", "sim", "world",
];
/// Part and scene `format` values this binary reads. A part's `format` covers its `model`.
pub const FORMATS: &[u32] = &[1];
const WIRE_KINDS: &[&str] = &["jumper", "trace", "contact"];
/// A live session's stop, investigation 1's 2-hour cap.
pub const STOP_PS: u64 = 7_200 * 1_000_000_000_000;
const DEFAULT_MAX_STEP_PS: u64 = 10_000_000_000;

/// A pin ref as written, without a leading `@`, split at its dots.
#[derive(Clone, Debug)]
pub(crate) struct Ref {
    pub(crate) text: String,
    pub(crate) segs: Vec<String>,
    pub(crate) span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TypeRef {
    /// `parts/<id>/` in the project.
    Id(String),
    /// Relative to the file that names it.
    Path(String),
}

pub(crate) struct Instance {
    pub(crate) name: String,
    pub(crate) name_span: Span,
    /// `None` when the type is missing or malformed, which is already reported.
    pub(crate) ty: Option<(TypeRef, Span)>,
    pub(crate) params: Vec<Prop>,
    pub(crate) not_simulated: Option<String>,
}

/// The param a child's value passes down, when it is `{ "param": "<name>" }`.
pub(crate) fn passed(value: &Node) -> Option<&str> {
    value
        .props()
        .filter(|p| p.len() == 1 && p[0].key == "param")
        .and_then(|p| p[0].value.str())
}

pub(crate) struct Wire {
    pub(crate) ends: [Ref; 2],
}

/// What a scene and a part share.
#[derive(Default)]
pub(crate) struct Comp {
    pub(crate) parts: Vec<Instance>,
    pub(crate) wires: Vec<Wire>,
    pub(crate) ground: Option<Ref>,
}

pub(crate) enum PinDef {
    Leads,
    /// A pin defined by child pins.
    Label(Vec<Ref>),
}

pub(crate) struct Pin {
    pub(crate) name: String,
    pub(crate) def: PinDef,
}

/// One engine part of a part's model.
pub(crate) struct Body {
    /// `None` for a model with one engine part, else its name in the model map.
    pub(crate) name: Option<String>,
    pub(crate) kind: String,
    /// Each engine pin, in the kind's order, with the part pin it sits on.
    pub(crate) pins: Vec<(&'static str, String)>,
}

pub(crate) enum ParamKind {
    Number {
        unit: Unit,
        range: Range,
    },
    Choice(Vec<(String, Vec<(String, Node)>)>),
    File,
    /// No `sets`: a part with children hands it to a child with `{ "param": "<name>" }`.
    Through,
}

#[derive(Clone, Copy)]
pub(crate) struct Range {
    pub(crate) above: Option<f64>,
    pub(crate) below: Option<f64>,
}

pub(crate) struct ParamDecl {
    pub(crate) name: String,
    /// The param's key in the part file.
    pub(crate) span: Span,
    pub(crate) kind: ParamKind,
    /// `None` when the param is required.
    pub(crate) default: Option<Node>,
    pub(crate) sets: Option<String>,
    /// The part file's own check refused a pointer of this param, so its values never land.
    pub(crate) sets_refused: bool,
}

impl ParamDecl {
    /// Every pointer into `model` this param can set.
    pub(crate) fn pointers(&self) -> Vec<&str> {
        let mut pointers = Vec::new();
        for (pointer, _) in self.writes() {
            if !pointers.contains(&pointer) {
                pointers.push(pointer);
            }
        }
        pointers
    }

    /// Every pointer into `model` this param can set, with the value a choice writes there.
    fn writes(&self) -> Vec<(&str, Option<&Node>)> {
        match &self.kind {
            ParamKind::Choice(choices) => choices
                .iter()
                .flat_map(|(_, sets)| sets.iter().map(|(p, v)| (p.as_str(), Some(v))))
                .collect(),
            _ => self
                .sets
                .as_deref()
                .map(|p| (p, None))
                .into_iter()
                .collect(),
        }
    }

    /// A param whose default is null is off until it gets a value, and that value turns on
    /// the objects its pointer passes through.
    pub(crate) fn opens(&self) -> bool {
        self.default.as_ref().is_some_and(Node::is_null)
    }

    /// Whether this param writes an object or a list at `at`, or null to turn it off, so other
    /// params may write the fields inside it: a choice that writes nothing else there, or a
    /// param that turns on what it writes.
    fn holds_object(&self, at: &str) -> bool {
        match &self.kind {
            ParamKind::Choice(_) => {
                self.writes()
                    .into_iter()
                    .filter(|(p, _)| *p == at)
                    .all(|(_, v)| {
                        v.is_some_and(|v| v.is_null() || v.props().is_some() || v.items().is_some())
                    })
            }
            _ => self.opens(),
        }
    }
}

/// A part whose pins are holes a leg plugs into, such as a breadboard's.
pub(crate) struct Holes {
    /// How far a leg goes in, in mm.
    pub(crate) depth: f64,
    /// Each strip's holes, the pins one clip joins.
    pub(crate) strips: Vec<Vec<String>>,
}

pub(crate) struct Part {
    pub(crate) file: FileId,
    /// The id, or the path a scene names it by, for messages.
    pub(crate) label: String,
    pub(crate) comp: Comp,
    pub(crate) pins: Vec<Pin>,
    pub(crate) model: Option<Node>,
    pub(crate) bodies: Vec<Body>,
    pub(crate) params: Vec<ParamDecl>,
    /// Each `not_simulated` entry's reason with the pins it lists.
    pub(crate) not_simulated: Vec<(String, Vec<String>)>,
    pub(crate) joins_only: bool,
    pub(crate) holes: Option<Holes>,
    /// A finding in this part file, which the engine's findings on the part may repeat.
    pub(crate) reported: Option<Reported>,
}

impl Part {
    pub(crate) fn pin(&self, name: &str) -> Option<&Pin> {
        self.pins.iter().find(|p| p.name == name)
    }

    pub(crate) fn param(&self, name: &str) -> Option<&ParamDecl> {
        self.params.iter().find(|p| p.name == name)
    }

    /// The param that turns on the object at `off`, such as `/bounce`: one with "default": null
    /// that sets a field inside it, else one that writes the object, or an object or a list
    /// around it.
    pub(crate) fn opener(&self, off: &str) -> Option<&ParamDecl> {
        let inside = |p: &str| p != off && within(p, off);
        self.params
            .iter()
            .find(|d| d.opens() && d.pointers().into_iter().any(inside))
            .or_else(|| {
                self.params.iter().find(|d| {
                    d.writes().into_iter().any(|(p, value)| {
                        within(off, p)
                            && value.is_none_or(|v| v.props().is_some() || v.items().is_some())
                    })
                })
            })
    }

    pub(crate) fn pin_list(&self) -> String {
        self.pins
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A layout entry's `on`: one pin's leg goes into a hole, such as `"on": { "anode": "bb.f20" }`.
pub(crate) struct Plug {
    /// The scene instance plugged in.
    pub(crate) name: String,
    /// The `on` object, where a finding about the whole plug points.
    pub(crate) span: Span,
    pub(crate) pin: String,
    pub(crate) pin_span: Span,
    /// The hole, as a board's name and a hole pin.
    pub(crate) hole: Ref,
    /// The turn about Z, in degrees.
    pub(crate) turn: f64,
}

pub(crate) struct Scene {
    pub(crate) file: FileId,
    /// The scene's root object, where a finding about the whole scene points.
    pub(crate) span: Span,
    pub(crate) comp: Comp,
    pub(crate) plugs: Vec<Plug>,
    pub(crate) nets: Vec<(String, Span, Ref)>,
    pub(crate) seed: u64,
    pub(crate) max_step_ps: u64,
    /// Each world quantity the scene sets, in the order it sets them.
    pub(crate) world: Vec<Ambient>,
}

/// A world quantity the scene sets: its value at t = 0, then each step's time and value.
pub(crate) struct Ambient {
    pub(crate) quantity: Quantity,
    pub(crate) initial: f64,
    pub(crate) steps: Vec<(u64, f64)>,
}

impl Scene {
    pub(crate) const STOP_PS: u64 = STOP_PS;
}

/// Checks `format`. A file in another format is not read further. `part` is the part's label
/// when the file is a part; a scene passes `None` and keeps [`Code::Format`].
fn format_ok(file: FileId, root: &Node, out: &mut Findings, part: Option<&str>, add: &str) -> bool {
    let reads = FORMATS
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let Some(node) = root.get("format") else {
        report_format(
            out,
            part,
            Span::start_of(file),
            "the file has no `format`".into(),
            format!("add \"format\": 1; this binary reads format {reads}"),
        );
        return false;
    };
    if node
        .number()
        .is_some_and(|n| FORMATS.iter().any(|f| f64::from(*f) == n))
    {
        return true;
    }
    let (message, hint) = format_refusal(node, part, &reads, add);
    report_format(out, part, node.span, message, hint);
    false
}

fn report_format(
    out: &mut Findings,
    part: Option<&str>,
    span: Span,
    message: String,
    hint: String,
) {
    let code = if part.is_some() {
        Code::PartFormat
    } else {
        Code::Format
    };
    out.push(code, span, message, hint);
}

/// What to say when `format` is present and not one [`FORMATS`] lists. A whole number below
/// the oldest is an older orbe, and one above the newest is a newer orbe. Anything else, a
/// gap included, stays the plain refusal.
///
/// A label that is not an id was opened by path. That file is the user's. `orbe add` writes
/// `parts/<id>/`, so it would replace a catalog copy, or fail `unknown_part`, and leave this
/// file at the old format. The hint tells them to update the file, or to point the scene at
/// the catalog part when the file is a copy of one. A catalog id is named in the `orbe add`
/// the other hints print, so the folder and the quoting match.
fn format_refusal(node: &Node, part: Option<&str>, reads: &str, add: &str) -> (String, String) {
    let plain = (
        format!("format {} is not one this binary reads", node.to_value()),
        format!("this binary reads format {reads}"),
    );
    let Some(n) = whole_format(node) else {
        return plain;
    };
    let Some((min, max)) = format_bounds() else {
        return plain;
    };
    if n < min {
        let Some(label) = part else {
            return (
                format!("format {n} is written for an older orbe"),
                format!("this binary reads format {reads}"),
            );
        };
        let message = format!("{label} is written for an older orbe");
        let hint = if is_id(label) {
            let command = crate::project::add_writing(add, &[label.to_string()], true)
                .expect("the id is named");
            format!("run `{command}`")
        } else {
            format!(
                "update this file to format {max}, including its model. if it copies a catalog part, point the scene at that id instead of this path"
            )
        };
        return (message, hint);
    }
    if n > max {
        return (
            format!("format {n} is newer than this binary reads; this project needs a newer orbe"),
            format!("this binary reads format {reads}"),
        );
    }
    plain
}

fn format_bounds() -> Option<(u32, u32)> {
    Some((
        FORMATS.iter().copied().min()?,
        FORMATS.iter().copied().max()?,
    ))
}

fn whole_format(node: &Node) -> Option<u32> {
    let n = node.number()?;
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX) {
        Some(n as u32)
    } else {
        None
    }
}

fn name_ok(name: &str, span: Span, kind: NameKind, what: &str, out: &mut Findings) -> bool {
    match problem(name, kind) {
        None => true,
        Some(reason) => {
            out.push(
                Code::Name,
                span,
                format!("{what} `{name}` {reason}"),
                crate::name::hint(&reason),
            );
            false
        }
    }
}

pub(crate) fn decode_scene(
    sources: &Sources,
    file: FileId,
    root: &Node,
    out: &mut Findings,
) -> Option<Scene> {
    let Some(props) = root.props() else {
        schema(out, root, "an object");
        return None;
    };
    if !format_ok(file, root, out, None, "") {
        return None;
    }
    let mut scene = Scene {
        file,
        span: root.span,
        comp: Comp::default(),
        plugs: Vec::new(),
        nets: Vec::new(),
        seed: 0,
        max_step_ps: DEFAULT_MAX_STEP_PS,
        world: Vec::new(),
    };
    for prop in props {
        let value = &prop.value;
        match prop.key.as_str() {
            "format" | "$schema" | "layout" => {}
            "parts" => scene.comp.parts = decode_instances(value, out),
            "wires" => scene.comp.wires = decode_wires(sources, value, out),
            "ground" => scene.comp.ground = decode_ref(value, out),
            "nets" => scene.nets = decode_nets(value, out),
            "sim" => decode_sim(value, &mut scene, out),
            "world" => scene.world = decode_world(value, out),
            key if shape::PART.iter().any(|(k, _)| *k == key) => out.push(
                Code::Role,
                prop.key_span,
                format!("a scene has no `{key}`; it is a part's key, and this file is read as a scene"),
                "a part file is used from a scene, as \"btn1\": { \"type\": \"./parts/my-part.jsonc\" }".into(),),
            _ => unknown_key(out, prop, SCENE_KEYS, "a scene"),
        }
    }
    if let Some(layout) = root.get("layout") {
        scene.plugs = decode_layout(layout, &scene.comp, true, out);
    }
    Some(scene)
}

pub(crate) fn decode_part(
    sources: &Sources,
    file: FileId,
    label: String,
    root: &Node,
    out: &mut Findings,
    add: &str,
) -> Option<Part> {
    let Some(props) = root.props() else {
        schema(out, root, "an object");
        return None;
    };
    if !format_ok(file, root, out, Some(&label), add) {
        return None;
    }
    let mut part = Part {
        file,
        label,
        comp: Comp::default(),
        pins: Vec::new(),
        model: None,
        bodies: Vec::new(),
        params: Vec::new(),
        not_simulated: Vec::new(),
        joins_only: false,
        holes: None,
        reported: None,
    };
    for prop in props {
        let value = &prop.value;
        let Some((_, value_shape)) = shape::PART.iter().find(|(k, _)| *k == prop.key) else {
            match prop.key.as_str() {
                key @ ("sim" | "nets" | "world") => out.push(
                    Code::Role,
                    prop.key_span,
                    format!("a part has no `{key}`; only a scene does"),
                    format!("move `{key}` to the scene that uses this part"),
                ),
                _ => unknown_key(out, prop, &shape::keys(shape::PART), "a part"),
            }
            continue;
        };
        shape::check(value, value_shape, Some(&prop.key), out);
        match prop.key.as_str() {
            "parts" => part.comp.parts = decode_instances(value, out),
            "wires" => part.comp.wires = decode_wires(sources, value, out),
            "ground" => part.comp.ground = decode_ref(value, out),
            "pins" => part.pins = decode_pins(value, out),
            "model" => part.model = Some(value.clone()),
            "readings" => decode_names(value, "reading", out),
            "not_simulated" => part.not_simulated = decode_not_simulated(value, out),
            "buy" => crate::buy::check(value, out),
            "joins_only" => part.joins_only = matches!(value.json, Json::Bool(true)),
            "holes" => part.holes = Some(decode_holes(value)),
            _ => {}
        }
    }
    if let Some(params) = root.get("params") {
        part.params = decode_params(params, &part, out);
    }
    if let Some(model) = part.model.clone() {
        part.bodies = decode_bodies(&model, &part, out);
        check_sets(&mut part, &model, out);
    }
    if let Some(layout) = root.get("layout") {
        decode_layout(layout, &part.comp, false, out);
    }
    check_own_names(root, out);
    if let (Some(holes), Some(node)) = (&part.holes, root.get("holes")) {
        check_holes(holes, node, &part, out);
    }
    for (_, pins) in &part.not_simulated {
        for pin in pins {
            if part.pin(pin).is_none() {
                let span = root.get("not_simulated").map_or(root.span, |n| n.span);
                out.push(
                    Code::UnknownPin,
                    span,
                    format!("`not_simulated` lists `{pin}`, which is not a pin of this part"),
                    format!("its pins: {}", part.pin_list()),
                );
            }
        }
    }
    Some(part)
}

fn decode_instances(node: &Node, out: &mut Findings) -> Vec<Instance> {
    let Some(props) = node.props() else {
        schema(out, node, "an object of parts by name");
        return Vec::new();
    };
    let mut instances = Vec::new();
    for prop in props {
        name_ok(&prop.key, prop.key_span, NameKind::Name, "part name", out);
        let Some(fields) = prop.value.props() else {
            schema(out, &prop.value, "an object with a `type`");
            continue;
        };
        let mut instance = Instance {
            name: prop.key.clone(),
            name_span: prop.key_span,
            ty: None,
            params: Vec::new(),
            not_simulated: None,
        };
        for field in fields {
            match field.key.as_str() {
                "type" => match field.value.str() {
                    Some(text) => {
                        instance.ty = classify_type(text, field.value.span, out)
                            .map(|t| (t, field.value.span))
                    }
                    None => schema(out, &field.value, "a part id or a ./ path"),
                },
                "not_simulated" => match field.value.str() {
                    Some(reason) => instance.not_simulated = Some(reason.to_string()),
                    None => schema(
                        out,
                        &field.value,
                        "a string saying why the part is left out",
                    ),
                },
                _ => instance.params.push(field.clone()),
            }
        }
        if !fields.iter().any(|f| f.key == "type") {
            out.push(
                Code::Type,
                prop.key_span,
                format!("`{}` has no `type`", prop.key),
                "add a part id, as \"type\": \"led-5mm\", or a ./ path to a part file".into(),
            );
        }
        instances.push(instance);
    }
    instances
}

/// A part, Block or registry id: kebab case, starting with a letter, at most 64 characters.
pub fn is_id(text: &str) -> bool {
    text.len() <= 64
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// An id, or `@<registry>/<id>`.
fn is_type_id(text: &str) -> bool {
    match text.strip_prefix('@').and_then(|t| t.split_once('/')) {
        Some((registry, id)) => is_id(registry) && is_id(id),
        None => is_id(text),
    }
}

fn classify_type(text: &str, span: Span, out: &mut Findings) -> Option<TypeRef> {
    if text.starts_with("./") || text.starts_with("../") {
        Some(TypeRef::Path(text.to_string()))
    } else if is_type_id(text) {
        Some(TypeRef::Id(text.to_string()))
    } else {
        out.push(
            Code::Type,
            span,
            format!("`{text}` is neither a part id nor a ./ or ../ path"),
            "an id is kebab case, such as `led-5mm`; a path to a part of your own starts with ./"
                .into(),
        );
        None
    }
}

fn decode_ref(node: &Node, out: &mut Findings) -> Option<Ref> {
    let Some(text) = node.str() else {
        schema(out, node, "a pin ref such as \"nano.GND\"");
        return None;
    };
    let text = text.strip_prefix('@').unwrap_or(text);
    let segs: Vec<String> = text.split('.').map(str::to_string).collect();
    if segs.iter().any(String::is_empty) {
        out.push(
            Code::Wires,
            node.span,
            format!("`{text}` is not a pin ref"),
            "a pin ref is a part name, a dot and a pin, such as \"nano.D2\"".into(),
        );
        return None;
    }
    Some(Ref {
        text: text.to_string(),
        segs,
        span: node.span,
    })
}

fn decode_wires(sources: &Sources, node: &Node, out: &mut Findings) -> Vec<Wire> {
    let Some(items) = node.items() else {
        schema(out, node, "a list of wires");
        return Vec::new();
    };
    let mut wires: Vec<Wire> = Vec::new();
    for item in items {
        let shape = item
            .items()
            .filter(|e| (2..=3).contains(&e.len()) && e[0].str().is_some() && e[1].str().is_some());
        let Some(elements) = shape else {
            out.push(Code::Wires, item.span, "a wire is two pin refs and an optional hint", "such as [\"nano.D2\", \"btn1.a\"] or [\"r1.b\", \"led1.anode\", { \"kind\": \"jumper\" }]".into());
            continue;
        };
        if let Some(hint) = elements.get(2) {
            decode_wire_hint(hint, out);
        }
        let (Some(a), Some(b)) = (decode_ref(&elements[0], out), decode_ref(&elements[1], out))
        else {
            continue;
        };
        if a.text == b.text {
            out.push(
                Code::Wires,
                item.span,
                format!("this wire joins `{}` to itself", a.text),
                "name two different pins".into(),
            );
            continue;
        }
        if let Some(first) = wires.iter().find(|w| {
            let [x, y] = &w.ends;
            (x.text == a.text && y.text == b.text) || (x.text == b.text && y.text == a.text)
        }) {
            out.push(
                Code::Duplicate,
                item.span,
                format!("the wire from `{}` to `{}` is listed twice", a.text, b.text),
                format!(
                    "the first is at {}; remove one",
                    sources.location(first.ends[0].span)
                ),
            );
            continue;
        }
        wires.push(Wire { ends: [a, b] });
    }
    wires
}

fn decode_wire_hint(hint: &Node, out: &mut Findings) {
    let Some(props) = hint.props() else {
        out.push(
            Code::Wires,
            hint.span,
            format!("a wire's hint is an object, found {}", hint.what()),
            "such as { \"kind\": \"trace\" }".into(),
        );
        return;
    };
    for prop in props {
        match prop.key.as_str() {
            "kind" => match prop.value.str() {
                Some(kind) if WIRE_KINDS.contains(&kind) => {}
                Some(kind) => out.push(
                    Code::Wires,
                    prop.value.span,
                    format!("`{kind}` is not a wire kind"),
                    format!(
                        "{}the kinds are jumper, trace and contact",
                        did_you_mean(kind, WIRE_KINDS.iter().copied(), "")
                    ),
                ),
                None => out.push(
                    Code::Wires,
                    prop.value.span,
                    "a wire's kind is a string",
                    "the kinds are jumper, trace and contact".into(),
                ),
            },
            "color" => {
                if prop.value.str().is_none() {
                    out.push(
                        Code::Wires,
                        prop.value.span,
                        "a wire's color is a string",
                        "name a color, such as \"red\"".into(),
                    );
                }
            }
            key => out.push(
                Code::Wires,
                prop.key_span,
                format!("a wire's hint has no key `{key}`"),
                format!(
                    "{}its keys are kind and color",
                    did_you_mean(key, ["kind", "color"], "")
                ),
            ),
        }
    }
}

/// Checks every placement, and returns the scene's plugs. Only a scene plugs a part in.
fn decode_layout(node: &Node, comp: &Comp, scene: bool, out: &mut Findings) -> Vec<Plug> {
    let mut plugs = Vec::new();
    let Some(props) = node.props() else {
        out.push(
            Code::Layout,
            node.span,
            "`layout` is an object of placements by part name",
            "each key is a part name and each value is { \"at\": [x, y] }".into(),
        );
        return plugs;
    };
    for prop in props {
        if !comp.parts.iter().any(|i| i.name == prop.key) {
            let hint = did_you_mean(&prop.key, comp.parts.iter().map(|i| i.name.as_str()), "");
            out.push(
                Code::Layout,
                prop.key_span,
                format!("there is no part named `{}` to place", prop.key),
                format!("{hint}a layout entry is keyed by a name in `parts`"),
            );
        }
        let Some(fields) = prop.value.props() else {
            out.push(
                Code::Layout,
                prop.value.span,
                "a placement is an object, such as { \"at\": [38, 12] }",
                "write { \"at\": [x, y] }".into(),
            );
            continue;
        };
        for field in fields {
            let numbers = |lens: &[usize]| {
                field.value.items().is_some_and(|i| {
                    lens.contains(&i.len()) && i.iter().all(|n| n.number().is_some())
                })
            };
            match field.key.as_str() {
                "at" if numbers(&[2, 3]) => {}
                "at" => out.push(
                    Code::Layout,
                    field.value.span,
                    "`at` is [x, y] or [x, y, z] in mm",
                    "write [x, y] or [x, y, z]".into(),
                ),
                "rotate" if numbers(&[3]) => {}
                "rotate" => out.push(
                    Code::Layout,
                    field.value.span,
                    "`rotate` is [rx, ry, rz] in degrees",
                    "write [rx, ry, rz]".into(),
                ),
                "on" if scene => {
                    if let Some(plug) = decode_on(&prop.key, &prop.value, field, out) {
                        plugs.push(plug);
                    }
                }
                "on" => out.push(
                    Code::Layout,
                    field.key_span,
                    "only a scene's layout plugs a part in with `on`",
                    "place a part's child with `at` and `rotate`".into(),
                ),
                "mate" => out.push(
                    Code::Layout,
                    field.key_span,
                    "`mate` is reserved for a later format",
                    "place the part with `at` and `rotate`".into(),
                ),
                key => out.push(
                    Code::Layout,
                    field.key_span,
                    format!("a placement has no key `{key}`"),
                    format!(
                        "{}its keys are at, rotate and on",
                        did_you_mean(key, ["at", "rotate", "on"], "")
                    ),
                ),
            }
        }
    }
    plugs
}

/// `"on": { "<pin>": "<board>.<hole>" }`, with the placement's `rotate` about Z only.
fn decode_on(name: &str, placement: &Node, on: &Prop, out: &mut Findings) -> Option<Plug> {
    let example =
        "write \"on\": { \"<pin>\": \"<board>.<hole>\" }, such as { \"anode\": \"bb.f20\" }";
    if let Some(at) = placement
        .props()
        .and_then(|p| p.iter().find(|f| f.key == "at"))
    {
        out.push(
            Code::Layout,
            at.key_span,
            format!("`{name}` has both `at` and `on`; `on` places it on the board"),
            "remove `at`".into(),
        );
        return None;
    }
    let entries = on.value.props().filter(|p| p.len() == 1);
    let Some([entry]) = entries else {
        out.push(
            Code::Layout,
            on.value.span,
            "`on` names one pin and the hole its leg goes into",
            example.into(),
        );
        return None;
    };
    let hole = decode_ref(&entry.value, out)?;
    if hole.segs.len() != 2 {
        out.push(
            Code::Layout,
            entry.value.span,
            format!(
                "`{}` is not a hole; a hole is a board's name and the hole, such as \"bb.f20\"",
                hole.text
            ),
            example.into(),
        );
        return None;
    }
    let mut turn = 0.0;
    if let Some(rotate) = placement.get("rotate") {
        let angles: Option<Vec<f64>> = rotate
            .items()
            .and_then(|items| items.iter().map(Node::number).collect());
        if let Some([rx, ry, rz]) = angles.as_deref() {
            if *rx != 0.0 || *ry != 0.0 {
                out.push(
                    Code::Layout,
                    rotate.span,
                    format!("`{name}` sits on a board, so it turns about Z only"),
                    format!("write \"rotate\": [0, 0, {rz}]"),
                );
                return None;
            }
            turn = *rz;
        }
    }
    Some(Plug {
        name: name.to_string(),
        span: on.value.span,
        pin: entry.key.clone(),
        pin_span: entry.key_span,
        hole,
        turn,
    })
}

fn decode_holes(node: &Node) -> Holes {
    let strips = node
        .get("strips")
        .and_then(Node::items)
        .unwrap_or_default()
        .iter()
        .map(|strip| {
            strip
                .items()
                .unwrap_or_default()
                .iter()
                .filter_map(|hole| hole.str().map(str::to_string))
                .collect()
        })
        .collect();
    Holes {
        depth: node.get("depth").and_then(Node::number).unwrap_or(0.0),
        strips,
    }
}

/// Each strip lists two or more holes, each a pin of the part on one lead, and no hole is in
/// two strips. A board needs a depth above zero.
fn check_holes(holes: &Holes, node: &Node, part: &Part, out: &mut Findings) {
    let depth = node.get("depth");
    if depth.and_then(Node::number).is_none_or(|d| d <= 0.0) {
        out.push(
            Code::Syntax,
            depth.map_or(node.span, |d| d.span),
            "`holes` needs a `depth` above 0, how far a leg goes in, in mm",
            "write \"depth\": 8".into(),
        );
    }
    let items = node.get("strips").and_then(Node::items).unwrap_or_default();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, (strip, written)) in holes.strips.iter().zip(items).enumerate() {
        if strip.len() < 2 {
            out.push(
                Code::Syntax,
                written.span,
                "a strip joins two or more holes",
                "list every hole one clip joins, such as [\"a1\", \"b1\", \"c1\", \"d1\", \"e1\"]"
                    .into(),
            );
        }
        for hole in strip {
            let one_lead = part
                .pin(hole)
                .is_some_and(|pin| matches!(pin.def, PinDef::Leads));
            if !one_lead {
                out.push(
                    Code::UnknownPin,
                    written.span,
                    format!(
                        "strip {} lists `{hole}`, which is not a pin of this part",
                        index + 1
                    ),
                    "each hole is a pin with one lead, as \"a1\": [1]".into(),
                );
            }
            if let Some(first) = seen.insert(hole, index) {
                out.push(
                    Code::Duplicate,
                    written.span,
                    format!("`{hole}` is in strips {} and {}", first + 1, index + 1),
                    "a hole sits in one strip".into(),
                );
            }
        }
    }
}

fn decode_nets(node: &Node, out: &mut Findings) -> Vec<(String, Span, Ref)> {
    let Some(props) = node.props() else {
        schema(out, node, "an object of net names to pin refs");
        return Vec::new();
    };
    let mut nets = Vec::new();
    for prop in props {
        if name_ok(&prop.key, prop.key_span, NameKind::Name, "net name", out)
            && let Some(r) = decode_ref(&prop.value, out)
        {
            nets.push((prop.key.clone(), prop.key_span, r));
        }
    }
    nets
}

fn decode_sim(node: &Node, scene: &mut Scene, out: &mut Findings) {
    let Some(props) = node.props() else {
        schema(out, node, "an object");
        return;
    };
    for prop in props {
        let value = &prop.value;
        match prop.key.as_str() {
            "seed" => match value.to_value().as_u64() {
                Some(seed) => scene.seed = seed,
                None => schema(out, value, "an unsigned integer"),
            },
            "max_step" => {
                let ps = match (value.str(), value.number()) {
                    (Some(text), _) => units::parse(text, Unit::Second, 12),
                    (None, Some(seconds)) => units::parse(&seconds.to_string(), Unit::Second, 12),
                    _ => Err(format!("found {}", value.what())),
                };
                match ps {
                    Ok(ps) if (1000.0..=STOP_PS as f64).contains(&ps) => {
                        scene.max_step_ps = ps.round() as u64
                    }
                    Ok(_) => out.push(
                        Code::Syntax,
                        value.span,
                        "`max_step` is from 1 ns to 2 hours",
                        "write a duration such as \"10ms\"".into(),
                    ),
                    Err(reason) => out.push(
                        Code::Syntax,
                        value.span,
                        format!("`max_step` is a duration, such as \"10ms\": {reason}"),
                        "write a duration such as \"10ms\"".into(),
                    ),
                }
            }
            _ => unknown_key(out, prop, &["seed", "max_step"], "`sim`"),
        }
    }
}

const WORLD_KEYS: &[&str] = &["initial", "steps"];
const STEP_KEYS: &[&str] = &["at", "to"];

/// `"world": { "light": 300, "temperature": { "initial": 25, "steps": [{ "at": "2s", "to": 30 }] } }`.
/// A value is a number in its quantity's unit. Steps come at least 2 ns apart, after t = 0 and
/// before the 2-hour stop, and each changes the value.
fn decode_world(node: &Node, out: &mut Findings) -> Vec<Ambient> {
    let Some(props) = node.props() else {
        schema(out, node, "an object of world quantities by name");
        return Vec::new();
    };
    let names: Vec<&str> = Quantity::ALL.iter().map(|q| q.name()).collect();
    let mut world = Vec::new();
    for prop in props {
        let Some(quantity) = Quantity::parse(&prop.key) else {
            let hint = did_you_mean(&prop.key, names.iter().copied(), "");
            out.push(
                Code::World,
                prop.key_span,
                format!("`{}` is not a world quantity", prop.key),
                format!("{hint}the quantities are {}", and_list(&names)),
            );
            continue;
        };
        if let Some(ambient) = decode_ambient(quantity, &prop.value, out) {
            world.push(ambient);
        }
    }
    world
}

fn decode_ambient(quantity: Quantity, node: &Node, out: &mut Findings) -> Option<Ambient> {
    if node.number().is_some() {
        let initial = world_value(quantity, node, out)?;
        return Some(Ambient {
            quantity,
            initial,
            steps: Vec::new(),
        });
    }
    let Some(props) = node.props() else {
        schema(
            out,
            node,
            &format!(
                "a number in {}, or {{ \"initial\", \"steps\" }}",
                quantity.unit()
            ),
        );
        return None;
    };
    let mut initial = None;
    let mut steps = Some(Vec::new());
    for prop in props {
        match prop.key.as_str() {
            "initial" => initial = world_value(quantity, &prop.value, out),
            "steps" => steps = decode_steps(quantity, &prop.value, out),
            _ => unknown_key(out, prop, WORLD_KEYS, &format!("`{}`", quantity.name())),
        }
    }
    if node.get("initial").is_none() {
        out.push(
            Code::Syntax,
            node.span,
            format!("`{}` has no `initial`", quantity.name()),
            format!("add \"initial\": {}", quantity.row().default),
        );
    }
    let initial = initial?;
    let steps = steps?;
    let mut ok = true;
    let mut last: Option<(u64, f64)> = None;
    for (index, &(at, to)) in steps.iter().enumerate() {
        let (before_ps, before) = last.unwrap_or((0, initial));
        let problem = if last.is_some() && at < before_ps + MIN_GAP_PS {
            Some(
                "is not at least 2 ns after the step before it; steps go in order of `at`"
                    .to_string(),
            )
        } else if to == before {
            Some(format!("does not change {}", quantity.name()))
        } else {
            None
        };
        if let Some(problem) = problem {
            ok = false;
            let span = node
                .get("steps")
                .and_then(Node::items)
                .and_then(|items| items.get(index))
                .map_or(node.span, |item| item.span);
            out.push(
                Code::World,
                span,
                format!("step {index} of `{}` {problem}", quantity.name()),
                "give each step a later `at` and a new `to`".into(),
            );
        }
        last = Some((at, to));
    }
    ok.then_some(Ambient {
        quantity,
        initial,
        steps,
    })
}

fn decode_steps(quantity: Quantity, node: &Node, out: &mut Findings) -> Option<Vec<(u64, f64)>> {
    let Some(items) = node.items() else {
        schema(out, node, "a list of { \"at\", \"to\" }");
        return None;
    };
    let mut steps = Vec::new();
    let mut ok = true;
    for item in items {
        let Some(props) = item.props() else {
            schema(out, item, "{ \"at\": \"1s\", \"to\": <value> }");
            ok = false;
            continue;
        };
        let mut at = None;
        let mut to = None;
        for prop in props {
            match prop.key.as_str() {
                "at" => at = step_time(&prop.value, out),
                "to" => to = world_value(quantity, &prop.value, out),
                _ => unknown_key(out, prop, STEP_KEYS, "a step"),
            }
        }
        for key in STEP_KEYS {
            if item.get(key).is_none() {
                out.push(
                    Code::Syntax,
                    item.span,
                    format!("a step has no `{key}`"),
                    "write a step as { \"at\": \"1s\", \"to\": <value> }".into(),
                );
            }
        }
        match (at, to) {
            (Some(at), Some(to)) => steps.push((at, to)),
            _ => ok = false,
        }
    }
    ok.then_some(steps)
}

/// A step's time: a duration after t = 0 and before the 2-hour stop, in whole picoseconds.
fn step_time(node: &Node, out: &mut Findings) -> Option<u64> {
    let ps = match node.str() {
        Some(text) => units::parse(text, Unit::Second, 12),
        None => Err(format!("found {}", node.what())),
    };
    match ps {
        Ok(ps) if ps >= 1.0 && ps < STOP_PS as f64 => Some(ps.round() as u64),
        Ok(_) => {
            out.push(
                Code::World,
                node.span,
                "a step's `at` is after 0 and before the 2-hour stop",
                "write a duration such as \"1s\"".into(),
            );
            None
        }
        Err(reason) => {
            out.push(
                Code::Syntax,
                node.span,
                format!("a step's `at` is a duration, such as \"1s\": {reason}"),
                "write a duration such as \"1s\"".into(),
            );
            None
        }
    }
}

/// A number in `quantity`'s unit, inside its range.
fn world_value(quantity: Quantity, node: &Node, out: &mut Findings) -> Option<f64> {
    let row = quantity.row();
    let Some(value) = node.number() else {
        schema(out, node, &format!("a number in {}", row.unit));
        return None;
    };
    if !quantity.holds(value) {
        out.push(
            Code::World,
            node.span,
            format!(
                "{} is from {} to {} {}, not {value}",
                row.name, row.min, row.max, row.unit
            ),
            format!("write a value from {} to {}", row.min, row.max),
        );
        return None;
    }
    Some(value)
}

fn decode_pins(node: &Node, out: &mut Findings) -> Vec<Pin> {
    let Some(props) = node.props() else {
        schema(out, node, "an object of pins by name");
        return Vec::new();
    };
    let mut pins = Vec::new();
    for prop in props {
        name_ok(&prop.key, prop.key_span, NameKind::Pin, "pin", out);
        let value = &prop.value;
        let def = if value
            .items()
            .is_some_and(|i| !i.is_empty() && i.iter().all(|n| n.number().is_some()))
        {
            PinDef::Leads
        } else if value.str().is_some() {
            PinDef::Label(decode_ref(value, out).into_iter().collect())
        } else if let Some(items) = value
            .items()
            .filter(|i| !i.is_empty() && i.iter().all(|n| n.str().is_some()))
        {
            PinDef::Label(items.iter().filter_map(|n| decode_ref(n, out)).collect())
        } else {
            schema(
                out,
                value,
                "lead numbers such as [1, 2], or child pin refs such as \"j2.1\"",
            );
            continue;
        };
        pins.push(Pin {
            name: prop.key.clone(),
            def,
        });
    }
    pins
}

/// Each name a part file gives to another of its own sections: `controls.press.joint` to a key
/// of `joints`, each `view.glow` entry's `by` to a reading, and each `view.tint` and
/// `view.recolour` entry's `by` to a param. A section that is not an object has its own finding.
fn check_own_names(root: &Node, out: &mut Findings) {
    let press = root.get("controls").and_then(|c| c.get("press"));
    let mut names: Vec<(&Node, &str, &str)> = press
        .and_then(|p| p.get("joint"))
        .map(|joint| (joint, "joints", "joint"))
        .into_iter()
        .collect();
    let view = root.get("view");
    for (list, section, what) in [
        ("glow", "readings", "reading"),
        ("tint", "params", "param"),
        ("recolour", "params", "param"),
    ] {
        let entries = view.and_then(|v| v.get(list)).and_then(Node::items);
        let by = entries
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e.get("by"));
        names.extend(by.map(|by| (by, section, what)));
    }
    for (node, section, what) in names {
        let Some(name) = node.str() else { continue };
        let props = match root.get(section).map(Node::props) {
            Some(None) => continue,
            found => found.flatten().unwrap_or_default(),
        };
        let keys: Vec<&str> = props.iter().map(|p| p.key.as_str()).collect();
        if keys.contains(&name) {
            continue;
        }
        let hint = match keys.as_slice() {
            [] => format!("add `{name}` to `{section}`"),
            [only] => format!(
                "{}its only {what} is {only}",
                did_you_mean(name, keys.iter().copied(), "")
            ),
            _ => format!(
                "{}its {section} are {}",
                did_you_mean(name, keys.iter().copied(), ""),
                and_list(&keys)
            ),
        };
        out.push(
            Code::UnknownRef,
            node.span,
            format!("this part has no {what} named `{name}`"),
            hint,
        );
    }
}

fn decode_names(node: &Node, what: &str, out: &mut Findings) {
    for prop in node.props().unwrap_or_default() {
        name_ok(&prop.key, prop.key_span, NameKind::Name, what, out);
    }
}

/// Each entry's reason with the pins it lists. `shape` reports the types.
fn decode_not_simulated(node: &Node, out: &mut Findings) -> Vec<(String, Vec<String>)> {
    let mut entries = Vec::new();
    for item in node.items().unwrap_or_default() {
        if item.props().is_some() && item.get("why").is_none() {
            out.push(
                Code::Syntax,
                item.span,
                "a not_simulated entry has no `why`",
                "say why it is left out, as \"why\": \"The engine has no temperature.\"".into(),
            );
        }
        let Some(why) = item.get("why").and_then(Node::str) else {
            continue;
        };
        let pins = item.get("pins").and_then(Node::items).unwrap_or_default();
        entries.push((
            why.to_string(),
            pins.iter()
                .filter_map(Node::str)
                .map(str::to_string)
                .collect(),
        ));
    }
    entries
}

/// The part's params. `shape` reports an unknown key or a value of the wrong type, so a field
/// written with the wrong type is left out here without a second finding.
fn decode_params(node: &Node, part: &Part, out: &mut Findings) -> Vec<ParamDecl> {
    let mut params = Vec::new();
    for prop in node.props().unwrap_or_default() {
        if prop.key == "type" || prop.key == "not_simulated" {
            out.push(
                Code::Param,
                prop.key_span,
                format!(
                    "no param may be named `{}`; an instance uses that key itself",
                    prop.key
                ),
                "rename it; an instance already uses that key".into(),
            );
            continue;
        }
        name_ok(&prop.key, prop.key_span, NameKind::Name, "param", out);
        if prop.value.props().is_none() {
            continue;
        }
        let field = |key: &str| prop.value.get(key);
        let shape = shape::param_shape(&prop.value, out);
        let kind = match shape {
            ParamShape::Choice => {
                ParamKind::Choice(field("choices").map(decode_choices).unwrap_or_default())
            }
            ParamShape::File => match field("as").map(Node::str) {
                Some(Some("text")) => ParamKind::File,
                Some(_) => continue,
                None => {
                    out.push(
                        Code::Param,
                        prop.key_span,
                        format!("file param `{}` needs \"as\": \"text\"", prop.key),
                        "add \"as\": \"text\"".into(),
                    );
                    continue;
                }
            },
            ParamShape::Number => {
                let Some(unit) = field("unit") else { continue };
                let Some(text) = unit.str() else { continue };
                let Some(parsed) = Unit::parse(text) else {
                    out.push(
                        Code::Param,
                        unit.span,
                        "a param's unit is one of ohm, F, H, V, A, s, Hz, W and 1",
                        "use one of ohm, F, H, V, A, s, Hz, W or 1".into(),
                    );
                    continue;
                };
                ParamKind::Number {
                    unit: parsed,
                    range: Range {
                        above: field("min_exclusive").and_then(Node::number),
                        below: field("max_exclusive").and_then(Node::number),
                    },
                }
            }
            ParamShape::Through => ParamKind::Through,
        };
        let sets = match kind {
            ParamKind::Number { .. } | ParamKind::File => {
                field("sets").and_then(Node::str).map(str::to_string)
            }
            ParamKind::Choice(_) | ParamKind::Through => None,
        };
        let required = field("required").is_some_and(|r| r.to_value().as_bool() != Some(false));
        let default = field("default").cloned();
        let through = matches!(kind, ParamKind::Through);
        if !through && default.is_none() && !required {
            out.push(
                Code::Param,
                prop.key_span,
                format!(
                    "param `{}` needs a `default` or \"required\": true",
                    prop.key
                ),
                "add \"default\" or \"required\": true".into(),
            );
        }
        let nowhere = reaches_nothing(&prop.key, through, part);
        if !through
            && !matches!(kind, ParamKind::Choice(_))
            && field("sets").is_none()
            && nowhere.is_none()
        {
            out.push(
                Code::Param,
                prop.key_span,
                format!("param `{}` has no `sets` pointer into `model`", prop.key),
                "add \"sets\": \"/field\" pointing at the model field this param writes".into(),
            );
        }
        if let Some((message, hint)) = nowhere {
            out.push(Code::Param, prop.key_span, message, hint);
        }
        params.push(ParamDecl {
            name: prop.key.clone(),
            span: prop.key_span,
            kind,
            default,
            sets,
            sets_refused: false,
        });
    }
    params
}

/// Why a param's value would reach nothing: it writes no `model` this part has, and no
/// simulated child takes it.
fn reaches_nothing(name: &str, through: bool, part: &Part) -> Option<(String, String)> {
    if !through && part.model.is_some() {
        return None;
    }
    let takers: Vec<&Instance> = part
        .comp
        .parts
        .iter()
        .filter(|child| child.params.iter().any(|p| passed(&p.value) == Some(name)))
        .collect();
    if takers.iter().any(|child| child.not_simulated.is_none()) {
        return None;
    }
    let no_child = format!(
        "no child takes `{name}`; pass it to a child with {{ \"param\": \"{name}\" }}, or remove it"
    );
    Some(match takers.as_slice() {
        [] if through && part.comp.parts.is_empty() => (
            format!("param `{name}` has no unit, choices or file, and this part has no children to pass it to"),
            "give this part a child that takes the param, or give the param a unit, choices or file".into(),
        ),
        [] if through => (
            format!("param `{name}` has no unit, choices or file, and no child of this part takes it, so its value reaches nothing"),
            no_child,
        ),
        [] => (
            format!("param `{name}` writes `model`, but this part has no `model` and no child of this part takes it, so its value reaches nothing"),
            no_child,
        ),
        takers => (
            format!(
                "param `{name}` is passed only to {}, which {} not simulated, so its value reaches nothing",
                takers.iter().map(|c| format!("`{}`", c.name)).collect::<Vec<_>>().join(", "),
                if takers.len() == 1 { "is" } else { "are" }
            ),
            format!("pass `{name}` to a simulated child, or remove it"),
        ),
    })
}

/// Every object a pointer passes through is in `model` and in each value a choice writes
/// around it, or a param turns it on: one with "default": null inside it, or one that writes
/// it. A list is passed by the index of an item it has. Otherwise some value of the param
/// would not take effect.
fn check_sets(part: &mut Part, model: &Node, out: &mut Findings) {
    let model = model.to_value();
    let mut refused = Vec::new();
    for (index, decl) in part.params.iter().enumerate() {
        let problem = decl
            .pointers()
            .into_iter()
            .find_map(|pointer| sets_problem(part, &model, pointer, &decl.name));
        if let Some((problem, hint)) = problem {
            out.push(
                Code::Param,
                decl.span,
                format!("param `{}`: {problem}", decl.name),
                hint,
            );
            refused.push(index);
        }
    }
    for (later, decl) in part.params.iter().enumerate() {
        if refused.contains(&later) {
            continue;
        }
        let clash = part.params[..later]
            .iter()
            .enumerate()
            .filter(|(earlier, _)| !refused.contains(earlier))
            .find_map(|(_, earlier)| clash(earlier, decl));
        if let Some((message, hint)) = clash {
            out.push(Code::Param, decl.span, message, hint);
            refused.push(later);
        }
    }
    for index in refused {
        part.params[index].sets_refused = true;
    }
}

/// Two params that write one field: the same pointer, or one inside the other's value. A
/// param that writes an object, or turns one on as a null default, leaves the fields inside
/// it to other params.
fn clash(earlier: &ParamDecl, later: &ParamDecl) -> Option<(String, String)> {
    let (a, b) = (&earlier.name, &later.name);
    for p in later.pointers() {
        for q in earlier.pointers() {
            if p == q {
                return Some((
                    format!("params `{a}` and `{b}` both write `{p}`"),
                    format!("let one param write `{p}`; remove it from `{b}` or from `{a}`"),
                ));
            }
            let (outer, at, inside, place) = if within(p, q) {
                (earlier, q, p, "inside")
            } else if within(q, p) {
                (later, p, q, "around")
            } else {
                continue;
            };
            if outer.holds_object(at) {
                continue;
            }
            return Some((
                format!("param `{b}` writes `{p}`, {place} `{q}`, which param `{a}` writes"),
                format!(
                    "let one param write `{inside}`, or make `{}` a choice that writes an object or a list at `{at}`, or give it \"default\": null",
                    outer.name
                ),
            ));
        }
    }
    None
}

/// The choice param and arm that wrote a value, or `None` for `model`.
type By<'p> = Option<(&'p ParamDecl, &'p str)>;

/// Where a value comes from, for a message: "in `model`" or "in choice `play`'s `one`".
fn place(by: By<'_>) -> String {
    match by {
        None => "in `model`".into(),
        Some((decl, arm)) => format!("in choice `{}`'s `{arm}`", decl.name),
    }
}

/// A value the object or list at one point on a pointer's way may hold once the params are
/// applied. `None` once the way is off: the objects from there on are made by the param that
/// turns them on.
#[derive(Clone)]
struct Held<'p> {
    value: Option<Value>,
    by: By<'p>,
    /// Who wrote the field the way takes next. A choice that merges an object into the one it
    /// lands on writes only its own fields.
    next_by: By<'p>,
}

/// A choice param that writes a point on a pointer's way, with each arm's writes there by the
/// point's index.
struct Chooser<'p> {
    decl: &'p ParamDecl,
    arms: Vec<(&'p str, Vec<(usize, Value)>)>,
    /// The shallowest point an arm writes, where the walk picks the arm.
    first: usize,
}

impl Chooser<'_> {
    fn writes(&self, arm: usize, at: usize) -> Option<&Value> {
        self.arms[arm]
            .1
            .iter()
            .find(|(point, _)| *point == at)
            .map(|(_, value)| value)
    }
}

/// What the arms picked so far leave for the rest of the way.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Picked<'p> {
    /// The arm each chooser picked, while that arm writes a point further on.
    bound: Vec<(usize, usize)>,
    /// Points further on that a shallower choice's value covers, with that param's name. The
    /// reader drops a default written there, so the covering value shows too.
    covers: Vec<(usize, &'p str)>,
}

impl<'p> Picked<'p> {
    /// Each way the choosers that write point `at` can go from here, with the arm whose write
    /// lands there. Of two params that write it, the first one's lands.
    fn picks(
        &self,
        at: usize,
        way: &[String],
        choosers: &[Chooser<'p>],
    ) -> Vec<(Picked<'p>, Option<(usize, usize)>)> {
        let mut picks = vec![(self.clone(), None)];
        for (c, chooser) in choosers.iter().enumerate() {
            let arms: Vec<usize> = match self.bound.iter().find(|(b, _)| *b == c) {
                Some(&(_, arm)) => vec![arm],
                None if chooser.first == at => (0..chooser.arms.len()).collect(),
                None => continue,
            };
            let mut grown: Vec<(Picked<'p>, Option<(usize, usize)>)> = Vec::new();
            for (from, write) in &picks {
                for &arm in &arms {
                    let mut picked = from.clone();
                    let mut write = *write;
                    picked.bound.retain(|(b, _)| *b != c);
                    if chooser.arms[arm].1.iter().any(|(point, _)| *point > at) {
                        picked.bound.push((c, arm));
                        picked.bound.sort_unstable();
                    }
                    if let Some(value) = chooser.writes(arm, at) {
                        write = write.or(Some((c, arm)));
                        for (k, ahead) in way.iter().enumerate().skip(at + 1) {
                            if matches!(cover(value, &ahead[way[at].len()..]), Cover::Value) {
                                picked.covers.push((k, chooser.decl.name.as_str()));
                            }
                        }
                        picked.covers.sort_unstable();
                        picked.covers.dedup();
                    }
                    let lands = |w: Option<(usize, usize)>| {
                        w.and_then(|(c, arm)| choosers[c].writes(arm, at))
                    };
                    if !grown
                        .iter()
                        .any(|(g, w)| *g == picked && lands(*w) == lands(write))
                    {
                        grown.push((picked, write));
                    }
                }
            }
            picks = grown;
        }
        picks
    }
}

/// Each choice param that writes a point on the pointer's way, and for `name` itself, only
/// the arms that write `pointer`.
fn choosers<'p>(part: &'p Part, pointer: &str, way: &[String], name: &str) -> Vec<Chooser<'p>> {
    part.params
        .iter()
        .filter_map(|decl| {
            let ParamKind::Choice(choices) = &decl.kind else {
                return None;
            };
            let arms: Vec<(&str, Vec<(usize, Value)>)> = choices
                .iter()
                .filter(|(_, sets)| decl.name != name || sets.iter().any(|(p, _)| p == pointer))
                .map(|(arm, sets)| {
                    let writes = sets
                        .iter()
                        .filter_map(|(p, v)| Some((way.iter().position(|w| w == p)?, v.to_value())))
                        .collect();
                    (arm.as_str(), writes)
                })
                .collect();
            let first = arms
                .iter()
                .flat_map(|(_, writes)| writes.iter().map(|(at, _)| *at))
                .min()?;
            Some(Chooser { decl, arms, first })
        })
        .collect()
}

/// Every value each point on the way may hold is checked, for every arm of every choice that
/// writes points on it, applied as the reader applies them: shallowest first, an object
/// merged into the one it lands on, anything else in place of what is there. The pointer is
/// good only when each of them has the way.
fn sets_problem(part: &Part, model: &Value, pointer: &str, name: &str) -> Option<(String, String)> {
    let tokens = match tokens(pointer) {
        Ok(tokens) => tokens,
        Err(problem) => {
            return Some((
                problem,
                "point `sets` at a field of an object in `model`, such as \"/resistance\"".into(),
            ));
        }
    };
    let way: Vec<String> = (1..tokens.len())
        .map(|end| pointer_of(&tokens[..end]))
        .collect();
    let choosers = choosers(part, pointer, &way, name);
    let pointer = pointer_of(&tokens);
    let (last, through) = tokens.split_last().expect("split yields one token");
    let mut states = States::default();
    states.add(
        &Picked::default(),
        Some(toward(model, tokens[0])),
        None,
        None,
    );
    for (i, (token, here)) in through.iter().zip(&way).enumerate() {
        let next = tokens[i + 1];
        let mut reached = States::default();
        for (picked, parents) in &states.groups {
            let mut merges = Vec::new();
            let mut kept = Vec::new();
            for (mut picked, write) in picked.picks(i, &way, &choosers) {
                let covered = picked.covers.iter().any(|&(k, by)| {
                    k == i && write.is_some_and(|(c, _)| choosers[c].decl.name != by)
                });
                picked.covers.retain(|&(k, _)| k > i);
                if let Some((c, arm)) = write {
                    let chooser = &choosers[c];
                    let by = Some((chooser.decl, chooser.arms[arm].0));
                    match chooser.writes(arm, i).expect("the arm writes this point") {
                        Value::Null => {}
                        Value::Object(written) if written.contains_key(next) => {
                            reached.add(&picked, Some(field(written, next)), by, by);
                        }
                        Value::Object(written) => {
                            if !merges.iter().any(|(p, _, _)| *p == picked) {
                                merges.push((picked.clone(), by, written));
                            }
                        }
                        value => reached.add(&picked, Some(value.clone()), by, by),
                    }
                    let droppable = covered && chooser.decl.default.is_some();
                    if !droppable {
                        continue;
                    }
                }
                if !kept.contains(&picked) {
                    kept.push(picked);
                }
            }
            for h in parents {
                let found = h.value.as_ref().map_or(Step::Missing, |v| step(v, token));
                for (picked, by, written) in &merges {
                    match found {
                        Step::Found(Value::Object(base)) => {
                            reached.add(picked, Some(field(base, next)), *by, h.next_by);
                        }
                        _ => reached.add(picked, Some(field(written, next)), *by, *by),
                    }
                }
                if kept.is_empty() {
                    continue;
                }
                let value = match found {
                    Step::Found(value) if value.is_object() || value.is_array() => {
                        Some(toward(value, next))
                    }
                    Step::Found(Value::Null) | Step::Missing if part.opener(here).is_some() => None,
                    found => {
                        return Some(off_the_way(&pointer, &tokens[..i], token, h, found, name));
                    }
                };
                for picked in &kept {
                    reached.add(picked, value.clone(), h.next_by, h.next_by);
                }
            }
        }
        states = reached;
    }
    states
        .groups
        .iter()
        .flat_map(|(_, held)| held)
        .find_map(|h| {
            let value = h.value.as_ref()?;
            match step(value, last) {
                _ if value.is_object() => None,
                Step::Found(_) => None,
                found => Some(off_the_way(&pointer, through, last, h, found, name)),
            }
        })
}

/// The values a point on the way may hold, grouped by what the arms picked so far leave for
/// the rest of the way. The walk branches on the arms once per group, and keeps one of the
/// values that differ only in who wrote them or in fields the way does not read, so its cost
/// follows the arms, not their product.
#[derive(Default)]
struct States<'p> {
    groups: Vec<(Picked<'p>, Vec<Held<'p>>)>,
    index: BTreeMap<Picked<'p>, usize>,
}

impl<'p> States<'p> {
    fn add(&mut self, picked: &Picked<'p>, value: Option<Value>, by: By<'p>, next_by: By<'p>) {
        let at = *self.index.entry(picked.clone()).or_insert_with(|| {
            self.groups.push((picked.clone(), Vec::new()));
            self.groups.len() - 1
        });
        let (_, held) = &mut self.groups[at];
        if !held.iter().any(|h| h.value == value) {
            held.push(Held { value, by, next_by });
        }
    }
}

/// What the way reads on from a value: an object's field `next`, or all of a list or a value
/// with no fields, which a message may show.
fn toward(value: &Value, next: &str) -> Value {
    match value {
        Value::Object(map) => field(map, next),
        value => value.clone(),
    }
}

fn field(map: &Map<String, Value>, next: &str) -> Value {
    Value::Object(
        map.get(next)
            .map(|value| (next.to_string(), value.clone()))
            .into_iter()
            .collect(),
    )
}

/// Why `pointer` cannot go on from `h`, the value at `tokens`, by `token`.
fn off_the_way(
    pointer: &str,
    tokens: &[&str],
    token: &str,
    h: &Held<'_>,
    found: Step<'_>,
    name: &str,
) -> (String, String) {
    let by = match found {
        Step::Found(_) => h.next_by,
        _ => h.by,
    };
    let place = place(by);
    let here = pointer_of(&[tokens, &[token]].concat());
    let (field, state) = match found {
        Step::Found(Value::Null) => (dotted(&here), "is null"),
        Step::Missing => (dotted(&here), "is missing"),
        Step::Found(leaf) => {
            let field = dotted(&here);
            return match by {
                None => (
                    format!("`{pointer}` passes through `{field}`, which is not an object"),
                    "point `sets` at a field of an object in `model`, such as \"/resistance\""
                        .into(),
                ),
                Some(by) => chosen_leaf(pointer, &field, leaf, by, &place),
            };
        }
        found => {
            let Some((decl, arm)) = by else {
                let parent = h.value.as_ref().expect("an off way steps as missing");
                return not_passable(pointer, tokens, token, parent, found)
                    .expect("a step that is not found");
            };
            let field = dotted(&pointer_of(tokens));
            let value = h.value.as_ref().expect("an off way steps as missing");
            let sets = format!(
                "`{pointer}` passes through `{field}`, which choice param `{}` sets to {value} in its choice `{arm}`",
                decl.name
            );
            return match found {
                Step::PastEnd(_) => (
                    sets,
                    format!(
                        "give `{field}` at least {} {place}, or point `sets` at a field every choice of `{}` keeps",
                        items(token.parse::<usize>().map_or(1, |index| index + 1)),
                        decl.name
                    ),
                ),
                Step::NotIndex => (
                    sets,
                    format!(
                        "index an item of `{field}` by number, such as \"{}/0\"",
                        pointer_of(tokens)
                    ),
                ),
                _ => chosen_leaf(pointer, &field, value, (decl, arm), &place),
            };
        }
    };
    (
        format!(
            "`{pointer}` passes through `{field}`, which {state} {place}, and no param turns it on, so this value never takes effect"
        ),
        format!(
            "write `{field}` as an object {place}, or give `{name}` \"default\": null so that setting it turns `{field}` on"
        ),
    )
}

/// A choice writes `field` as a value with no fields, such as `{ "/bounce": 5 }`.
fn chosen_leaf(
    pointer: &str,
    field: &str,
    value: &Value,
    (decl, arm): (&ParamDecl, &str),
    place: &str,
) -> (String, String) {
    (
        format!(
            "`{pointer}` passes through `{field}`, which choice param `{}` sets to {value} in its choice `{arm}`",
            decl.name
        ),
        format!("write `{field}` as an object {place}, or as null to turn it off"),
    )
}

/// "1 item" or "`n` items".
fn items(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}

/// Why a pointer cannot go on from `parent`, the value at `tokens`, by `token`, a step that is
/// neither an object's key nor an item a list has.
fn not_passable(
    pointer: &str,
    tokens: &[&str],
    token: &str,
    parent: &Value,
    found: Step<'_>,
) -> Option<(String, String)> {
    let need = || items(token.parse::<usize>().map_or(1, |index| index + 1));
    let field = dotted(&pointer_of(tokens));
    let shown = if field.is_empty() {
        "model".to_string()
    } else {
        field
    };
    match found {
        Step::Found(_) | Step::Missing => None,
        Step::PastEnd(0) => Some((
            format!("`{pointer}` indexes into `{shown}`, which is an empty list"),
            format!(
                "give `{shown}` at least {} in `model`, or point `sets` at a field `model` has",
                need()
            ),
        )),
        Step::PastEnd(len) => Some((
            format!(
                "`{pointer}` indexes past the end of `{shown}`, which has {}",
                items(len)
            ),
            format!(
                "give `{shown}` at least {} in `model`, or index an item it has, from 0 to {}",
                need(),
                len - 1
            ),
        )),
        Step::NotIndex => Some((
            format!("`{pointer}` names a key in `{shown}`, which is a list"),
            format!(
                "index an item of `{shown}` by number, such as \"{}/0\"",
                pointer_of(tokens)
            ),
        )),
        Step::Leaf => Some((
            format!("`{pointer}` passes through `{shown}`, which is {parent}, not an object"),
            "point `sets` at a field of an object in `model`, such as \"/resistance\"".into(),
        )),
    }
}

fn decode_choices(node: &Node) -> Vec<(String, Vec<(String, Node)>)> {
    node.props()
        .unwrap_or_default()
        .iter()
        .map(|choice| {
            let sets = choice
                .value
                .get("sets")
                .and_then(Node::props)
                .unwrap_or_default();
            (
                choice.key.clone(),
                sets.iter()
                    .map(|s| (s.key.clone(), s.value.clone()))
                    .collect(),
            )
        })
        .collect()
}

fn decode_bodies(model: &Node, part: &Part, out: &mut Findings) -> Vec<Body> {
    let Some(props) = model.props() else {
        schema(
            out,
            model,
            "an engine part, or an object of named engine parts",
        );
        return Vec::new();
    };
    if model.get("kind").is_some() {
        return decode_body(None, model, part, out).into_iter().collect();
    }
    let mut bodies = Vec::new();
    for prop in props {
        if !name_ok(&prop.key, prop.key_span, NameKind::Name, "engine part", out) {
            continue;
        }
        if let Some(pin) = part
            .pins
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(&prop.key))
        {
            out.push(
                Code::Name,
                prop.key_span,
                format!("engine part `{}` looks like pin `{}`", prop.key, pin.name),
                "name engine parts apart from pins, such as `host_tx` beside `TXD`".into(),
            );
        }
        bodies.extend(decode_body(Some(prop.key.clone()), &prop.value, part, out));
    }
    bodies
}

fn decode_body(name: Option<String>, node: &Node, part: &Part, out: &mut Findings) -> Option<Body> {
    let Some(kind_node) = node.get("kind") else {
        schema(out, node, "an engine part with a `kind`");
        return None;
    };
    let Some(kind) = kind_node.str() else {
        schema(out, kind_node, "an engine kind such as \"resistor\"");
        return None;
    };
    let kinds = engine_kinds();
    let Some(engine_pins) = kinds
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, pins)| *pins)
    else {
        let named = did_you_mean(kind, kinds.iter().map(|(name, _)| *name), "");
        let hint = if named.is_empty() {
            format!(
                "the kinds are {}",
                kinds
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            named.trim_end().to_string()
        };
        out.push(
            Code::Syntax,
            kind_node.span,
            format!("`{kind}` is not an engine kind this reader knows"),
            hint,
        );
        return None;
    };
    let map = node.get("pins").and_then(Node::props).unwrap_or_default();
    for entry in map {
        if !engine_pins.contains(&entry.key.as_str()) {
            out.push(
                Code::UnknownPin,
                entry.key_span,
                format!("the engine's `{kind}` has no pin `{}`", entry.key),
                format!("its pins: {}", engine_pins.join(", ")),
            );
        }
    }
    let mut pins = Vec::new();
    for &engine_pin in engine_pins {
        let (part_pin, span) = match map.iter().find(|e| e.key == engine_pin) {
            Some(entry) => (
                entry.value.str().unwrap_or_default().to_string(),
                entry.value.span,
            ),
            None => (engine_pin.to_string(), kind_node.span),
        };
        if part.pin(&part_pin).is_none() {
            out.push(
                Code::UnknownPin,
                span,
                format!("engine pin `{engine_pin}` sits on `{part_pin}`, which is not a pin of this part"),
                format!("map it in the model's `pins`; the part's pins: {}", part.pin_list()),);
            continue;
        }
        pins.push((engine_pin, part_pin));
    }
    Some(Body {
        name,
        kind: kind.to_string(),
        pins,
    })
}
