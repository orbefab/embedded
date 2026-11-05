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

pub(crate) type Converted = Result<Vec<(String, Value)>, (Code, String, Option<String>)>;

/// The pointers a param's value sets in the model, and what goes at each.
pub(crate) fn convert(decl: &ParamDecl, arg: &Arg) -> Converted {
    let node = &arg.node;
    let pointer = decl.sets.clone().unwrap_or_default();
    match &decl.kind {
        ParamKind::Number {
            unit,
            min_exclusive,
        } => number(*unit, *min_exclusive, node).map(|v| vec![(pointer, v)]),
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
                    Err((
                        Code::Param,
                        format!("{written} is not a choice of `{}`", decl.name),
                        Some(format!("{hint}the choices: {}", names.join(", "))),
                    ))
                }
            }
        }
        ParamKind::File { text } => {
            let Some(rel) = node.str() else {
                return Err((
                    Code::Param,
                    format!("a file param takes a path, found {}", node.what()),
                    None,
                ));
            };
            match fs::read(arg.dir.join(rel)) {
                Ok(bytes) if *text => match String::from_utf8(bytes) {
                    Ok(text) => Ok(vec![(pointer, Value::String(text))]),
                    Err(_) => Err((Code::Param, format!("`{rel}` is not text"), None)),
                },
                Ok(bytes) => Ok(vec![(
                    pointer,
                    json!({ "sha256": hex(&Sha256::digest(&bytes)), "bytes": bytes.len() }),
                )]),
                Err(_) => Err((
                    Code::FirmwareMissing,
                    format!("no file at `{rel}`"),
                    Some(
                        "build it, or fix the path; it is relative to the file that sets it".into(),
                    ),
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
) -> Result<Value, (Code, String, Option<String>)> {
    let expected = || {
        (
            Code::Param,
            format!(
                "expected a number in {}, such as {}",
                unit.as_str(),
                unit.example()
            ),
            None,
        )
    };
    if node.is_null() {
        return Ok(Value::Null);
    }
    if node.props().is_some() {
        let written = node.get("unit").and_then(Node::str);
        let fits =
            node.get("value").and_then(Node::number).is_some() && node.get("source").is_some();
        return match written {
            Some(u) if fits && (u == unit.as_str() || (unit == Unit::Second && u == "ps")) => {
                Ok(node.to_value())
            }
            Some(u) if fits => Err((
                Code::Param,
                format!("the unit is `{u}`, and this param is in {}", unit.as_str()),
                None,
            )),
            _ => Err((
                Code::Param,
                "a value object is { \"value\", \"unit\", \"source\" }".into(),
                None,
            )),
        };
    }
    let text = match (node.str(), node.number()) {
        (Some(text), _) => text.to_string(),
        (None, Some(_)) => node.to_value().to_string(),
        _ => return Err(expected()),
    };
    let shift = if unit == Unit::Second { 12 } else { 0 };
    let value = units::parse(&text, unit, shift).map_err(|reason| {
        (
            Code::Param,
            format!(
                "{reason}; expected a number in {}, such as {}",
                unit.as_str(),
                unit.example()
            ),
            None,
        )
    })?;
    let si = if unit == Unit::Second {
        value / 1e12
    } else {
        value
    };
    if let Some(min) = min_exclusive
        && si <= min
    {
        return Err((
            Code::Param,
            format!(
                "{text} is out of range; it must be more than {min} {}",
                unit.as_str()
            ),
            None,
        ));
    }
    Ok(match unit {
        Unit::Second => json!({ "value": value.round() as u64, "unit": "ps", "source": "user" }),
        Unit::One if value.fract() == 0.0 => {
            json!({ "value": value as i64, "unit": "1", "source": "user" })
        }
        _ => json!({ "value": value, "unit": unit.as_str(), "source": "user" }),
    })
}

/// Puts `value` at a JSON pointer, merging two objects. A missing or null object on the way
/// is made only when `create` is set, and otherwise nothing is written.
pub(crate) fn set(
    target: &mut Value,
    pointer: &str,
    value: Value,
    create: bool,
) -> Result<bool, String> {
    let tokens: Vec<&str> = pointer
        .strip_prefix('/')
        .ok_or_else(|| format!("`{pointer}` is not a pointer into the model"))?
        .split('/')
        .collect();
    let (last, through) = tokens.split_last().expect("split yields one token");
    let mut node = target;
    for token in through {
        let object = node
            .as_object_mut()
            .ok_or_else(|| format!("`{pointer}` passes through a value that is not an object"))?;
        if object.get(*token).is_none_or(Value::is_null) {
            if !create {
                return Ok(false);
            }
            object.insert((*token).to_string(), Value::Object(Map::new()));
        }
        node = object.get_mut(*token).expect("inserted above");
    }
    let object = node
        .as_object_mut()
        .ok_or_else(|| format!("`{pointer}` passes through a value that is not an object"))?;
    match (object.get_mut(*last), value) {
        (Some(Value::Object(existing)), Value::Object(new)) => existing.extend(new),
        (_, value) => {
            object.insert((*last).to_string(), value);
        }
    }
    Ok(true)
}
