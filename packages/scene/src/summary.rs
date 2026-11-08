//! One card per catalog part. `orbe part` and `orbe search` print it and do not parse
//! `part.jsonc`.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::catalog::{self, Original};
use crate::model::passed;
use crate::project::hex;
use crate::syntax::Node;

const PART_LICENSE: &str = "MIT OR Apache-2.0";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Card {
    pub name: String,
    pub title: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    pub real: Real,
    pub pins: Vec<Pin>,
    pub params: Vec<Param>,
    pub ratings: Vec<Rating>,
    pub not_simulated: Vec<NotSimulated>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<Vec<Doc>>,
    pub files: Vec<PartFile>,
    pub uses: BTreeMap<String, String>,
    pub used_by: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buy: Option<Vec<Buy>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<On>,
    #[serde(skip_serializing_if = "skip_false")]
    pub pressable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Real {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datasheet: Option<String>,
    #[serde(skip_serializing_if = "skip_false")]
    pub generic: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Pin {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leads: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refs: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Param {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub above: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub below: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<Choice>>,
    #[serde(skip_serializing_if = "skip_false")]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Option<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Choice {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Rating {
    pub name: String,
    pub value: f64,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NotSimulated {
    pub what: String,
    pub why: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pins: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Doc {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PartFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub license: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Buy {
    pub seller: String,
    pub url: String,
    pub checked: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct On {
    pub of: String,
    pub above: f64,
    pub unit: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Missing {
    pub parts: usize,
    pub description: usize,
    pub categories: usize,
    pub docs: usize,
    pub datasheet_url: usize,
    pub buy: usize,
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} catalog parts: {} lack a description, {} lack categories, {} lack a datasheet, {} lack a datasheet url, {} lack buy",
            self.parts, self.description, self.categories, self.docs, self.datasheet_url, self.buy
        )
    }
}

/// Every catalog part, by id.
pub fn catalog_cards() -> Vec<Card> {
    let loaded: Vec<(String, Original)> = catalog::ids()
        .map(|id| {
            let original = catalog::find(&id).expect("the built-in catalog parses");
            (id, original)
        })
        .collect();
    let mut used_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, original) in &loaded {
        for prop in original
            .tree
            .get("uses")
            .and_then(Node::props)
            .unwrap_or_default()
        {
            used_by
                .entry(prop.key.clone())
                .or_default()
                .push(id.clone());
        }
    }
    for names in used_by.values_mut() {
        names.sort();
    }
    let mut cards: Vec<Card> = loaded
        .iter()
        .map(|(id, original)| {
            let used = used_by.get(id).cloned().unwrap_or_default();
            build(id, original, used, &|want| {
                loaded
                    .iter()
                    .find(|(other, _)| other == want)
                    .map(|(_, found)| &found.tree)
            })
        })
        .collect();
    cards.sort_by(|a, b| a.name.cmp(&b.name));
    cards
}

pub fn card(name: &str) -> Option<Card> {
    catalog_cards().into_iter().find(|card| card.name == name)
}

/// Parts whose id, title, maker, number, summary or tags contain every word.
/// Exact id first, then parts whose title holds every word, then the rest. Ids break ties.
pub fn search(query: &str) -> Vec<Card> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let joined = words.join(" ");
    let mut found: Vec<(u8, Card)> = catalog_cards()
        .into_iter()
        .filter_map(|card| rank(&card, &words, &joined).map(|rank| (rank, card)))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.cmp(&b.1.name)));
    found.into_iter().map(|(_, card)| card).collect()
}

/// Catalog ids close to `name`, best first. The id itself is not one of them.
pub fn near_names(name: &str) -> Vec<String> {
    let want = name.to_lowercase();
    let mut scored: Vec<(f64, String)> = catalog::ids()
        .filter(|id| id != name)
        .map(|id| (strsim::jaro_winkler(&want, &id.to_lowercase()), id))
        .filter(|(score, _)| *score >= 0.8)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(8);
    scored.into_iter().map(|(_, id)| id).collect()
}

pub fn missing_fields(cards: &[Card]) -> Missing {
    let lack = |keep: fn(&Card) -> bool| cards.iter().filter(|card| keep(card)).count();
    Missing {
        parts: cards.len(),
        description: lack(|card| card.description.is_none()),
        categories: lack(|card| card.categories.is_none()),
        docs: lack(|card| card.docs.is_none()),
        datasheet_url: lack(|card| {
            card.docs
                .as_ref()
                .is_none_or(|docs| docs.iter().all(|doc| doc.url.is_none()))
        }),
        buy: lack(|card| card.buy.is_none()),
    }
}

pub fn si_amount(value: f64, unit: &str) -> String {
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
    format!("{scaled} {prefix}{unit}")
}

pub fn param_sentence(param: &Param) -> String {
    let takes = if let Some(choices) = &param.choices {
        let names: Vec<String> = choices.iter().map(|choice| choice.name.clone()).collect();
        format!("one of {}", quoted(&names))
    } else if let Some(ext) = &param.file {
        if ext.is_empty() {
            "the path of a file".to_string()
        } else {
            format!("the path of a `.{ext}` file")
        }
    } else {
        let mut text = match param.unit.as_deref() {
            Some("1") | None => "a number".to_string(),
            Some(unit) => format!("a value in {unit}"),
        };
        if let Some(above) = &param.above {
            text.push_str(&format!(" above {above}"));
        }
        if let Some(below) = &param.below {
            let joint = if param.above.is_some() { " and" } else { "" };
            text.push_str(&format!("{joint} below {below}"));
        }
        text
    };
    if param.required {
        format!("{takes}, required")
    } else if param.default.as_ref().is_some_and(Option::is_none) {
        format!("{takes}, not set by default")
    } else if let Some(Some(value)) = &param.default {
        format!("{takes}, {value} by default")
    } else {
        takes
    }
}

fn rank(card: &Card, words: &[String], joined: &str) -> Option<u8> {
    let haystack = [
        card.name.as_str(),
        card.title.as_str(),
        card.real.maker.as_deref().unwrap_or(""),
        card.real.number.as_deref().unwrap_or(""),
        card.description.as_deref().unwrap_or(""),
    ]
    .join("\n");
    let mut haystack = haystack.to_lowercase();
    if let Some(categories) = &card.categories {
        for category in categories {
            haystack.push('\n');
            haystack.push_str(&category.to_lowercase());
        }
    }
    if !words.iter().all(|word| haystack.contains(word)) {
        return None;
    }
    if card.name.eq_ignore_ascii_case(joined)
        || words
            .iter()
            .any(|word| card.name.eq_ignore_ascii_case(word))
    {
        return Some(0);
    }
    let title = words_of(&card.title);
    if words
        .iter()
        .all(|word| title.iter().any(|have| have == word))
    {
        return Some(1);
    }
    Some(2)
}

fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

fn build<'a>(
    id: &str,
    original: &'a Original,
    used_by: Vec<String>,
    tree_of: &dyn Fn(&str) -> Option<&'a Node>,
) -> Card {
    let tree = &original.tree;
    let license = tree
        .get("license")
        .and_then(Node::str)
        .unwrap_or(PART_LICENSE);
    Card {
        name: id.to_string(),
        title: text(tree, "title").unwrap_or_default(),
        version: original.version.clone(),
        description: text(tree, "summary"),
        categories: list(tree, "tags"),
        real: real(tree),
        pins: pins(tree),
        params: tree
            .get("params")
            .and_then(Node::props)
            .unwrap_or_default()
            .iter()
            .filter_map(|prop| param(tree, &prop.key, tree_of))
            .collect(),
        ratings: ratings(tree),
        not_simulated: gaps(tree),
        docs: docs(tree),
        files: files(license, original),
        uses: uses(tree),
        used_by,
        buy: None,
        on: on(tree),
        pressable: tree.get("controls").and_then(|c| c.get("press")).is_some(),
    }
}

