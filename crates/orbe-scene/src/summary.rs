//! What each built-in catalog part offers a scene: its pins, its params and what can be read
//! or pressed. The skill lists parts from this, so it always matches the catalog.

use crate::catalog;
use crate::model::passed;
use crate::syntax::Node;

pub struct PartSummary {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub pins: Vec<String>,
    pub params: Vec<ParamSummary>,
    /// The `on` reading: the quantity it reads, and the amount above which the part is on.
    pub on: Option<(String, String)>,
    /// Each `model.ratings` entry with a value, as its name and amount: `("current", "30 mA")`.
    pub ratings: Vec<(String, String)>,
    /// `controls.press`, so `press` and `release` work on it.
    pub pressable: bool,
    /// The catalog parts whose `uses` name this one.
    pub used_by: Vec<String>,
}

pub struct ParamSummary {
    pub name: String,
    pub takes: Takes,
    pub default: ParamDefault,
}

pub enum Takes {
    Number {
        unit: String,
        above: Option<String>,
        below: Option<String>,
    },
    Choice(Vec<String>),
    File {
        ext: Option<String>,
    },
}

pub enum ParamDefault {
    Required,
    /// `"default": null`: the model's own value, or off, until the scene sets one.
    Unset,
    Value(String),
}

/// Every part of the built-in catalog, by id.
pub fn catalog_parts() -> Vec<PartSummary> {
    let trees: Vec<(String, Node)> = catalog::ids()
        .map(|id| {
            let tree = catalog::find(&id)
                .expect("the built-in catalog parses")
                .tree;
            (id, tree)
        })
        .collect();
    let tree_of = |id: &str| trees.iter().find(|(i, _)| i == id).map(|(_, t)| t);
    let text = |tree: &Node, key: &str| tree.get(key).and_then(Node::str).map(str::to_string);
    trees
        .iter()
        .map(|(id, tree)| PartSummary {
            id: id.clone(),
            title: text(tree, "title").unwrap_or_default(),
            summary: text(tree, "summary"),
            pins: keys(tree.get("pins")),
            params: tree
                .get("params")
                .and_then(Node::props)
                .unwrap_or_default()
                .iter()
                .filter_map(|p| param(tree, &p.key, &tree_of))
                .collect(),
            on: tree
                .get("readings")
                .and_then(|r| r.get("on"))
                .and_then(|on| {
                    let of = on.get("of")?.str()?.to_string();
                    Some((of, amount(on.get("above")?)?))
                }),
            ratings: tree
                .get("model")
                .and_then(|m| m.get("ratings"))
                .and_then(Node::props)
                .unwrap_or_default()
                .iter()
                .filter_map(|r| Some((r.key.clone(), amount(&r.value)?)))
                .collect(),
            pressable: tree.get("controls").and_then(|c| c.get("press")).is_some(),
            used_by: trees
                .iter()
                .filter(|(_, other)| keys(other.get("uses")).contains(id))
                .map(|(other, _)| other.clone())
                .collect(),
        })
        .collect()
}

fn keys(node: Option<&Node>) -> Vec<String> {
    node.and_then(Node::props)
        .unwrap_or_default()
        .iter()
        .map(|p| p.key.clone())
        .collect()
}

/// A param as the scene sees it. A param with no `sets` and no `choices` passes its value to
/// a child, so it takes what the child's param takes.
fn param<'t>(
    tree: &'t Node,
    name: &str,
    tree_of: &dyn Fn(&str) -> Option<&'t Node>,
) -> Option<ParamSummary> {
    let decl = tree.get("params")?.get(name)?;
    let unit = decl.get("unit").and_then(Node::str);
    let required = decl
        .get("required")
        .is_some_and(|r| r.to_value().as_bool() != Some(false));
    let default = match decl.get("default") {
        _ if required => Some(ParamDefault::Required),
        Some(node) if node.is_null() => Some(ParamDefault::Unset),
        Some(node) => Some(ParamDefault::Value(written(node, unit))),
        None => None,
    };
    let takes = if let Some(choices) = decl.get("choices") {
        Takes::Choice(keys(Some(choices)))
    } else if decl.get("file").is_some() {
        Takes::File {
            ext: decl.get("file").and_then(Node::str).map(str::to_string),
        }
    } else if let Some(unit) = unit {
        Takes::Number {
            unit: unit.to_string(),
            above: decl.get("min_exclusive").map(|n| written(n, None)),
            below: decl.get("max_exclusive").map(|n| written(n, None)),
        }
    } else {
        let child = tree.get("parts")?.props()?.iter().find_map(|inst| {
            let key = inst
                .value
                .props()?
                .iter()
                .find(|p| passed(&p.value) == Some(name))?;
            Some((inst.value.get("type")?.str()?, key.key.clone()))
        })?;
        let inner = param(tree_of(child.0)?, &child.1, tree_of)?;
        return Some(ParamSummary {
            name: name.to_string(),
            takes: inner.takes,
            default: default.unwrap_or(inner.default),
        });
    };
    Some(ParamSummary {
        name: name.to_string(),
        takes,
        default: default.unwrap_or(ParamDefault::Required),
    })
}

/// A value as a scene would write it: a number with its unit, or a string as it is.
fn written(node: &Node, unit: Option<&str>) -> String {
    match (node.raw_number(), node.str()) {
        (Some(raw), _) => match unit {
            Some("1") | None => raw.to_string(),
            Some(unit) => format!("{raw} {unit}"),
        },
        (_, Some(text)) => format!("\"{text}\""),
        _ => node.to_value().to_string(),
    }
}

/// A `{ "value": 0.001, "unit": "A" }` amount, with an SI prefix: `1 mA`.
fn amount(node: &Node) -> Option<String> {
    let value = node.get("value")?.number()?;
    let unit = node.get("unit")?.str()?;
    let (scale, prefix) = [
        (1e6, "M"),
        (1e3, "k"),
        (1.0, ""),
        (1e-3, "m"),
        (1e-6, "u"),
        (1e-9, "n"),
    ]
    .into_iter()
    .find(|(scale, _)| value.abs() >= *scale)
    .unwrap_or((1e-12, "p"));
    let scaled = format!("{:.6}", value / scale);
    let scaled = scaled.trim_end_matches('0').trim_end_matches('.');
    Some(format!("{scaled} {prefix}{unit}"))
}
