//! The engine's findings on the netlist the reader wrote, placed in the files that wrote it.

use serde_json::{Value, json};

use super::{Circuit, Written};
use crate::diagnostic::Code;
use crate::source::Span;
use crate::syntax::Node;

impl<'p> Circuit<'p> {
    /// An engine finding placed at the value that set the field, or else at the field in the
    /// part file's model. When `quiet`, the reader's own findings or a refused part may be its
    /// cause, so only a finding about one part's values is kept.
    pub(super) fn engine_finding(
        &mut self,
        code: &'static str,
        value: &Value,
        origin: Option<&Written>,
        quiet: bool,
    ) {
        let message = value["message"].as_str().unwrap_or_default().to_string();
        let about_values = code == "netlist.syntax" || value["data"]["field"].is_string();
        let Some(origin) = origin.filter(|_| about_values || !quiet) else {
            if !quiet {
                self.out.push(
                    Code::Engine(code),
                    Span::start_of(self.project.scene.file),
                    message,
                    "change the value so it meets the engine's rule".into(),
                );
            }
            return;
        };
        if self.failed.contains(&origin.inst) {
            return;
        }
        if code == "netlist.syntax" {
            let detail = value["data"]["detail"].as_str().unwrap_or_default();
            self.schema_finding(origin, detail);
            return;
        }
        let field = value["data"]["field"].as_str().unwrap_or_default();
        let rule = value["data"]["rule"].as_str().unwrap_or_default();
        let (requirement, hint) = match rule {
            "bounce_room" => {
                let edges = origin
                    .fields
                    .pointer("/bounce/max_bounces/value")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let least = duration(2 * edges * sim_core::netlist::MIN_GAP_PS);
                (
                    format!("with max_bounces {edges}, it must be at least {least}"),
                    format!("write a bounce of at least {least}, or lower max_bounces"),
                )
            }
            "positive" => (
                requirement(&message),
                "write a value greater than zero".into(),
            ),
            _ => (
                requirement(&message),
                "change the value so it meets the engine's rule".into(),
            ),
        };
        if let Some((_, setter)) = origin.set_by.iter().find(|(f, _)| {
            !field.is_empty() && (f.starts_with(field) || field.starts_with(f.as_str()))
        }) {
            self.out.push(
                Code::Param,
                setter.span,
                format!("`{}` is {}; {requirement}", setter.site, setter.shown),
                hint,
            );
            return;
        }
        let model = self.body_model(origin);
        let span = model.at_field(field).map_or(model.span, |n| n.span);
        self.out.push(
            Code::Engine(code),
            span,
            format!("`{}`: {message}", self.insts[origin.inst].dotted()),
            hint,
        );
    }

    /// The part file's model for one engine part.
    fn body_model(&self, origin: &Written) -> &'p Node {
        let part = self.part(origin.inst).expect("an engine part has a part");
        let model = part.model.as_ref().expect("an engine part has a model");
        origin
            .body
            .as_ref()
            .and_then(|name| model.get(name))
            .unwrap_or(model)
    }

    /// The engine's schema refused a part. Its message names the key or shows the value, so
    /// the finding goes to the value that set it, or else to that key or value in the model.
    fn schema_finding(&mut self, origin: &Written, detail: &str) {
        let detail = detail.rsplit_once(" at line ").map_or(detail, |(d, _)| d);
        let subject = Subject::of(detail);
        let (what, hint) = subject.words(detail);
        let setter = origin.set_by.iter().find(|(field, _)| match &subject {
            Subject::Key(key) => field.rsplit('.').next() == Some(key.as_str()),
            Subject::Seen(seen, _) => origin
                .fields
                .pointer(&format!("/{}", field.replace('.', "/")))
                .is_some_and(|v| holds(v, seen)),
            Subject::Other => false,
        });
        if let Some((_, setter)) = setter {
            self.out.push(
                Code::Param,
                setter.span,
                format!("`{}`: {what}", setter.site),
                hint,
            );
            return;
        }
        let model = self.body_model(origin);
        let found = match &subject {
            Subject::Key(key) => {
                find(model, &|n: &Node| n.get(key).is_some()).and_then(|n| n.get(key))
            }
            Subject::Seen(seen, _) => find(model, &|n: &Node| {
                n.props().is_none() && n.items().is_none() && holds(&n.to_value(), seen)
            }),
            Subject::Other => None,
        };
        self.out.push(
            Code::Engine("netlist.syntax"),
            found.map_or(model.span, |n| n.span),
            format!("`{}`: {what}", self.insts[origin.inst].dotted()),
            hint,
        );
    }
}

/// What a schema message is about: a key it names, or a value it shows, with the text it shows.
enum Subject {
    Key(String),
    Seen(Value, String),
    Other,
}

