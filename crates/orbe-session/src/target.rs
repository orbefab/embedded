//! Refs as written, `@btn1` or `@nano.D13`, and what they name in the reader's map.

use std::fmt;

use orbe_scene::{MapPart, RefMap};
use sim_core::name::Quantity;

use crate::reply::{Code, Error};

/// `@` and a dotted scene name, such as `@nano.D13` or `@led1.i`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref(Vec<String>);

impl Ref {
    pub fn parse(text: &str) -> Result<Ref, String> {
        let name = text.strip_prefix('@').ok_or_else(|| {
            format!("`{text}` is not a ref; a ref is `@` and a name, such as `@btn1`")
        })?;
        let segs: Vec<String> = name.split('.').map(str::to_string).collect();
        if segs.iter().any(String::is_empty) {
            return Err(format!("`{text}` has an empty name between dots"));
        }
        Ok(Ref(segs))
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.0.join("."))
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

const QUANTITIES: [Quantity; 8] = [
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
/// quantity.
pub fn resolve(map: &RefMap, r: &Ref) -> Result<Target, Error> {
    let first = &r.0[0];
    if !map.top.contains(first) {
        return Err(unknown(
            r,
            format!("no part named `{first}`"),
            format!("parts: {}", map.top.join(", ")),
        ));
    }
    let mut path = first.clone();
    for (i, seg) in r.0.iter().enumerate().skip(1) {
        let part = &map.parts[&path];
        let last = i + 1 == r.0.len();
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
            let engine = one_engine(&path, part)?;
            return Ok(Target::Quantity {
                path,
                engine,
                quantity,
            });
        }
        return Err(unknown(
            r,
            format!("`{path}` has no pin or part named `{seg}`"),
            names(part),
        ));
    }
    Ok(Target::Part(path))
}

/// The flat name of the one engine part a part's own model writes.
pub fn one_engine(path: &str, part: &MapPart) -> Result<String, Error> {
    match part.engine.as_slice() {
        [one] => Ok(one.name.clone()),
        [] => Err(Error::new(
            Code::NotSettable,
            format!("`{path}` has no engine part of its own"),
            if part.children.is_empty() {
                "it is not simulated".to_string()
            } else {
                format!("name one of its parts: {}", part.children.join(", "))
            },
        )),
        many => Err(Error::new(
            Code::NotSettable,
            format!("`{path}` is {} engine parts", many.len()),
            "a quantity or an input names one engine part".to_string(),
        )),
    }
}

fn unknown(r: &Ref, message: String, hint: String) -> Error {
    Error::new(Code::UnknownRef, format!("{r}: {message}"), hint)
}

fn names(part: &MapPart) -> String {
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
    if part.engine.len() == 1 {
        found.push("quantities: i, v, p".to_string());
    }
    found.join("; ")
}
