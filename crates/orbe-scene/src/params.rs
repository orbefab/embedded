//! A param's value, as an instance or a default writes it, turned into what it sets in the
//! part's model.

use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::diagnostic::Code;
use crate::model::{ParamDecl, ParamKind};
use crate::name::did_you_mean;
use crate::project::hex;
use crate::syntax::Node;
use crate::units::{self, Unit};

/// A param value on its way to the part that sets it, with where it was written.
#[derive(Clone)]
pub(crate) struct Arg {
    pub(crate) node: Node,
    /// The param the file sets, such as `nano.firmware`, for messages.
    pub(crate) site: String,
    /// The folder of the file that wrote it, for a file param's path.
    pub(crate) dir: PathBuf,
    /// Set by an instance, not a declaration's default.
    pub(crate) explicit: bool,
}

pub(crate) type Converted = Result<Vec<(String, Value)>, (Code, String, String)>;

fn fail(message: impl Into<String>, hint: impl Into<String>) -> (Code, String, String) {
    (Code::Param, message.into(), hint.into())
}

/// The pointers a param's value sets in the model, and what goes at each.
pub(crate) fn convert(decl: &ParamDecl, arg: &Arg) -> Converted {
    let node = &arg.node;
    let pointer = decl.sets.clone().unwrap_or_default();
    match &decl.kind {
        ParamKind::Number {
            unit,
            min_exclusive,
        } => {
            if node.is_null() {
                return if decl.default.as_ref().is_some_and(Node::is_null) {
                    Ok(vec![(pointer, Value::Null)])
                } else {
                    Err(fail(
                        "null is not a value",
                        format!("write a number, such as {}", unit.example()),
                    ))
                };
            }
            number(*unit, *min_exclusive, node).map(|v| vec![(pointer, v)])
        }
        ParamKind::Choice(choices) => {
            let names: Vec<&str> = choices.iter().map(|(n, _)| n.as_str()).collect();
            let chosen = node
                .str()
                .and_then(|s| choices.iter().find(|(n, _)| n == s));
            match chosen {
                Some((_, sets)) => Ok(sets
                    .iter()
                    .map(|(p, v)| (p.clone(), v.to_value()))
                    .collect()),
                None => {
                    let written = node
                        .str()
                        .map_or_else(|| node.what().to_string(), |s| format!("`{s}`"));
                    let hint = node
                        .str()
                        .map(|s| did_you_mean(s, names.iter().copied(), ""))
                        .unwrap_or_default();
                    Err(fail(
                        format!("{written} is not a choice of `{}`", decl.name),
                        format!("{hint}the choices: {}", names.join(", ")),
                    ))
                }
            }
        }
        ParamKind::File { text } => {
            let Some(rel) = node.str() else {
                return Err(fail(
                    format!("a file param takes a path, found {}", node.what()),
                    "write a path relative to the file that sets it",
                ));
            };
            match fs::read(arg.dir.join(rel)) {
                Ok(bytes) if *text => match String::from_utf8(bytes) {
                    Ok(text) => Ok(vec![(pointer, Value::String(text))]),
                    Err(_) => Err(fail(
                        format!("`{rel}` is not text"),
                        "save it as UTF-8 text",
                    )),
                },
                Ok(bytes) => Ok(vec![(
                    pointer,
                    json!({ "sha256": hex(&Sha256::digest(&bytes)), "bytes": bytes.len() }),
                )]),
                Err(_) => Err((
                    Code::FirmwareMissing,
                    format!("no file at `{rel}`"),
                    "build it, or fix the path; it is relative to the file that sets it".into(),
                )),
            }
        }
        ParamKind::Through => Ok(Vec::new()),
    }
}

