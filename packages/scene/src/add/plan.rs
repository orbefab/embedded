//! What `orbe add` would do, as a pure function of the request, the catalog and a snapshot of
//! the project. It names the folders to write, or it refuses. A folder already at `parts/<id>/`
//! is `part_exists`: the command writes nothing, and nothing in that folder is compared.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::catalog::{Catalog, Version};
use crate::name::did_you_mean;
use crate::project;

use super::snapshot::{Snapshot, Target};
use super::{AddCode, Refusal, Request};

pub(super) enum Plan {
    Add(AddPlan),
    Refuse(Refusal),
    Usage(String),
}

pub(super) struct AddPlan {
    pub(super) write: Vec<Write>,
    pub(super) notes: Vec<String>,
}

/// A folder to write. An existing folder is `part_exists`, not a replacement.
pub(super) struct Write {
    pub(super) id: String,
    pub(super) version: Version,
}

struct Spec {
    id: String,
    version: Option<Version>,
    raw: String,
}

/// An id, version or registry the catalog does not have. This is the request's own error, so
/// it is reported before any look at the project folder. A missing project would hide it.
pub(super) fn refuse_unknown(request: &Request) -> Option<Refusal> {
    let specs = parse_specs(&request.ids).ok()?;
    match unknown_specs(&Catalog::builtin(), &specs) {
        Some(Plan::Refuse(refusal)) => Some(refusal),
        _ => None,
    }
}

/// The usage error in a request, which no project state changes.
pub(super) fn usage(request: &Request) -> Option<String> {
    if request.ids.is_empty() {
        let example =
            project::add_writing("orbe add", &["led-5mm".to_string()]).expect("the id is named");
        return Some(format!(
            "orbe add takes at least one part id, such as `{example}`"
        ));
    }
    parse_specs(&request.ids).err()
}

/// The ids whose `parts/<id>` the plan may write: every catalog id the request can reach
/// through `uses`, at any version.
pub(super) fn wanted(catalog: &Catalog, request: &Request) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = parse_specs(&request.ids)
        .unwrap_or_default()
        .into_iter()
        .map(|spec| spec.id)
        .collect();
    while let Some(id) = queue.pop() {
        if catalog.newest(&id).is_none() || !seen.insert(id.clone()) {
            continue;
        }
        for version in catalog.versions_of(&id) {
            let entry = catalog.get(&id, version).expect("a listed version");
            queue.extend(entry.uses.keys().cloned());
        }
    }
    seen
}

/// `add` is the `orbe add` command as hints print it, with `-C` when the run named a folder.
pub(super) fn plan(catalog: &Catalog, snapshot: &Snapshot, request: &Request, add: &str) -> Plan {
    if let Some(message) = usage(request) {
        return Plan::Usage(message);
    }
    if let Some(refusal) = unreadable_parts(snapshot) {
        return Plan::Refuse(refusal);
    }
    let specs = parse_specs(&request.ids).expect("usage checked the ids");
    if let Some(plan) = unknown_specs(catalog, &specs) {
        return plan;
    }
    let chosen = match choose(catalog, &specs) {
        Ok(chosen) => chosen,
        Err(plan) => return plan,
    };
    let mut existing = Vec::new();
    for (id, version) in &chosen {
        if let Some(line) = already(snapshot, id, version) {
            existing.push(line);
        }
    }
    if !existing.is_empty() {
        existing.sort();
        let pronoun = if existing.len() == 1 { "it" } else { "them" };
        let command = project::add_writing(add, &request.ids).expect("the request names an id");
        return Plan::Refuse(Refusal {
            code: AddCode::PartExists,
            message: existing.join("\n"),
            hint: format!("remove {pronoun} and run `{command}`"),
        });
    }
    let mut write: Vec<Write> = chosen
        .into_iter()
        .map(|(id, version)| Write { id, version })
        .collect();
    write.sort_by(|a, b| a.id.cmp(&b.id));
    Plan::Add(AddPlan {
        write,
        notes: Vec::new(),
    })
}

fn parse_specs(ids: &[String]) -> Result<Vec<Spec>, String> {
    let mut specs = Vec::new();
    let mut bad = Vec::new();
    for raw in ids {
        if raw.is_empty() || raw.contains(char::is_whitespace) {
            bad.push(format!("`{raw}` is not a part id"));
            continue;
        }
        let split = if raw.starts_with('@') {
            None
        } else {
            raw.split_once('@')
        };
        let Some((id, version)) = split else {
            specs.push(Spec {
                id: raw.clone(),
                version: None,
                raw: raw.clone(),
            });
            continue;
        };
        if id.is_empty() {
            bad.push(format!("`{raw}` is not a part id"));
            continue;
        }
        let Some(version) = Version::parse(version) else {
            bad.push(format!(
                "`{raw}` is not a version. a version is three numbers, such as `id@1.2.3`"
            ));
            continue;
        };
        specs.push(Spec {
            id: id.to_string(),
            version: Some(version),
            raw: raw.clone(),
        });
    }
    if bad.is_empty() {
        Ok(specs)
    } else {
        Err(bad.join("\n"))
    }
}