impl Subject {
    fn of(detail: &str) -> Subject {
        let quoted = |s: &str| s.split('`').nth(1).map(str::to_string);
        for prefix in ["missing field ", "unknown field ", "duplicate field "] {
            if let Some(rest) = detail.strip_prefix(prefix) {
                return quoted(rest).map_or(Subject::Other, Subject::Key);
            }
        }
        if let Some(name) = detail.strip_prefix("unknown variant ").and_then(quoted) {
            return Subject::Seen(Value::String(name.clone()), name);
        }
        let Some(rest) = detail
            .strip_prefix("invalid type: ")
            .or_else(|| detail.strip_prefix("invalid value: "))
        else {
            return Subject::Other;
        };
        let seen = rest.split(", expected").next().unwrap_or(rest);
        if seen == "null" {
            return Subject::Seen(Value::Null, "null".into());
        }
        if let Some(text) = seen.strip_prefix("string ") {
            let text = text.trim_matches('"');
            return Subject::Seen(Value::String(text.into()), format!("\"{text}\""));
        }
        if let Some(word) = seen.strip_prefix("boolean ") {
            let word = word.trim_matches('`');
            return Subject::Seen(Value::Bool(word == "true"), word.into());
        }
        match quoted(seen).and_then(|n| n.parse::<f64>().ok().map(|f| (f, n))) {
            Some((number, text)) => Subject::Seen(json!(number), text),
            None => Subject::Other,
        }
    }

    /// The finding's message and hint, in the scene's terms where the message allows.
    fn words(&self, detail: &str) -> (String, String) {
        let expected = detail.split_once(", expected ").map(|(_, e)| e);
        if let (Subject::Seen(seen, text), Some((low, high))) =
            (self, expected.and_then(integer_range))
        {
            let hint = format!("write a whole number from {low} to {high}");
            return match seen.as_f64() {
                Some(n) if n.fract() != 0.0 => (
                    format!(
                        "{text} is not a whole number; the engine takes an integer from {low} to {high} here"
                    ),
                    hint,
                ),
                _ => (format!("{text} is outside {low} to {high}"), hint),
            };
        }
        match self {
            Subject::Key(key) if detail.starts_with("missing field") => (
                format!("the engine needs `{key}`, and nothing writes it"),
                format!("add `{key}` to `model`"),
            ),
            Subject::Key(key) if detail.starts_with("unknown field") => (
                format!("the engine has no field `{key}` here"),
                format!(
                    "point `sets` or the model at a field it has: {}",
                    expected.unwrap_or("see the engine's netlist docs")
                ),
            ),
            _ => (
                format!("the engine does not read this: {detail}"),
                "write the value as the engine's field takes it".into(),
            ),
        }
    }
}

fn integer_range(expected: &str) -> Option<(i128, i128)> {
    Some(match expected {
        "u8" => (0, u8::MAX.into()),
        "u16" => (0, u16::MAX.into()),
        "u32" => (0, u32::MAX.into()),
        "u64" => (0, u64::MAX.into()),
        "i8" => (i8::MIN.into(), i8::MAX.into()),
        "i16" => (i16::MIN.into(), i16::MAX.into()),
        "i32" => (i32::MIN.into(), i32::MAX.into()),
        "i64" => (i64::MIN.into(), i64::MAX.into()),
        _ => return None,
    })
}

/// Whether `value` is `seen` or holds it, numbers compared by value.
fn holds(value: &Value, seen: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|v| holds(v, seen)),
        Value::Object(map) => map.values().any(|v| holds(v, seen)),
        Value::Number(n) => seen.as_f64().is_some_and(|s| n.as_f64() == Some(s)),
        _ => value == seen,
    }
}

/// The first node, depth first, that `matches`.
fn find<'n>(node: &'n Node, matches: &dyn Fn(&Node) -> bool) -> Option<&'n Node> {
    if matches(node) {
        return Some(node);
    }
    let children: Vec<&Node> = match (node.props(), node.items()) {
        (Some(props), _) => props.iter().map(|p| &p.value).collect(),
        (_, Some(items)) => items.iter().collect(),
        _ => Vec::new(),
    };
    children.into_iter().find_map(|c| find(c, matches))
}

/// The engine's requirement, after its own `#part field is value; `.
fn requirement(message: &str) -> String {
    message
        .split_once("; ")
        .map_or(message, |(_, r)| r)
        .to_string()
}

/// A duration in picoseconds in the largest unit that keeps it whole, such as `8ns`.
fn duration(ps: u64) -> String {
    [
        ("s", 1_000_000_000_000),
        ("ms", 1_000_000_000),
        ("us", 1_000_000),
        ("ns", 1_000),
    ]
    .into_iter()
    .find(|(_, scale)| ps != 0 && ps.is_multiple_of(*scale))
    .map_or(format!("{ps}ps"), |(unit, scale)| {
        format!("{}{unit}", ps / scale)
    })
}
