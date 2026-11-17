//! The JSONC front-end: text to a tree with a span on every value and key. Nothing past this
//! module knows the syntax.

use jsonc_parser::ast::{ObjectPropName, Value as Ast};
use jsonc_parser::errors::ParseErrorKind;
use jsonc_parser::{CollectOptions, ParseOptions, parse_to_ast};
use serde_json::Value;

use crate::diagnostic::{Code, Finding, Findings};
use crate::source::{FileId, Sources, Span};

#[derive(Clone, Debug)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    /// The number as written, so integers stay integers and floats keep their digits.
    Num(String),
    Str(String),
    Arr(Vec<Node>),
    Obj(Vec<Prop>),
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) span: Span,
    pub(crate) json: Json,
}

#[derive(Clone, Debug)]
pub(crate) struct Prop {
    pub(crate) key: String,
    pub(crate) key_span: Span,
    pub(crate) value: Node,
}

/// The first parse of one file. The tree is `None` when the file did not parse. `findings`
/// is what that parse pushed, so a later call reports the same ones.
#[derive(Clone)]
pub(crate) struct Cached {
    node: Option<Node>,
    findings: Vec<Finding>,
}

/// JSON as RFC 8259 defines it, plus `//` and `/* */` comments and trailing commas. Every
/// looser option jsonc-parser has is off, so a missing comma is an error and not read quietly.
fn options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

pub(crate) fn parse(sources: &Sources, file: FileId, out: &mut Findings) -> Option<Node> {
    if let Some(cached) = sources.parsed(file) {
        out.extend_from(&cached.findings);
        return cached.node;
    }
    #[cfg(test)]
    record_parse(&sources.file(file).display);
    let mark = out.len();
    let node = parse_fresh(sources, file, out);
    sources.remember(
        file,
        Cached {
            node: node.clone(),
            findings: out.copied_since(mark),
        },
    );
    node
}

fn parse_fresh(sources: &Sources, file: FileId, out: &mut Findings) -> Option<Node> {
    let text = &sources.file(file).text;
    let parsed = match parse_to_ast(text, &CollectOptions::default(), &options()) {
        Ok(parsed) => parsed,
        Err(error) => {
            let range = error.range();
            let start = text[..range.start]
                .trim_end_matches(|c: char| c.is_ascii_digit() || c == '.')
                .len();
            let span = Span {
                file,
                start,
                end: range.end,
            };
            out.push(
                Code::Syntax,
                span,
                error.kind().to_string(),
                trap_hint(text, start, error.kind()),
            );
            return None;
        }
    };
    let Some(value) = parsed.value else {
        out.push(
            Code::Syntax,
            Span::start_of(file),
            "the file holds no JSON value",
            "write one JSON value".into(),
        );
        return None;
    };
    let node = convert(file, &value);
    check_duplicate_keys(sources, &node, out);
    let before = out.len();
    check_numbers(&node, out);
    (out.len() == before).then_some(node)
}

/// A number JSON writes but no 64-bit float holds, such as `1e400`, is not read as anything
/// else.
fn check_numbers(node: &Node, out: &mut Findings) {
    match &node.json {
        Json::Num(raw) if serde_json::from_str::<Value>(raw).is_err() => out.push(
            Code::Syntax,
            node.span,
            format!("{raw} is not a finite number"),
            "write a number from -1.7e308 to 1.7e308, such as 1e9, or a string such as \"1G\""
                .into(),
        ),
        Json::Arr(items) => items.iter().for_each(|item| check_numbers(item, out)),
        Json::Obj(props) => props.iter().for_each(|p| check_numbers(&p.value, out)),
        _ => {}
    }
}

/// The library's messages for the likeliest slips are weak, so name the fix.
fn trap_hint(text: &str, at: usize, kind: &ParseErrorKind) -> String {
    let rest = &text[at.min(text.len())..];
    let word: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && !",]}".contains(*c))
        .collect();
    if rest.starts_with('#') {
        return "comments start with `//`".into();
    }
    if word.starts_with(|c: char| c.is_ascii_digit()) && word.contains(char::is_alphabetic) {
        return format!(
            "a value with a prefix or unit is a string, such as \"10k\" or \"1ms\", not {word}"
        );
    }
    match kind {
        ParseErrorKind::ExpectedComma => "add a comma between the items".into(),
        ParseErrorKind::SingleQuotedStringsNotAllowed => "strings take double quotes".into(),
        ParseErrorKind::UnexpectedWord | ParseErrorKind::ExpectedStringObjectProperty => {
            "keys and strings take double quotes".into()
        }
        _ => "fix the JSON here; strings take double quotes".into(),
    }
}

