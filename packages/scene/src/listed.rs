//! The parts a scene places, with each value the scene writes. A page lists them. It reads no
//! firmware and builds no netlist: a file param is the path as written.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::diagnostic::{Code, Diagnostic, Findings};
use crate::files::Files;
use crate::model::{ParamDecl, ParamKind, passed};
use crate::name::did_you_mean;
use crate::params;
use crate::project::{self, Owner, Project};
use crate::source::Span;
use crate::syntax::{self, Node};

/// One value a scene writes on a part.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WrittenValue {
    pub name: String,
    /// A number param, in its SI unit. A duration is seconds, not picoseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    /// The param's unit, such as `ohm` or `1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// A choice, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A file param's path, as written. The file need not be present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// One part a scene places, in the scene's order.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScenePart {
    #[serde(rename = "ref")]
    pub r#ref: String,
    /// The catalog id, or the path a scene names the part by.
    pub part: String,
    /// The part file has a press input. A child of this part does not count.
    pub press: bool,
    pub values: Vec<WrittenValue>,
}

/// The top-level parts of `scene`, in file order, and the values the scene writes on each.
/// A part the scene does not read, or a value the reader refuses, is the error.
pub fn scene_parts(files: &Files, scene: &Path) -> Result<Vec<ScenePart>, Vec<Diagnostic>> {
    let mut findings = Findings::default();
    let project = match project::load(files, scene, &mut findings) {
        Ok(project) => project,
        Err(sources) => return Err(findings.into_diagnostics(&sources)),
    };
    if !findings.is_empty() {
        return Err(findings.into_diagnostics(&project.sources));
    }
    let mut listed = Vec::new();
    for (index, instance) in project.scene.comp.parts.iter().enumerate() {
        let Some(&part_index) = project.types.get(&(Owner::Scene, index)) else {
            findings.push(
                Code::Type,
                instance.name_span,
                format!("`{}` has no part", instance.name),
                "name a part this catalog has".into(),
            );
            continue;
        };
        let part = &project.parts[part_index];
        let mut values = Vec::new();
        for prop in &instance.params {
            match written(
                &project,
                part_index,
                &instance.name,
                &prop.key,
                &prop.value,
                prop.key_span,
            ) {
                Ok(Some(value)) => values.push(value),
                Ok(None) => {}
                Err((span, message, hint)) => findings.push(Code::Param, span, message, hint),
            }
        }
        listed.push(ScenePart {
            r#ref: instance.name.clone(),
            part: part.label.clone(),
            press: press_input(&project, part.file),
            values,
        });
    }
    let diagnostics = findings.into_diagnostics(&project.sources);
    if diagnostics.is_empty() {
        Ok(listed)
    } else {
        Err(diagnostics)
    }
}

/// The param `name` as the part declares it. A param the part only passes through is the param
/// the child declares, which is what a value written here sets.
fn effective<'a>(project: &'a Project<'_>, part_index: usize, name: &str) -> Option<&'a ParamDecl> {
    let mut at = part_index;
    let mut key = name.to_string();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert((at, key.clone())) {
            return None;
        }
        let part = &project.parts[at];
        let decl = part.param(&key)?;
        if !matches!(decl.kind, ParamKind::Through) {
            return Some(decl);
        }
        let mut forwarded = None;
        for (index, instance) in part.comp.parts.iter().enumerate() {
            for prop in &instance.params {
                if passed(&prop.value) != Some(key.as_str()) {
                    continue;
                }
                let Some(&child) = project.types.get(&(Owner::Part(at), index)) else {
                    continue;
                };
                forwarded = Some((child, prop.key.clone()));
                break;
            }
            if forwarded.is_some() {
                break;
            }
        }
        let Some((child, child_key)) = forwarded else {
            return Some(decl);
        };
        at = child;
        key = child_key;
    }
}