/// `@registry/id` is a real id shape, and this orbe copies the catalog built into it. Other
/// missing ids get a suggestion from this catalog.
fn unknown_specs(catalog: &Catalog, specs: &[Spec]) -> Option<Plan> {
    let mut registry = Vec::new();
    let mut unknown = Vec::new();
    let mut versions = Vec::new();
    for spec in specs {
        if spec.raw.starts_with('@') {
            registry.push(spec.raw.clone());
            continue;
        }
        if catalog.newest(&spec.id).is_none() {
            unknown.push(spec.id.clone());
            continue;
        }
        if let Some(version) = spec.version
            && catalog.get(&spec.id, version).is_none()
        {
            let have = catalog
                .versions_of(&spec.id)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            versions.push(format!(
                "the catalog has no `{}@{version}`. versions: {have}",
                spec.id
            ));
        }
    }
    if !registry.is_empty() {
        let listed = registry
            .iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let verb = if registry.len() == 1 { "is" } else { "are" };
        return Some(Plan::Refuse(Refusal {
            code: AddCode::Registry,
            message: format!(
                "{listed} {verb} not in the official catalog. v1 reads only the official catalog"
            ),
            hint: String::new(),
        }));
    }
    if !unknown.is_empty() {
        let mut hints = Vec::new();
        let mut lines = Vec::new();
        for id in &unknown {
            lines.push(format!("no Block or part named `{id}`"));
            let mut names = catalog.ids();
            for block in super::block::ids() {
                names.insert(block);
            }
            let suggestion = did_you_mean(id, names, "");
            let suggestion = suggestion.trim();
            if !suggestion.is_empty() {
                hints.push(suggestion.to_string());
            }
        }
        let hint = if hints.is_empty() {
            "the catalog has no Block or part with that id".into()
        } else {
            hints.join(" ")
        };
        return Some(Plan::Refuse(Refusal {
            code: AddCode::UnknownPart,
            message: lines.join("\n"),
            hint,
        }));
    }
    if !versions.is_empty() {
        return Some(Plan::Refuse(Refusal {
            code: AddCode::UnknownPart,
            message: versions.join("\n"),
            hint: String::new(),
        }));
    }
    None
}

/// The version of each id the command would copy: the one the request names, or the newest
/// this catalog carries, then the versions those parts use. Two demands for one id at two
/// versions, with neither named by the request, is a usage error.
fn choose(catalog: &Catalog, specs: &[Spec]) -> Result<BTreeMap<String, Version>, Plan> {
    let mut chosen = BTreeMap::new();
    let mut named = BTreeSet::new();
    for spec in specs {
        let version = spec.version.unwrap_or_else(|| {
            catalog
                .newest(&spec.id)
                .expect("unknown ids are refused first")
                .version
        });
        if let Some(previous) = chosen.get(&spec.id)
            && previous != &version
            && named.contains(&spec.id)
        {
            return Err(Plan::Usage(format!(
                "`{}` is named at `{previous}` and at `{version}`",
                spec.id
            )));
        }
        if spec.version.is_some() {
            named.insert(spec.id.clone());
        }
        chosen.insert(spec.id.clone(), version);
    }
    let mut queue: Vec<(String, Version, String)> = chosen
        .iter()
        .map(|(id, version)| (id.clone(), *version, id.clone()))
        .collect();
    let mut walked = BTreeSet::new();
    while let Some((id, version, by)) = queue.pop() {
        if !walked.insert((id.clone(), version)) {
            continue;
        }
        let entry = catalog.get(&id, version).ok_or_else(|| {
            Plan::Refuse(Refusal {
                code: AddCode::UnknownPart,
                message: format!("the catalog has no `{id}@{version}`"),
                hint: String::new(),
            })
        })?;
        for (child, child_version) in &entry.uses {
            if catalog.get(child, *child_version).is_none() {
                return Err(Plan::Refuse(Refusal {
                    code: AddCode::UnknownPart,
                    message: format!(
                        "`{id}` uses `{child}@{child_version}`, and the catalog has no `{child}@{child_version}`"
                    ),
                    hint: String::new(),
                }));
            }
            match chosen.get(child) {
                Some(have) if have != child_version && !named.contains(child) => {
                    return Err(Plan::Usage(format!(
                        "`{child}` is used at `{have}` and at `{child_version}`"
                    )));
                }
                Some(_) => {}
                None => {
                    chosen.insert(child.clone(), *child_version);
                    queue.push((child.clone(), *child_version, by.clone()));
                }
            }
        }
    }
    Ok(chosen)
}

/// The `part_exists` line for `id` when `parts/<id>` is already there. The version is the
/// copy's own. No other file in the folder is read.
fn already(snapshot: &Snapshot, id: &str, _version: &Version) -> Option<String> {
    match snapshot.target(id) {
        Target::Absent => None,
        Target::Folder(folder) => Some(match folder.version {
            Some(version) => format!("parts/{id} is already {id} {version}"),
            None => format!("parts/{id} is already there"),
        }),
        Target::Other(what) => Some(format!("parts/{id} is already {what}, not a folder")),
    }
}

/// `parts/` exists and this process cannot list it, so a missing folder is not the same
/// thing as a part that is absent.
fn unreadable_parts(snapshot: &Snapshot) -> Option<Refusal> {
    let denied = snapshot.parts_denied.as_ref()?;
    let hint = if denied.permission {
        project::chmod_hint(Some(&snapshot.root), Path::new("parts"), true)
    } else {
        "check that the path exists and is readable".into()
    };
    Some(Refusal {
        code: AddCode::Unreadable,
        message: format!("cannot read parts: {}", denied.error),
        hint,
    })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