fn convert(file: FileId, value: &Ast<'_>) -> Node {
    let span = |range: jsonc_parser::common::Range| Span {
        file,
        start: range.start,
        end: range.end,
    };
    match value {
        Ast::NullKeyword(n) => Node {
            span: span(n.range),
            json: Json::Null,
        },
        Ast::BooleanLit(b) => Node {
            span: span(b.range),
            json: Json::Bool(b.value),
        },
        Ast::NumberLit(n) => Node {
            span: span(n.range),
            json: Json::Num(n.value.to_string()),
        },
        Ast::StringLit(s) => Node {
            span: span(s.range),
            json: Json::Str(s.value.to_string()),
        },
        Ast::Array(a) => Node {
            span: span(a.range),
            json: Json::Arr(a.elements.iter().map(|e| convert(file, e)).collect()),
        },
        Ast::Object(o) => Node {
            span: span(o.range),
            json: Json::Obj(
                o.properties
                    .iter()
                    .map(|p| {
                        let (key, key_range) = match &p.name {
                            ObjectPropName::String(s) => (s.value.to_string(), s.range),
                            ObjectPropName::Word(w) => (w.value.to_string(), w.range),
                        };
                        Prop {
                            key,
                            key_span: span(key_range),
                            value: convert(file, &p.value),
                        }
                    })
                    .collect(),
            ),
        },
    }
}

fn check_duplicate_keys(sources: &Sources, node: &Node, out: &mut Findings) {
    match &node.json {
        Json::Obj(props) => {
            for (i, prop) in props.iter().enumerate() {
                if let Some(first) = props[..i].iter().find(|p| p.key == prop.key) {
                    out.push(
                        Code::Duplicate,
                        prop.key_span,
                        format!("`{}` is listed twice", prop.key),
                        format!(
                            "the first `{}` is at {}; rename or remove one",
                            prop.key,
                            sources.location(first.key_span)
                        ),
                    );
                }
                check_duplicate_keys(sources, &prop.value, out);
            }
        }
        Json::Arr(items) => items
            .iter()
            .for_each(|item| check_duplicate_keys(sources, item, out)),
        _ => {}
    }
}

#[cfg(test)]
pub(crate) fn take_parses() -> Vec<String> {
    PARSES.with(|parses| std::mem::take(&mut *parses.borrow_mut()))
}

#[cfg(test)]
thread_local! {
    static PARSES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_parse(path: &str) {
    PARSES.with(|parses| parses.borrow_mut().push(path.to_string()));
}

impl Node {
    pub(crate) fn props(&self) -> Option<&[Prop]> {
        match &self.json {
            Json::Obj(props) => Some(props),
            _ => None,
        }
    }

    pub(crate) fn items(&self) -> Option<&[Node]> {
        match &self.json {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    pub(crate) fn str(&self) -> Option<&str> {
        match &self.json {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn number(&self) -> Option<f64> {
        match &self.json {
            Json::Num(raw) => raw.parse().ok(),
            _ => None,
        }
    }

    /// The number as written, before it is parsed.
    pub(crate) fn raw_number(&self) -> Option<&str> {
        match &self.json {
            Json::Num(raw) => Some(raw),
            _ => None,
        }
    }

    pub(crate) fn is_null(&self) -> bool {
        matches!(self.json, Json::Null)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Node> {
        self.props()?
            .iter()
            .find(|p| p.key == key)
            .map(|p| &p.value)
    }

    /// What this node is, for a message: "a string", "an object".
    pub(crate) fn what(&self) -> &'static str {
        match self.json {
            Json::Null => "null",
            Json::Bool(_) => "a boolean",
            Json::Num(_) => "a number",
            Json::Str(_) => "a string",
            Json::Arr(_) => "a list",
            Json::Obj(_) => "an object",
        }
    }

    pub(crate) fn to_value(&self) -> Value {
        match &self.json {
            Json::Null => Value::Null,
            Json::Bool(b) => Value::Bool(*b),
            Json::Num(raw) => {
                serde_json::from_str(raw).expect("parse refuses a number it cannot hold")
            }
            Json::Str(s) => Value::String(s.clone()),
            Json::Arr(items) => Value::Array(items.iter().map(Node::to_value).collect()),
            Json::Obj(props) => Value::Object(
                props
                    .iter()
                    .map(|p| (p.key.clone(), p.value.to_value()))
                    .collect(),
            ),
        }
    }

    /// The deepest node on the way to a JSON pointer, such as `/events/0/t_ps`: the node
    /// itself when it is there, else the last one there on its way.
    pub(crate) fn at_pointer(&self, pointer: &str) -> &Node {
        let mut node = self;
        for token in pointer.split('/').skip(1) {
            let next = match &node.json {
                Json::Obj(_) => node.get(token),
                Json::Arr(items) => token.parse::<usize>().ok().and_then(|i| items.get(i)),
                _ => None,
            };
            match next {
                Some(next) => node = next,
                None => break,
            }
        }
        node
    }
}
