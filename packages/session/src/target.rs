//! Refs as written, `@btn1` or `@nano.D13`, and what they name in the reader's map.

use std::fmt;

use orbe_engine::name::Quantity;
use orbe_scene::{MapPart, RefMap};
use serde::{Deserialize, Serialize};

use crate::reply::{Code, Error};

/// `@` and a dotted scene name, such as `@nano.D13` or `@led1.i`: a part and the names below
/// it, none empty. Its wire form is that text, read by `parse`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Ref {
    part: String,
    below: Vec<String>,
}

impl Ref {
    pub fn parse(text: &str) -> Result<Ref, String> {
        let name = text.strip_prefix('@').ok_or_else(|| {
            format!("`{text}` is not a ref; a ref is `@` and a name, such as `@btn1`")
        })?;
        let mut segs = name.split('.').map(str::to_string);
        let part = segs.next().unwrap_or_default();
        let below: Vec<String> = segs.collect();
        if part.is_empty() || below.iter().any(String::is_empty) {
            return Err(format!("`{text}` has an empty name between dots"));
        }
        Ok(Ref { part, below })
    }
}

impl TryFrom<String> for Ref {
    type Error = String;

    fn try_from(text: String) -> Result<Ref, String> {
        Ref::parse(&text)
    }
}

impl From<Ref> for String {
    fn from(r: Ref) -> String {
        r.to_string()
    }
}

#[cfg(test)]
impl schemars::JsonSchema for Ref {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Ref".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "`@` and a dotted scene name, such as `@btn1` or `@nano.D13`.",
            "type": "string",
            "pattern": "^@",
        })
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.part)?;
        self.below.iter().try_for_each(|seg| write!(f, ".{seg}"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A part, by its dotted path.
    Part(String),
    /// A pin, with the engine net it sits on. `None` when the pin is not simulated.
    Pin {
        path: String,
        pin: String,
        net: Option<String>,
    },
    /// A quantity of the part's one engine part.
    Quantity {
        path: String,
        engine: String,
        quantity: Quantity,
    },
}

pub(crate) const QUANTITIES: [Quantity; 8] = [
    Quantity::I,
    Quantity::V,
    Quantity::P,
    Quantity::Ib,
    Quantity::Ic,
    Quantity::Id,
    Quantity::Iin,
    Quantity::Iout,
];

/// Walks one instance per segment from the scene. The last segment may also be a pin or a
/// quantity. `reports` lists the quantities the engine reports for an engine part.
pub fn resolve(
    map: &RefMap,
    r: &Ref,
    reports: &dyn Fn(&str) -> Vec<Quantity>,
) -> Result<Target, Error> {
    let first = &r.part;
    if !map.top.contains(first) {
        return Err(unknown(
            r,
            format!("no part named `{first}`"),
            format!("parts: {}", map.top.join(", ")),
        ));
    }
    let mut path = first.clone();
    for (i, seg) in r.below.iter().enumerate() {
        let part = &map.parts[&path];
        let last = i + 1 == r.below.len();
        if part.children.contains(seg) {
            path = format!("{path}.{seg}");
            continue;
        }
        if last && let Some(net) = part.pins.get(seg) {
            return Ok(Target::Pin {
                path,
                pin: seg.clone(),
                net: net.clone(),
            });
        }
        if last && let Some(&quantity) = QUANTITIES.iter().find(|q| q.as_str() == seg) {
            let engine = one_engine(&path, part, Code::NoReading)?;
            let reported = reports(&engine);
            if !reported.contains(&quantity) {
                return Err(Error::new(
                    Code::NoReading,
                    format!("the engine reports no `{path}.{seg}`"),
                    reports_hint(&path, &reported),
                ));
            }
            return Ok(Target::Quantity {
                path,
                engine,
                quantity,
            });
        }
        let reported = one_engine(&path, part, Code::NoReading)
            .map(|engine| reports(&engine))
            .unwrap_or_default();
        return Err(unknown(
            r,
            format!("`{path}` has no pin or part named `{seg}`"),
            names(part, &reported),
        ));
    }
    Ok(Target::Part(path))
}

/// The flat name of the one engine part a part's own model writes. `code` is the caller's:
/// `no_reading` for a read, `not_settable` for an input.
pub fn one_engine(path: &str, part: &MapPart, code: Code) -> Result<String, Error> {
    match part.engine.as_slice() {
        [one] => Ok(one.name.clone()),
        [] => Err(Error::new(
            code,
            format!("`{path}` has no engine part of its own"),
            if part.children.is_empty() {
                "it is not simulated".to_string()
            } else {
                format!("name one of its parts: {}", part.children.join(", "))
            },
        )),
        many => Err(Error::new(
            code,
            format!("`{path}` is {} engine parts", many.len()),
            format!(
                "a quantity or an input names one engine part; read one of its {}",
                names(part, &[])
            ),
        )),
    }
}

pub(crate) fn reports_hint(path: &str, reported: &[Quantity]) -> String {
    let names: Vec<&str> = reported.iter().map(|q| q.as_str()).collect();
    match names.as_slice() {
        [] => format!("`{path}` reports no quantity; read one of its pins"),
        [one] => format!("`{path}` reports {one}"),
        [rest @ .., last] => format!("`{path}` reports {} and {last}", rest.join(", ")),
    }
}

fn unknown(r: &Ref, message: String, hint: String) -> Error {
    Error::new(Code::UnknownRef, format!("{r}: {message}"), hint)
}

/// What a ref can name inside `part`: its pins, its parts and the quantities it reports.
pub(crate) fn names(part: &MapPart, reported: &[Quantity]) -> String {
    let mut found = Vec::new();
    if !part.pins.is_empty() {
        found.push(format!(
            "pins: {}",
            part.pins.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if !part.children.is_empty() {
        found.push(format!("parts: {}", part.children.join(", ")));
    }
    if !reported.is_empty() {
        let names: Vec<&str> = reported.iter().map(|q| q.as_str()).collect();
        found.push(format!("quantities: {}", names.join(", ")));
    }
    found.join("; ")
}
