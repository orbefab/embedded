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
            number(decl, *unit, *min_exclusive, node).map(|v| vec![(pointer, v)])
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
    decl: &ParamDecl,
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
        return value_object(decl, unit, min_exclusive, node);
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
    bounded(decl, unit, min_exclusive, &text, value)
}

fn value_object(
    decl: &ParamDecl,
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
        let Ok(whole) = shown.parse::<u64>() else {
            return Err(fail(
                format!("{shown} is not a whole number of picoseconds"),
                "write an integer number of ps, or a string such as \"1ms\"",
            ));
        };
        below_min(min_exclusive, unit, shown, whole as f64 / 1e12)?;
        return Ok(json!({ "value": whole, "unit": "ps", "source": source.to_value() }));
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
    if decl
        .sets
        .as_deref()
        .is_some_and(|p| p.ends_with("max_bounces"))
    {
        let Ok(whole) = shown.parse::<u64>() else {
            return Err(fail(
                format!("{shown} is not a whole number"),
                "max_bounces counts contact edges; write a whole number such as 2",
            ));
        };
        if whole > 255 {
            return Err(fail(
                format!("{shown} is outside 0 to 255"),
                "max_bounces counts contact edges; write a whole number such as 2",
            ));
        }
        return Ok(json!({ "value": whole, "unit": "1", "source": source.to_value() }));
    }
    Ok(json!({ "value": number, "unit": unit_text, "source": source.to_value() }))
}

fn bounded(
    decl: &ParamDecl,
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
    let si = if unit == Unit::Second {
        value / 1e12
    } else {
        value
    };
    below_min(min_exclusive, unit, text, si)?;
    if unit == Unit::Second {
        if value > (1u64 << 63) as f64 {
            return Err(fail(
                format!("{text} does not fit in picoseconds"),
                "write a shorter duration, such as \"1ms\"",
            ));
        }
        return Ok(json!({ "value": value.round() as u64, "unit": "ps", "source": "user" }));
    }
    if decl
        .sets
        .as_deref()
        .is_some_and(|p| p.ends_with("max_bounces"))
    {
        if value < 0.0 || value.fract() != 0.0 {
            return Err(fail(
                format!("{text} is not a whole number"),
                "max_bounces counts contact edges; write a whole number such as 2",
            ));
        }
        if value > 255.0 {
            return Err(fail(
                format!("{text} is outside 0 to 255"),
                "max_bounces counts contact edges; write a whole number such as 2",
            ));
        }
        return Ok(json!({ "value": value as u64, "unit": "1", "source": "user" }));
    }
    Ok(match unit {
        Unit::One if value.fract() == 0.0 => {
            json!({ "value": value as i64, "unit": "1", "source": "user" })
        }
        _ => json!({ "value": value, "unit": unit.as_str(), "source": "user" }),
    })
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