fn written(
    project: &Project<'_>,
    part_index: usize,
    instance: &str,
    name: &str,
    node: &Node,
    key_span: Span,
) -> Result<Option<WrittenValue>, (Span, String, String)> {
    let part = &project.parts[part_index];
    if part.param(name).is_none() {
        let names: Vec<&str> = part
            .params
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        let list = if names.is_empty() {
            "it takes no params".to_string()
        } else {
            format!("its params: {}", names.join(", "))
        };
        let hint = did_you_mean(name, names.iter().copied(), "");
        return Err((
            key_span,
            format!("`{instance}` is a {} and has no param `{name}`", part.label),
            format!("{hint}{list}"),
        ));
    }
    if passed(node).is_some() {
        return Err((
            node.span,
            "a scene sets a value; only a part passes its own param through with { \"param\": ... }"
                .into(),
            "set the value here, such as \"220\" or \"1ms\"".into(),
        ));
    }
    let Some(decl) = effective(project, part_index, name) else {
        return Err((
            key_span,
            format!("`{name}` passes through a part that reaches itself"),
            "a param cannot pass through to itself".into(),
        ));
    };
    if node.is_null() {
        return if decl.default.as_ref().is_some_and(Node::is_null) {
            Ok(None)
        } else {
            Err((
                node.span,
                "null is not a value".into(),
                "write the value, or leave the param out".into(),
            ))
        };
    }
    match &decl.kind {
        ParamKind::Number { unit, range } => {
            let value = params::number(*unit, *range, node)
                .map_err(|(_, message, hint)| (node.span, message, hint))?;
            let amount = value["value"]
                .as_f64()
                .expect("a number param's value is a number");
            let unit_text = value["unit"]
                .as_str()
                .expect("a number param's value names its unit");
            // The reader stores a duration in picoseconds. A page wants seconds.
            let (amount, unit_text) = if unit_text == "ps" {
                (amount / 1e12, "s")
            } else {
                (amount, unit_text)
            };
            Ok(Some(WrittenValue {
                name: name.to_string(),
                amount: Some(amount),
                unit: Some(unit_text.to_string()),
                text: None,
                file: None,
            }))
        }
        ParamKind::Choice(choices) => {
            let names: Vec<&str> = choices.iter().map(|(choice, _)| choice.as_str()).collect();
            let Some(text) = node.str().filter(|text| names.contains(text)) else {
                let written = node
                    .str()
                    .map_or_else(|| node.what().to_string(), |text| format!("`{text}`"));
                let hint = node
                    .str()
                    .map(|text| did_you_mean(text, names.iter().copied(), ""))
                    .unwrap_or_default();
                return Err((
                    node.span,
                    format!("{written} is not a choice of `{name}`"),
                    format!("{hint}the choices: {}", names.join(", ")),
                ));
            };
            Ok(Some(WrittenValue {
                name: name.to_string(),
                amount: None,
                unit: None,
                text: Some(text.to_string()),
                file: None,
            }))
        }
        ParamKind::File(_) => {
            let Some(path) = node.str() else {
                return Err((
                    node.span,
                    format!("a file param takes a path, found {}", node.what()),
                    "write a path relative to the file that sets it".into(),
                ));
            };
            Ok(Some(WrittenValue {
                name: name.to_string(),
                amount: None,
                unit: None,
                text: None,
                file: Some(path.to_string()),
            }))
        }
        ParamKind::Through => {
            let Some(text) = node.str() else {
                return Err((
                    node.span,
                    format!("`{name}` takes a value, found {}", node.what()),
                    "write the value this param passes on".into(),
                ));
            };
            Ok(Some(WrittenValue {
                name: name.to_string(),
                amount: None,
                unit: None,
                text: Some(text.to_string()),
                file: None,
            }))
        }
    }
}

fn press_input(project: &Project<'_>, file: crate::source::FileId) -> bool {
    syntax::parse(&project.sources, file, &mut Findings::default()).is_some_and(|tree| {
        tree.get("controls")
            .and_then(|controls| controls.get("press"))
            .is_some()
    })
}
