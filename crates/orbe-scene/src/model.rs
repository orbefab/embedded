//! The files' model: what a scene or a part file says, decoded from the syntax tree. Each
//! decoder reports what one file alone can show, and keeps going after a finding.

use serde_json::Value;

use crate::diagnostic::{Code, Findings, Reported};
use crate::kinds;
use crate::name::{NameKind, did_you_mean, problem};
use crate::params::{Step, dotted, pointer_of, step, tokens, within};
use crate::shape::{self, ParamShape, schema, unknown_key};
use crate::source::{FileId, Sources, Span};
use crate::syntax::{Node, Prop};
use crate::units::{self, Unit};

const SCENE_KEYS: &[&str] = &[
    "format", "$schema", "parts", "wires", "layout", "ground", "nets", "sim",
];
const FORMATS: &[u32] = &[1];
const WIRE_KINDS: &[&str] = &["jumper", "trace", "contact"];
/// A live session's stop, investigation 1's 2-hour cap.
const STOP_PS: u64 = 7_200 * 1_000_000_000_000;
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
        min_exclusive: Option<f64>,
    },
    Choice(Vec<(String, Vec<(String, Node)>)>),
    File {
        text: bool,
    },
    /// No `sets`: a part with children hands it to a child with `{ "param": "<name>" }`.
    Through,
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
        self.writes().into_iter().map(|(p, _)| p).collect()
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

    /// Each choice that writes `at`: its param, its name and its value there. Also whether one
    /// param writes `at` in every choice, so the value `model` has there never shows.
    fn chosen(&self, at: &str) -> (Vec<(&ParamDecl, &str, &Node)>, bool) {
        let mut arms = Vec::new();
        let mut every = false;
        for decl in &self.params {
            let ParamKind::Choice(choices) = &decl.kind else {
                continue;
            };
            let before = arms.len();
            for (name, sets) in choices {
                if let Some((_, value)) = sets.iter().find(|(p, _)| p == at) {
                    arms.push((decl, name.as_str(), value));
                }
            }
            every |= !choices.is_empty() && arms.len() - before == choices.len();
        }
        (arms, every)
    }

    pub(crate) fn pin_list(&self) -> String {
        self.pins
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(crate) struct Scene {
    pub(crate) file: FileId,
    /// The scene's root object, where a finding about the whole scene points.
    pub(crate) span: Span,
    pub(crate) comp: Comp,
    pub(crate) nets: Vec<(String, Span, Ref)>,
    pub(crate) seed: u64,
    pub(crate) max_step_ps: u64,
}

impl Scene {
    pub(crate) const STOP_PS: u64 = STOP_PS;
}

/// Checks `format`. A file in another format is not read further.
fn format_ok(file: FileId, root: &Node, out: &mut Findings) -> bool {
    let reads = FORMATS
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    match root.get("format") {
        None => {
            out.push(
                Code::Format,
                Span::start_of(file),
                "the file has no `format`",
                format!("add \"format\": 1; this binary reads format {reads}"),
            );
            false
        }
        Some(node)
            if node
                .number()
                .is_some_and(|n| FORMATS.iter().any(|f| f64::from(*f) == n)) =>
        {
            true
        }
        Some(node) => {
            out.push(
                Code::Format,
                node.span,
                format!("format {} is not one this binary reads", node.to_value()),
                format!("this binary reads format {reads}"),
            );
            false
        }
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
    if !format_ok(file, root, out) {
        return None;
    }
    let mut scene = Scene {
        file,
        span: root.span,
        comp: Comp::default(),
        nets: Vec::new(),
        seed: 0,
        max_step_ps: DEFAULT_MAX_STEP_PS,
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
            key if shape::PART.iter().any(|(k, _)| *k == key) => out.push(
                Code::Role,
                prop.key_span,
                format!("a scene has no `{key}`; it is a part's key, and this file is read as a scene"),
                "a part file is used from a scene, as \"btn1\": { \"type\": \"./parts/my-part.jsonc\" }".into(),),
            _ => unknown_key(out, prop, SCENE_KEYS, "a scene"),
        }
    }
    if let Some(layout) = root.get("layout") {
        decode_layout(layout, &scene.comp, out);
    }
    Some(scene)
}

pub(crate) fn decode_part(
    sources: &Sources,
    file: FileId,
    label: String,
    root: &Node,
    out: &mut Findings,
) -> Option<Part> {
    let Some(props) = root.props() else {
        schema(out, root, "an object");
        return None;
    };
    if !format_ok(file, root, out) {
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
        reported: None,
    };
    for prop in props {
        let value = &prop.value;
        let Some((_, value_shape)) = shape::PART.iter().find(|(k, _)| *k == prop.key) else {
            match prop.key.as_str() {
                key @ ("sim" | "nets") => out.push(
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
        decode_layout(layout, &part.comp, out);
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

fn is_id(text: &str) -> bool {
    let plain = |s: &str| {
        s.len() <= 64
            && s.starts_with(|c: char| c.is_ascii_lowercase())
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    match text.strip_prefix('@').and_then(|t| t.split_once('/')) {
        Some((registry, id)) => plain(registry) && plain(id),
        None => plain(text),
    }
}

fn classify_type(text: &str, span: Span, out: &mut Findings) -> Option<TypeRef> {
    if text.starts_with("./") || text.starts_with("../") {
        Some(TypeRef::Path(text.to_string()))
    } else if is_id(text) {
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

fn decode_layout(node: &Node, comp: &Comp, out: &mut Findings) {
    let Some(props) = node.props() else {
        out.push(
            Code::Layout,
            node.span,
            "`layout` is an object of placements by part name",
            "each key is a part name and each value is { \"at\": [x, y] }".into(),
        );
        return;
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
                        "{}its keys are at and rotate",
                        did_you_mean(key, ["at", "rotate"], "")
                    ),
                ),
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
                Some(Some("text")) => ParamKind::File { text: true },
                Some(Some("blob")) => ParamKind::File { text: false },
                Some(_) => continue,
                None => {
                    out.push(
                        Code::Param,
                        prop.key_span,
                        format!(
                            "file param `{}` needs \"as\": \"text\" or \"blob\"",
                            prop.key
                        ),
                        "add \"as\": \"text\" or \"as\": \"blob\"".into(),
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
                        "a param's unit is one of ohm, F, V, A, s, Hz, W and 1",
                        "use one of ohm, F, V, A, s, Hz, W or 1".into(),
                    );
                    continue;
                };
                ParamKind::Number {
                    unit: parsed,
                    min_exclusive: field("min_exclusive").and_then(Node::number),
                }
            }
            ParamShape::Through => ParamKind::Through,
        };
        let sets = match kind {
            ParamKind::Number { .. } | ParamKind::File { .. } => {
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
        if !through && !matches!(kind, ParamKind::Choice(_)) && field("sets").is_none() {
            out.push(
                Code::Param,
                prop.key_span,
                format!("param `{}` has no `sets` pointer into `model`", prop.key),
                "add \"sets\": \"/field\" pointing at the model field this param writes".into(),
            );
        }
        if through && part.comp.parts.is_empty() {
            out.push(Code::Param, prop.key_span, format!("param `{}` has no unit, choices or file, and this part has no children to pass it to", prop.key), "give this part a child that takes the param, or give the param a unit, choices or file".into());
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

/// A value the object or list at one point on a pointer's way may hold once the params are
/// applied, and the choice that wrote it, when one did. `None` once the way is off: the
/// objects from there on are made by the param that turns them on.
struct Held<'p> {
    value: Option<Value>,
    by: Option<(&'p ParamDecl, &'p str)>,
}

impl Held<'_> {
    /// Where the value comes from, for a message: "in `model`" or "in choice `play`'s `one`".
    fn place(&self) -> String {
        match self.by {
            None => "in `model`".into(),
            Some((decl, arm)) => format!("in choice `{}`'s `{arm}`", decl.name),
        }
    }
}

/// Every value each point on the way may hold is checked: `model`'s, unless one param writes
/// that point in every choice, and each value a choice writes there, with the objects and
/// lists inside it. The pointer is good only when each of them has the way.
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
    let (last, through) = tokens.split_last().expect("split yields one token");
    let mut held = vec![Held {
        value: Some(model.clone()),
        by: None,
    }];
    for (i, token) in through.iter().enumerate() {
        let here = pointer_of(&tokens[..=i]);
        let (arms, every) = part.chosen(&here);
        let mut stepped = Vec::new();
        for h in &held {
            let found = h.value.as_ref().map_or(Step::Missing, |v| step(v, token));
            match found {
                Step::Found(next) if next.is_object() || next.is_array() => stepped.push(Held {
                    value: Some(next.clone()),
                    by: h.by,
                }),
                _ if every => {}
                Step::Found(Value::Null) | Step::Missing if part.opener(&here).is_some() => {
                    stepped.push(Held {
                        value: None,
                        by: h.by,
                    });
                }
                found => return Some(off_the_way(pointer, &tokens[..i], token, h, found, name)),
            }
        }
        let mut next: Vec<Held> = Vec::new();
        for (decl, arm, value) in arms {
            let by = Some((decl, arm));
            match value.to_value() {
                Value::Null => {}
                // An object a choice writes merges into the one there.
                Value::Object(written) => {
                    let bases: Vec<_> = stepped
                        .iter()
                        .filter_map(|h| h.value.as_ref()?.as_object())
                        .collect();
                    if bases.is_empty() {
                        next.push(Held {
                            value: Some(Value::Object(written)),
                            by,
                        });
                    } else {
                        for base in bases {
                            let mut value = base.clone();
                            value.extend(written.clone());
                            next.push(Held {
                                value: Some(Value::Object(value)),
                                by,
                            });
                        }
                    }
                }
                value => next.push(Held {
                    value: Some(value),
                    by,
                }),
            }
        }
        if !every {
            next.extend(stepped);
        }
        held = next;
    }
    held.iter().find_map(|h| {
        let value = h.value.as_ref()?;
        match step(value, last) {
            _ if value.is_object() => None,
            Step::Found(_) => None,
            found => Some(off_the_way(pointer, through, last, h, found, name)),
        }
    })
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
    let place = h.place();
    let here = pointer_of(&[tokens, &[token]].concat());
    let (field, state) = match found {
        Step::Found(Value::Null) => (dotted(&here), "is null"),
        Step::Missing => (dotted(&here), "is missing"),
        Step::Found(leaf) => {
            let field = dotted(&here);
            return match h.by {
                None => (
                    format!("`{pointer}` passes through `{field}`, which is not an object"),
                    "point `sets` at a field of an object in `model`, such as \"/resistance\""
                        .into(),
                ),
                Some(_) => chosen_leaf(pointer, &field, leaf, h, &place),
            };
        }
        found => {
            let Some((decl, arm)) = h.by else {
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
                _ => chosen_leaf(pointer, &field, value, h, &place),
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
    h: &Held<'_>,
    place: &str,
) -> (String, String) {
    let (decl, arm) = h.by.expect("a value a choice wrote");
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
    let Some(engine_pins) = kinds::pins(kind) else {
        let named = did_you_mean(kind, kinds::KINDS.iter().map(|(k, _)| *k), "");
        let hint = if named.is_empty() {
            format!(
                "the kinds are {}",
                kinds::KINDS
                    .iter()
                    .map(|(k, _)| *k)
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
