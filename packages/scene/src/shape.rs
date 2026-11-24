//! The keys a file may hold and the type of each value. A key the format does not define, or a
//! value of another type, is `scene.syntax` at that key or value. Scenes and part files report
//! an unknown key the same way, through [`unknown_key`].

use crate::diagnostic::{Code, Findings};
use crate::name::did_you_mean;
use crate::params::source_ok;
use crate::syntax::{Json, Node, Prop};

pub(crate) enum Shape {
    /// Read by its own decoder, by the param it feeds, or by the engine.
    Any,
    Str,
    Num,
    Bool,
    OrNull(&'static Shape),
    /// One of these strings.
    Word(&'static [&'static str]),
    List(&'static Shape),
    /// Names of the writer's choosing, each to a value of one shape.
    Map(&'static Shape),
    /// What the object is, for a message, and the keys it may hold.
    Obj(&'static str, &'static [(&'static str, Shape)]),
    /// A value object's `source`.
    Source,
}

/// A value with its unit and source, as `facts`, `readings` and `joints` write one.
const VALUE: Shape = Shape::Obj(
    "a value object",
    &[
        ("value", Shape::Num),
        ("unit", Shape::Str),
        ("source", Shape::Source),
    ],
);

const NAME_OR_NULL: Shape = Shape::OrNull(&Shape::Str);

const REAL: Shape = Shape::Obj(
    "`real`",
    &[
        ("maker", NAME_OR_NULL),
        ("number", NAME_OR_NULL),
        ("datasheet", NAME_OR_NULL),
        ("generic", Shape::Bool),
    ],
);

const CHOICE: Shape = Shape::Obj(
    "a choice",
    &[
        ("sets", Shape::Map(&Shape::Any)),
        ("tint", Shape::Str),
        ("real", REAL),
    ],
);

const PARAM_KEYS: &[(&str, Shape)] = &[
    ("unit", Shape::Str),
    ("choices", Shape::Map(&CHOICE)),
    ("file", Shape::Str),
    ("as", Shape::Word(&["text", "blob"])),
    ("default", Shape::Any),
    ("example", Shape::Any),
    ("required", Shape::Bool),
    ("min_exclusive", Shape::Num),
    ("max_exclusive", Shape::Num),
    ("sets", Shape::Str),
    ("live", Shape::Bool),
];

const PARAM: Shape = Shape::Obj("a param declaration", PARAM_KEYS);

#[derive(Clone, Copy)]
pub(crate) enum ParamShape {
    Choice,
    File,
    Number,
    /// No unit, choices or file: a part with children hands the value to a child, whose param
    /// checks it.
    Through,
}

/// A param's kinds, told apart by their keys: what each is called, the key that marks it, of
/// which the first a declaration holds decides, and the keys that kind reads.
const PARAM_KINDS: &[(ParamShape, &str, Option<&str>, &[&str])] = &[
    (
        ParamShape::Choice,
        "a choice param",
        Some("choices"),
        &["choices", "default", "example", "required"],
    ),
    (
        ParamShape::File,
        "a file param",
        Some("file"),
        &["file", "as", "default", "example", "required", "sets"],
    ),
    (
        ParamShape::Number,
        "a number param",
        Some("unit"),
        &[
            "unit",
            "default",
            "example",
            "required",
            "min_exclusive",
            "max_exclusive",
            "sets",
            "live",
        ],
    ),
    (
        ParamShape::Through,
        "a param that passes its value through",
        None,
        &["default", "required"],
    ),
];

/// The kind of a param declaration, with a `scene.syntax` finding at each key that kind does
/// not read, at `"required": true` beside a `default`, and at an `example` beside a `default`.
/// [`check`] reports a key no kind has,
/// such as a misspelt `unit`, and then the kind is not known, so its keys wait for the next run.
pub(crate) fn param_shape(node: &Node, out: &mut Findings) -> ParamShape {
    let props = node.props().unwrap_or_default();
    let has = |key: &str| props.iter().any(|p| p.key == key);
    let (kind, what, mark, reads) = PARAM_KINDS
        .iter()
        .find(|(_, _, mark, _)| mark.is_none_or(has))
        .expect("the last kind has no mark");
    let known = props
        .iter()
        .all(|p| keys(PARAM_KEYS).contains(&p.key.as_str()));
    let or = match mark {
        Some(_) => String::new(),
        None => ", or give it a unit, choices or file to make it a param of that kind".into(),
    };
    for prop in props {
        let key = prop.key.as_str();
        if known && !reads.contains(&key) {
            out.push(
                Code::Syntax,
                prop.key_span,
                format!("{what} does not read `{key}`"),
                format!("remove `{key}`{or}; {what} reads only {}", reads.join(", ")),
            );
        }
        if key == "required" && matches!(prop.value.json, Json::Bool(true)) && has("default") {
            out.push(
                Code::Syntax,
                prop.key_span,
                "a param with a `default` is not required",
                "remove \"required\": true, or remove `default` so that every use must set it"
                    .into(),
            );
        }
        if key == "example" && has("default") {
            out.push(
                Code::Syntax,
                prop.key_span,
                "a param with a `default` has no `example`",
                "remove `example`, or remove `default` so that every use must set it".into(),
            );
        }
    }
    *kind
}

const READING: Shape = Shape::Obj(
    "a reading",
    &[("of", Shape::Str), ("above", VALUE), ("full", VALUE)],
);

const CONTROLS: Shape = Shape::Obj(
    "`controls`",
    &[(
        "press",
        Shape::Obj("a control", &[("input", Shape::Str), ("joint", Shape::Str)]),
    )],
);

const JOINT: Shape = Shape::Obj(
    "a joint",
    &[
        ("node", Shape::Str),
        ("slide", Shape::List(&Shape::Num)),
        ("turn", Shape::List(&Shape::Num)),
        ("range", VALUE),
        ("by", Shape::Str),
    ],
);

const NODE_BY: Shape = Shape::Obj("a view entry", &[("node", Shape::Str), ("by", Shape::Str)]);

const VIEW: Shape = Shape::Obj(
    "`view`",
    &[
        ("glow", Shape::List(&NODE_BY)),
        ("tint", Shape::List(&NODE_BY)),
        (
            "recolour",
            Shape::List(&Shape::Obj(
                "a recolour entry",
                &[
                    ("nodes", Shape::List(&Shape::Str)),
                    ("by", Shape::Str),
                    ("rule", Shape::Str),
                ],
            )),
        ),
        ("screen", Shape::List(&NODE_BY)),
    ],
);

const NOT_SIMULATED: Shape = Shape::Obj(
    "a not_simulated entry",
    &[
        ("what", Shape::Str),
        ("why", Shape::Str),
        ("pins", Shape::List(&Shape::Str)),
    ],
);

const BUY: Shape = Shape::Obj(
    "a buy entry",
    &[
        ("seller", Shape::Str),
        ("url", Shape::Str),
        ("checked", Shape::Str),
    ],
);

const HOLES: Shape = Shape::Obj(
    "`holes`",
    &[
        ("depth", Shape::Num),
        ("strips", Shape::List(&Shape::List(&Shape::Str))),
    ],
);

/// A part file's keys. `model` is the engine's, so the engine checks it.
pub(crate) const PART: &[(&str, Shape)] = &[
    ("format", Shape::Any),
    ("$schema", Shape::Str),
    ("part", Shape::Str),
    ("version", Shape::Str),
    ("orbe", Shape::Str),
    ("title", Shape::Str),
    // A released part file may still say `"listed": false`. The key parses and is ignored.
    // `registry.jsonc`'s `unlisted` decides what search and the site list.
    ("listed", Shape::Bool),
    ("summary", Shape::Str),
    ("tags", Shape::List(&Shape::Str)),
    ("license", Shape::Str),
    ("docs", Shape::Map(&NAME_OR_NULL)),
    ("real", REAL),
    ("uses", Shape::Map(&Shape::Str)),
    ("pins", Shape::Any),
    ("model", Shape::Any),
    ("params", Shape::Map(&PARAM)),
    ("parts", Shape::Any),
    ("wires", Shape::Any),
    ("layout", Shape::Any),
    ("ground", Shape::Any),
    ("readings", Shape::Map(&READING)),
    ("controls", CONTROLS),
    ("joints", Shape::Map(&JOINT)),
    ("facts", Shape::Map(&VALUE)),
    ("view", VIEW),
    ("not_simulated", Shape::List(&NOT_SIMULATED)),
    ("buy", Shape::List(&BUY)),
    ("joins_only", Shape::Bool),
    ("open", Shape::List(&Shape::Str)),
    ("holes", HOLES),
];

pub(crate) fn keys(fields: &[(&'static str, Shape)]) -> Vec<&'static str> {
    fields.iter().map(|(k, _)| *k).collect()
}

/// Reports every key and value under `node` that `shape` does not allow. `key` holds `node`,
/// for a message.
pub(crate) fn check(node: &Node, shape: &Shape, key: Option<&str>, out: &mut Findings) {
    match (shape, &node.json) {
        (Shape::Any, _)
        | (Shape::Str, Json::Str(_))
        | (Shape::Num, Json::Num(_))
        | (Shape::Bool, Json::Bool(_))
        | (Shape::OrNull(_), Json::Null) => {}
        (Shape::OrNull(inner), _) if inner.allows_type(node) => check(node, inner, key, out),
        (Shape::Word(words), Json::Str(word)) if !words.contains(&word.as_str()) => out.push(
            Code::Syntax,
            node.span,
            format!("`{word}` is not one of {}", quoted(words)),
            format!(
                "{}write {}",
                did_you_mean(word, words.iter().copied(), ""),
                quoted(words)
            ),
        ),
        (Shape::Word(_), Json::Str(_)) => {}
        (Shape::List(item), Json::Arr(items)) => {
            items.iter().for_each(|i| check(i, item, None, out));
        }
        (Shape::Map(value), Json::Obj(props)) => {
            for prop in props {
                check(&prop.value, value, Some(&prop.key), out);
            }
        }
        (Shape::Obj(what, fields), Json::Obj(props)) => {
            for prop in props {
                match fields.iter().find(|(k, _)| *k == prop.key) {
                    Some((_, shape)) => check(&prop.value, shape, Some(&prop.key), out),
                    None => unknown_key(out, prop, &keys(fields), what),
                }
            }
        }
        (Shape::Source, _) if source_ok(node) => {}
        _ => {
            let expected = shape.expected();
            let message = match key {
                Some(key) => format!("`{key}` takes {expected}, found {}", node.what()),
                None => format!("expected {expected}, found {}", node.what()),
            };
            out.push(
                Code::Syntax,
                node.span,
                message,
                format!("write {expected}"),
            );
        }
    }
}

impl Shape {
    /// Whether `node` is of the JSON type this shape takes, whatever it holds.
    fn allows_type(&self, node: &Node) -> bool {
        matches!(
            (self, &node.json),
            (Shape::Any, _)
                | (Shape::Str | Shape::Word(_), Json::Str(_))
                | (Shape::Num, Json::Num(_))
                | (Shape::Bool, Json::Bool(_))
                | (Shape::List(_), Json::Arr(_))
                | (Shape::Map(_) | Shape::Obj(..), Json::Obj(_))
                | (Shape::Source, Json::Str(_) | Json::Obj(_))
        )
    }

    fn expected(&self) -> String {
        match self {
            Shape::Any => "a value".into(),
            Shape::Str => "a string".into(),
            Shape::Num => "a number".into(),
            Shape::Bool => "true or false".into(),
            Shape::OrNull(inner) => format!("{} or null", inner.expected()),
            Shape::Word(words) => quoted(words),
            Shape::List(_) => "a list".into(),
            Shape::Map(_) | Shape::Obj(..) => "an object".into(),
            Shape::Source => "a source: \"user\", { \"closed_form\": { \"note\" } }, or { \"datasheet\": { \"doc\", \"at\", \"basis\" } }".into(),
        }
    }
}

fn quoted(words: &[&str]) -> String {
    let words: Vec<String> = words.iter().map(|w| format!("\"{w}\"")).collect();
    match words.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// A key `what` does not have, with the closest of its `keys` when one is near.
pub(crate) fn unknown_key(out: &mut Findings, prop: &Prop, keys: &[&str], what: &str) {
    let hint = did_you_mean(&prop.key, keys.iter().copied(), "");
    out.push(
        Code::Syntax,
        prop.key_span,
        format!("{what} has no key `{}`", prop.key),
        format!("{hint}its keys are {}", keys.join(", ")),
    );
}

/// A value of another JSON type than `expected`.
pub(crate) fn schema(out: &mut Findings, node: &Node, expected: &str) {
    out.push(
        Code::Syntax,
        node.span,
        format!("expected {expected}, found {}", node.what()),
        format!("write {expected}"),
    );
}