fn number(
    unit: Unit,
    min_exclusive: Option<f64>,
    node: &Node,
) -> Result<Value, (Code, String, String)> {
    let example = || {
        format!(
            "write a number and an optional prefix, such as {}",
            unit.example()
        )
    };
    if node.props().is_some() {
        return value_object(unit, min_exclusive, node);
    }
    let text = match (node.str(), node.number()) {
        (Some(text), _) => text.to_string(),
        (None, Some(_)) => node.raw_number().unwrap_or("").to_string(),
        _ => {
            return Err(fail(
                format!(
                    "expected a number in {}, such as {}",
                    unit.as_str(),
                    unit.example()
                ),
                example(),
            ));
        }
    };
    let shift = if unit == Unit::Second { 12 } else { 0 };
    let value = units::parse(&text, unit, shift).map_err(|reason| {
        fail(
            format!(
                "{reason}; expected a number in {}, such as {}",
                unit.as_str(),
                unit.example()
            ),
            example(),
        )
    })?;
    if !value.is_finite() {
        return Err(fail(
            format!("{text} is not a finite number"),
            format!("write a finite number, such as {}", unit.example()),
        ));
    }
    bounded(unit, min_exclusive, &text, value)
}

fn value_object(
    unit: Unit,
    min_exclusive: Option<f64>,
    node: &Node,
) -> Result<Value, (Code, String, String)> {
    let shape = "a value object is { \"value\", \"unit\", \"source\" }";
    let shape_hint = format!(
        "write value, unit and source, or a string such as {}",
        unit.example()
    );
    let Some(props) = node.props() else {
        return Err(fail(shape, shape_hint));
    };
    let keys_ok = props.len() == 3
        && props
            .iter()
            .all(|p| matches!(p.key.as_str(), "value" | "unit" | "source"));
    if !keys_ok {
        return Err(fail(shape, shape_hint));
    }
    let value_node = node.get("value").expect("value is one of the three keys");
    let Some(unit_text) = node.get("unit").and_then(Node::str) else {
        return Err(fail(shape, shape_hint));
    };
    let source = node.get("source").expect("source is one of the three keys");
    if !source_ok(source) {
        let shown = source
            .raw_number()
            .or_else(|| source.str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| source.what().to_string());
        return Err(fail(
            format!("source `{shown}` is not a source"),
            "source is \"user\", { \"closed_form\": { \"note\" } }, or { \"datasheet\": { \"doc\", \"at\", \"basis\" } }",
        ));
    }
    let Some(number) = value_node.number() else {
        return Err(fail(shape, shape_hint));
    };
    let shown = value_node.raw_number().unwrap_or("");
    if !number.is_finite() {
        return Err(fail(
            format!("{shown} is not a finite number"),
            format!("write a finite number, such as {}", unit.example()),
        ));
    }
    if unit == Unit::Second {
        if unit_text != "ps" {
            return Err(fail(
                format!(
                    "the unit is `{unit_text}`, and a duration in the netlist is an integer number of picoseconds"
                ),
                "write \"1ms\", or a value object with \"unit\": \"ps\"",
            ));
        }
        below_min(min_exclusive, unit, shown, number / 1e12)?;
        return Ok(json!({ "value": number, "unit": "ps", "source": source.to_value() }));
    }
    if unit_text != unit.as_str() {
        return Err(fail(
            format!(
                "the unit is `{unit_text}`, and this param is in {}",
                unit.as_str()
            ),
            format!(
                "use the unit {}, or a string such as {}",
                unit.as_str(),
                unit.example()
            ),
        ));
    }
    below_min(min_exclusive, unit, shown, number)?;
    Ok(json!({ "value": number, "unit": unit_text, "source": source.to_value() }))
}

fn bounded(
    unit: Unit,
    min_exclusive: Option<f64>,
    text: &str,
    value: f64,
) -> Result<Value, (Code, String, String)> {
    if unit == Unit::Second && value < 0.0 {
        return Err(fail(
            format!("{text} is negative"),
            "write a positive duration, such as \"1ms\"",
        ));
    }
    let (si, unit_text) = if unit == Unit::Second {
        (value / 1e12, "ps")
    } else {
        (value, unit.as_str())
    };
    below_min(min_exclusive, unit, text, si)?;
    Ok(json!({ "value": value, "unit": unit_text, "source": "user" }))
}