fn text(tree: &Node, key: &str) -> Option<String> {
    let value = tree.get(key).and_then(Node::str)?;
    (!value.trim().is_empty()).then(|| value.to_string())
}

fn list(tree: &Node, key: &str) -> Option<Vec<String>> {
    let items: Vec<String> = tree
        .get(key)?
        .items()?
        .iter()
        .filter_map(Node::str)
        .filter(|item| !item.trim().is_empty())
        .map(str::to_string)
        .collect();
    (!items.is_empty()).then_some(items)
}

fn real(tree: &Node) -> Real {
    let real = tree.get("real");
    let text = |key: &str| {
        real.and_then(|node| node.get(key))
            .and_then(Node::str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
    };
    Real {
        maker: text("maker"),
        number: text("number"),
        datasheet: text("datasheet"),
        generic: real
            .and_then(|node| node.get("generic"))
            .and_then(|node| node.to_value().as_bool())
            .unwrap_or(false),
    }
}

fn pins(tree: &Node) -> Vec<Pin> {
    tree.get("pins")
        .and_then(Node::props)
        .unwrap_or_default()
        .iter()
        .map(|prop| {
            let value = &prop.value;
            if let Some(items) = value.items() {
                if !items.is_empty() && items.iter().all(|item| item.number().is_some()) {
                    return Pin {
                        name: prop.key.clone(),
                        leads: Some(
                            items
                                .iter()
                                .filter_map(|item| item.number().map(|n| n as u64))
                                .collect(),
                        ),
                        refs: None,
                    };
                }
                if !items.is_empty() && items.iter().all(|item| item.str().is_some()) {
                    return Pin {
                        name: prop.key.clone(),
                        leads: None,
                        refs: Some(
                            items
                                .iter()
                                .filter_map(Node::str)
                                .map(str::to_string)
                                .collect(),
                        ),
                    };
                }
            }
            if let Some(refer) = value.str() {
                return Pin {
                    name: prop.key.clone(),
                    leads: None,
                    refs: Some(vec![refer.to_string()]),
                };
            }
            Pin {
                name: prop.key.clone(),
                leads: None,
                refs: None,
            }
        })
        .collect()
}

enum OwnDefault {
    Required,
    Unset,
    Value(String),
    Inherit,
}

fn param<'t>(
    tree: &'t Node,
    name: &str,
    tree_of: &dyn Fn(&str) -> Option<&'t Node>,
) -> Option<Param> {
    let decl = tree.get("params")?.get(name)?;
    let unit = decl.get("unit").and_then(Node::str);
    let explicit = decl
        .get("required")
        .is_some_and(|node| node.to_value().as_bool() != Some(false));
    let own = if explicit {
        OwnDefault::Required
    } else {
        match decl.get("default") {
            Some(node) if node.is_null() => OwnDefault::Unset,
            Some(node) => OwnDefault::Value(written(node, unit)),
            None => OwnDefault::Inherit,
        }
    };
    if let Some(choices) = decl.get("choices").and_then(Node::props) {
        let mut param = Param {
            name: name.to_string(),
            unit: None,
            above: None,
            below: None,
            file: None,
            choices: Some(
                choices
                    .iter()
                    .map(|choice| Choice {
                        name: choice.key.clone(),
                        number: choice
                            .value
                            .get("real")
                            .and_then(|real| real.get("number"))
                            .and_then(Node::str)
                            .map(str::to_string),
                        tint: choice
                            .value
                            .get("tint")
                            .and_then(Node::str)
                            .map(str::to_string),
                    })
                    .collect(),
            ),
            required: false,
            default: None,
        };
        apply_default(&mut param, own);
        return Some(param);
    }
    if decl.get("file").is_some() {
        let mut param = Param {
            name: name.to_string(),
            unit: None,
            above: None,
            below: None,
            file: Some(
                decl.get("file")
                    .and_then(Node::str)
                    .unwrap_or("")
                    .to_string(),
            ),
            choices: None,
            required: false,
            default: None,
        };
        apply_default(&mut param, own);
        return Some(param);
    }
    if let Some(unit) = unit {
        let mut param = Param {
            name: name.to_string(),
            unit: Some(unit.to_string()),
            above: decl.get("min_exclusive").map(|node| written(node, None)),
            below: decl.get("max_exclusive").map(|node| written(node, None)),
            file: None,
            choices: None,
            required: false,
            default: None,
        };
        apply_default(&mut param, own);
        return Some(param);
    }
    let child = tree.get("parts")?.props()?.iter().find_map(|inst| {
        let key = inst
            .value
            .props()?
            .iter()
            .find(|prop| passed(&prop.value) == Some(name))?;
        Some((inst.value.get("type")?.str()?, key.key.clone()))
    })?;
    let mut inner = param(tree_of(child.0)?, &child.1, tree_of)?;
    inner.name = name.to_string();
    if !matches!(own, OwnDefault::Inherit) {
        apply_default(&mut inner, own);
    }
    Some(inner)
}

