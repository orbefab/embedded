//! The catalog ids and param links a part or scene file names, read with the same parser
//! [`crate::syntax`] uses. A project that has not nested its parts yet can still see those ids.

use std::path::Path;

use crate::diagnostic::Findings;
use crate::model::is_id;
use crate::source::Sources;
use crate::syntax::{self, Node};

/// Catalog ids named as `type` under the top-level `parts` object, in order, once each.
/// A `./` path, a `../` path and a registry id are not catalog ids. A file that does not
/// parse names none.
pub fn catalog_part_ids(text: &str) -> Vec<String> {
    let Some(root) = tree(text) else {
        return Vec::new();
    };
    let Some(parts) = root.get("parts").and_then(Node::props) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for spec in parts {
        let Some(ty) = spec.value.get("type").and_then(Node::str) else {
            continue;
        };
        if is_id(ty) && !ids.iter().any(|id| id == ty) {
            ids.push(ty.to_string());
        }
    }
    ids
}

/// The 1-based line and column of the `params` key `name`, including its quotes.
/// `None` when the file does not parse or the param is not there.
pub fn param_key_at(text: &str, name: &str) -> Option<(usize, usize)> {
    let root = tree(text)?;
    let params = root.get("params")?;
    let prop = params.props()?.iter().find(|prop| prop.key == name)?;
    let (line, column) = line_col(text, prop.key_span.start);
    Some((line, column))
}

/// `params.<name>` gives the value a unit, choices or a file, so this part declares it.
pub fn declares_param(text: &str, name: &str) -> bool {
    let Some(root) = tree(text) else {
        return false;
    };
    let Some(decl) = root.get("params").and_then(|params| params.get(name)) else {
        return false;
    };
    decl.get("file").is_some() || decl.get("unit").is_some() || decl.get("choices").is_some()
}

/// A param this part does not pass on. A missing `params.<name>` is one this part owns.
pub fn owns_param(text: &str, name: &str) -> bool {
    let Some(root) = tree(text) else {
        return true;
    };
    let Some(decl) = root.get("params").and_then(|params| params.get(name)) else {
        return true;
    };
    decl.get("file").is_some() || decl.get("unit").is_some() || decl.get("choices").is_some()
}

/// The child catalog id and the param name a value of `name` is passed to.
pub fn passed_to(text: &str, name: &str) -> Option<(String, String)> {
    let root = tree(text)?;
    let parts = root.get("parts")?.props()?;
    for spec in parts {
        let ty = spec.value.get("type").and_then(Node::str)?;
        if !is_id(ty) {
            continue;
        }
        let props = spec.value.props()?;
        for prop in props {
            if prop.value.get("param").and_then(Node::str) == Some(name) {
                return Some((ty.to_string(), prop.key.clone()));
            }
        }
    }
    None
}

/// The part file names a ground pin.
pub fn names_ground(text: &str) -> bool {
    let Some(root) = tree(text) else {
        return false;
    };
    root.get("ground").and_then(Node::str).is_some()
}

fn tree(text: &str) -> Option<Node> {
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new("part.jsonc"), text.to_string());
    let mut findings = Findings::default();
    syntax::parse(&sources, file, &mut findings)
}

fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[line_start..].chars().count() + 1)
}