fn below_min(
    min_exclusive: Option<f64>,
    unit: Unit,
    text: &str,
    si: f64,
) -> Result<(), (Code, String, String)> {
    if let Some(min) = min_exclusive
        && si <= min
    {
        return Err(fail(
            format!(
                "{text} is out of range; it must be more than {min} {}",
                unit.as_str()
            ),
            format!("write a value greater than {min} {}", unit.as_str()),
        ));
    }
    Ok(())
}

fn source_ok(node: &Node) -> bool {
    if let Some(text) = node.str() {
        return text == "user";
    }
    let Some(props) = node.props() else {
        return false;
    };
    if props.len() != 1 {
        return false;
    }
    match props[0].key.as_str() {
        "closed_form" => props[0].value.props().is_some_and(|fields| {
            fields.len() == 1 && fields[0].key == "note" && fields[0].value.str().is_some()
        }),
        "datasheet" => props[0].value.props().is_some_and(|fields| {
            fields.len() == 3
                && fields
                    .iter()
                    .any(|f| f.key == "doc" && f.value.str().is_some())
                && fields
                    .iter()
                    .any(|f| f.key == "at" && f.value.str().is_some())
                && fields.iter().any(|f| {
                    f.key == "basis"
                        && matches!(
                            f.value.str(),
                            Some("typical" | "min" | "max" | "nominal" | "absolute_max")
                        )
                })
        }),
        _ => false,
    }
}

/// The tokens of a pointer into a model, such as `/bounce/window`.
fn tokens(pointer: &str) -> Result<Vec<&str>, String> {
    pointer
        .strip_prefix('/')
        .map(|rest| rest.split('/').collect())
        .ok_or_else(|| format!("`{pointer}` is not a pointer into `model`; it starts with /"))
}

/// Where `pointer` stops in `model`: `Ok(None)` when every object on its way is there, else
/// the first one that is null or missing, such as `/bounce`. That object is off.
pub(crate) fn off_at(model: &Value, pointer: &str) -> Result<Option<String>, String> {
    let tokens = tokens(pointer)?;
    let mut node = model;
    for (i, token) in tokens[..tokens.len() - 1].iter().enumerate() {
        let object = node.as_object().ok_or_else(|| {
            format!(
                "`{pointer}` passes through `{}`, which is not an object",
                tokens[..i].join(".")
            )
        })?;
        match object.get(*token) {
            None | Some(Value::Null) => return Ok(Some(format!("/{}", tokens[..=i].join("/")))),
            Some(next) => node = next,
        }
    }
    if node.is_object() {
        Ok(None)
    } else {
        Err(format!(
            "`{pointer}` passes through `{}`, which is not an object",
            tokens[..tokens.len() - 1].join(".")
        ))
    }
}

/// Puts `value` at a JSON pointer, merging two objects. An object on the way that is off is
/// made only when `create` is set. Returns whether the value was written.
pub(crate) fn set(target: &mut Value, pointer: &str, value: Value, create: bool) -> bool {
    let Ok(tokens) = tokens(pointer) else {
        return false;
    };
    let (last, through) = tokens.split_last().expect("split yields one token");
    let mut node = target;
    for token in through {
        let Some(object) = node.as_object_mut() else {
            return false;
        };
        if object.get(*token).is_none_or(Value::is_null) {
            if !create {
                return false;
            }
            object.insert((*token).to_string(), Value::Object(Map::new()));
        }
        node = object.get_mut(*token).expect("inserted above");
    }
    let Some(object) = node.as_object_mut() else {
        return false;
    };
    match (object.get_mut(*last), value) {
        (Some(Value::Object(existing)), Value::Object(new)) => existing.extend(new),
        (_, value) => {
            object.insert((*last).to_string(), value);
        }
    }
    true
}