fn apply_default(param: &mut Param, own: OwnDefault) {
    match own {
        OwnDefault::Unset => {
            param.required = false;
            param.default = Some(None);
        }
        OwnDefault::Value(value) => {
            param.required = false;
            param.default = Some(Some(value));
        }
        OwnDefault::Required | OwnDefault::Inherit => {
            param.required = true;
            param.default = None;
        }
    }
}

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

fn ratings(tree: &Node) -> Vec<Rating> {
    tree.get("model")
        .and_then(|model| model.get("ratings"))
        .and_then(Node::props)
        .unwrap_or_default()
        .iter()
        .filter_map(|prop| {
            let value = prop.value.get("value")?.number()?;
            let unit = prop.value.get("unit")?.str()?.to_string();
            Some(Rating {
                name: prop.key.clone(),
                value,
                unit,
                source: prop.value.get("source").map(Node::to_value),
            })
        })
        .collect()
}

fn gaps(tree: &Node) -> Vec<NotSimulated> {
    tree.get("not_simulated")
        .and_then(Node::items)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| {
            let pins: Vec<String> = item
                .get("pins")
                .and_then(Node::items)
                .unwrap_or_default()
                .iter()
                .filter_map(Node::str)
                .map(str::to_string)
                .collect();
            Some(NotSimulated {
                what: item.get("what")?.str()?.to_string(),
                why: item.get("why")?.str()?.to_string(),
                pins: (!pins.is_empty()).then_some(pins),
            })
        })
        .collect()
}

fn docs(tree: &Node) -> Option<Vec<Doc>> {
    let props = tree.get("docs").and_then(Node::props)?;
    let docs: Vec<Doc> = props
        .iter()
        .map(|prop| Doc {
            name: prop.key.clone(),
            url: prop.value.str().map(str::to_string),
        })
        .collect();
    (!docs.is_empty()).then_some(docs)
}

fn files(part_license: &str, original: &Original) -> Vec<PartFile> {
    let mut files: Vec<PartFile> = original
        .files()
        .iter()
        .map(|(path, bytes)| PartFile {
            path: (*path).to_string(),
            sha256: hex(&Sha256::digest(bytes)),
            bytes: bytes.len() as u64,
            license: file_license(part_license, bytes),
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn file_license(part_license: &str, bytes: &[u8]) -> String {
    spdx(bytes).unwrap_or_else(|| {
        if part_license.is_empty() {
            PART_LICENSE.to_string()
        } else {
            part_license.to_string()
        }
    })
}

fn spdx(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    for line in text.lines().take(40) {
        let line = line.trim();
        let line = line
            .trim_start_matches("//")
            .trim_start_matches('#')
            .trim_start_matches("/*")
            .trim_start_matches('*')
            .trim();
        let Some(rest) = line.strip_prefix("SPDX-License-Identifier:") else {
            continue;
        };
        let id = rest.trim();
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    None
}

fn uses(tree: &Node) -> BTreeMap<String, String> {
    tree.get("uses")
        .and_then(Node::props)
        .unwrap_or_default()
        .iter()
        .filter_map(|prop| Some((prop.key.clone(), prop.value.str()?.to_string())))
        .collect()
}

fn on(tree: &Node) -> Option<On> {
    let on = tree
        .get("readings")
        .and_then(|readings| readings.get("on"))?;
    let above = on.get("above")?;
    Some(On {
        of: on.get("of")?.str()?.to_string(),
        above: above.get("value")?.number()?,
        unit: above.get("unit")?.str()?.to_string(),
    })
}

fn quoted(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn skip_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_spdx_line_overrides_the_part_license() {
        let bytes = b"// SPDX-License-Identifier: CC-BY-SA-4.0\n{}\n";
        assert_eq!(file_license("MIT OR Apache-2.0", bytes), "CC-BY-SA-4.0");
        assert_eq!(
            file_license(
                "MIT OR Apache-2.0",
                b"{\"license\": \"MIT OR Apache-2.0\"}\n"
            ),
            "MIT OR Apache-2.0"
        );
        assert_eq!(file_license("", b"not utf8 \xff"), PART_LICENSE);
    }
}
