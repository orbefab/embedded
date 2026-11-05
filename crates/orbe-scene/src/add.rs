//! `orbe add` decides what to copy, then writes.
//!
//! [`plan`] is a pure function of the request, the catalog and a snapshot of the project. It
//! either names the folders to write or refuses. [`run`] is the only function that creates
//! part folders or `parts.lock`, and it does that by staging into a temporary folder and
//! renaming into place.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::catalog::{Catalog, CatalogEntry, Version};
use crate::diagnostic::Findings;
use crate::name::did_you_mean;
use crate::project::{self, LockFields, hash_files};
use crate::source::Sources;
use crate::syntax::{self, Node};

const REQUEST: &str = "request";
const TESTS_MAY_CHANGE: &str = "tests may change";

/// What the CLI asked `orbe add` to do. The project is the current directory.
pub struct Request {
    pub ids: Vec<String>,
    pub overwrite: bool,
    pub diff: bool,
    pub relock: bool,
}

/// A reply the CLI prints as text or as one JSON envelope.
pub enum Reply {
    Add(AddReply),
    Diff(DiffReply),
    Relock(RelockReply),
    Refuse(Refusal),
    Usage(String),
}

pub struct AddReply {
    pub parts: Vec<Copied>,
    pub lock_written: bool,
    pub notes: Vec<String>,
}

pub struct Copied {
    pub id: String,
    pub version: String,
    pub wrote: bool,
}

pub struct DiffReply {
    pub notes: Vec<String>,
    pub files: String,
}

pub struct RelockReply {
    pub lock_written: bool,
    pub parts: Vec<String>,
    pub modified: Vec<String>,
    pub notes: Vec<String>,
}

pub struct Refusal {
    pub code: &'static str,
    pub message: String,
    pub hint: String,
}

/// A project file could not be read or written. The CLI exits 3.
#[derive(Debug)]
pub struct RunError {
    message: String,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Reply {
    pub fn code(&self) -> u8 {
        match self {
            Reply::Usage(_) => 2,
            Reply::Refuse(_) => 1,
            _ => 0,
        }
    }

    pub fn text(&self) -> String {
        match self {
            Reply::Add(reply) => add_text(reply),
            Reply::Diff(reply) => diff_text(reply),
            Reply::Relock(reply) => relock_text(reply),
            Reply::Refuse(refusal) => refuse_text(refusal),
            Reply::Usage(message) => format!("error[usage]: {message}"),
        }
    }

    pub fn json(&self) -> Value {
        match self {
            Reply::Add(reply) => json!({
                "ok": true,
                "data": {
                    "written": versions(&reply.parts, true),
                    "skipped": versions(&reply.parts, false),
                    "lock_written": reply.lock_written,
                    "notes": reply.notes,
                }
            }),
            Reply::Diff(reply) => json!({
                "ok": true,
                "data": { "diff": reply.files, "notes": reply.notes }
            }),
            Reply::Relock(reply) => json!({
                "ok": true,
                "data": {
                    "lock_written": reply.lock_written,
                    "parts": reply.parts,
                    "modified": reply.modified,
                    "notes": reply.notes,
                }
            }),
            Reply::Refuse(refusal) => refuse_json(refusal),
            Reply::Usage(message) => json!({
                "ok": false,
                "error": { "code": "usage", "message": message }
            }),
        }
    }
}

/// Copies parts into `root` and writes `parts.lock`. A refusal writes nothing.
pub fn run(root: &Path, request: &Request) -> Result<Reply, RunError> {
    let restored = recover(root)?;
    let catalog = Catalog::builtin();
    let snapshot = load(root)?;
    let reply = match plan(&catalog, &snapshot, request) {
        Plan::Add(planned) => {
            write_plan(root, &catalog, &planned.write, &planned.lock)?;
            Reply::Add(summarize_add(planned))
        }
        Plan::Relock(planned) => {
            write_plan(root, &catalog, &[], &planned.lock)?;
            Reply::Relock(RelockReply {
                lock_written: planned.lock.is_some(),
                parts: planned.parts,
                modified: planned.modified,
                notes: Vec::new(),
            })
        }
        Plan::Diff(planned) => Reply::Diff(DiffReply {
            notes: planned.notes,
            files: planned.files,
        }),
        Plan::Refuse(refusal) => Reply::Refuse(refusal),
        Plan::Usage(message) => Reply::Usage(message),
    };
    Ok(with_restored(reply, restored))
}

/// A previous `orbe add` may have moved a folder aside and then stopped. Put that folder back
/// when its place is empty, then remove the staging folder.
fn recover(root: &Path) -> Result<Vec<String>, RunError> {
    let staging = root.join(".orbe-add-staging");
    if !staging.exists() {
        return Ok(Vec::new());
    }
    let mut restored = Vec::new();
    if staging.is_dir() {
        let mut names = Vec::new();
        for entry in fs::read_dir(&staging).map_err(|error| io_at(&staging, error))? {
            let entry = entry.map_err(|error| io_at(&staging, error))?;
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
        names.sort();
        for name in names {
            let Some(id) = name.strip_prefix("old-") else {
                continue;
            };
            if !single_component(id) {
                continue;
            }
            let aside = staging.join(&name);
            if !aside.is_dir() {
                continue;
            }
            let dest = root.join("parts").join(id);
            if dest.exists() {
                continue;
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|error| io_at(parent, error))?;
            }
            fs::rename(&aside, &dest).map_err(|error| io_at(&dest, error))?;
            restored.push(id.to_string());
        }
    }
    if staging.is_dir() {
        fs::remove_dir_all(&staging).map_err(|error| io_at(&staging, error))?;
    } else {
        fs::remove_file(&staging).map_err(|error| io_at(&staging, error))?;
    }
    Ok(restored)
}

fn with_restored(reply: Reply, restored: Vec<String>) -> Reply {
    if restored.is_empty() {
        return reply;
    }
    let lines: Vec<String> = restored
        .into_iter()
        .map(|id| format!("restored parts/{id} from an interrupted add"))
        .collect();
    match reply {
        Reply::Add(mut reply) => {
            reply.notes.extend(lines);
            Reply::Add(reply)
        }
        Reply::Diff(mut reply) => {
            reply.notes.extend(lines);
            Reply::Diff(reply)
        }
        Reply::Relock(mut reply) => {
            reply.notes.extend(lines);
            Reply::Relock(reply)
        }
        Reply::Refuse(mut refusal) => {
            if !refusal.message.is_empty() {
                refusal.message.push('\n');
            }
            refusal.message.push_str(&lines.join("\n"));
            Reply::Refuse(refusal)
        }
        Reply::Usage(message) => Reply::Usage(format!("{message}\n{}", lines.join("\n"))),
    }
}

struct FolderSnap {
    version: Option<Version>,
    hash: String,
    files: BTreeMap<String, Vec<u8>>,
}

impl FolderSnap {
    fn from_files(files: BTreeMap<String, Vec<u8>>) -> FolderSnap {
        let version = files.get("part.jsonc").and_then(|bytes| version_of(bytes));
        let hash = hash_files(
            files
                .iter()
                .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        );
        FolderSnap {
            version,
            hash,
            files,
        }
    }
}

struct Snapshot {
    folders: BTreeMap<String, FolderSnap>,
    lock: BTreeMap<String, LockFields>,
    lock_text: Option<String>,
    lock_error: Option<String>,
}

enum Plan {
    Add(AddPlan),
    Diff(DiffPlan),
    Relock(RelockPlan),
    Refuse(Refusal),
    Usage(String),
}

struct AddPlan {
    write: Vec<(String, Version)>,
    skip: Vec<(String, Version)>,
    lock: Option<String>,
    notes: Vec<String>,
}

struct DiffPlan {
    notes: Vec<String>,
    files: String,
}

struct RelockPlan {
    lock: Option<String>,
    parts: Vec<String>,
    modified: Vec<String>,
}

struct Spec {
    id: String,
    version: Option<Version>,
    raw: String,
}

struct Demand {
    version: Version,
    by: String,
}

fn plan(catalog: &Catalog, snapshot: &Snapshot, request: &Request) -> Plan {
    if request.relock {
        if !request.ids.is_empty() || request.diff || request.overwrite {
            return Plan::Usage(
                "orbe add --relock takes no part ids, --diff, or --overwrite".into(),
            );
        }
        return plan_relock(catalog, snapshot);
    }
    if request.ids.is_empty() {
        return Plan::Usage(
            "orbe add takes at least one part id, such as `orbe add led-5mm`".into(),
        );
    }
    let specs = match parse_specs(&request.ids) {
        Ok(specs) => specs,
        Err(plan) => return plan,
    };
    if let Some(plan) = unknown_specs(catalog, &specs) {
        return plan;
    }
    if let Some(message) = snapshot.lock_error.clone() {
        return Plan::Refuse(Refusal {
            code: "lock_syntax",
            message,
            hint: "run `orbe add --relock`".into(),
        });
    }
    let selection = match select(catalog, &specs, request.overwrite) {
        Ok(selection) => selection,
        Err(plan) => return plan,
    };
    if request.diff {
        return plan_diff(catalog, snapshot, &selection);
    }
    plan_add(catalog, snapshot, &specs, &selection, request.overwrite)
}

fn parse_specs(ids: &[String]) -> Result<Vec<Spec>, Plan> {
    let mut specs = Vec::new();
    let mut bad = Vec::new();
    for raw in ids {
        if raw.is_empty() || raw.contains(char::is_whitespace) {
            bad.push(format!("`{raw}` is not a part id"));
            continue;
        }
        if raw.starts_with('@') {
            specs.push(Spec {
                id: raw.clone(),
                version: None,
                raw: raw.clone(),
            });
            continue;
        }
        let Some((id, version)) = raw.split_once('@') else {
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
                "`{raw}` is not a version. a version is three numbers, such as `led-5mm@0.1.0`"
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
        Err(Plan::Usage(bad.join("\n")))
    }
}

/// `@registry/id` is a real id shape, and v1 has only the official catalog. Other missing
/// ids get a suggestion from this catalog.
fn unknown_specs(catalog: &Catalog, specs: &[Spec]) -> Option<Plan> {
    let mut registry = Vec::new();
    let mut unknown = Vec::new();
    let mut versions = Vec::new();
    for spec in specs {
        if spec.id.starts_with('@') || spec.raw.starts_with('@') {
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
            code: "registry",
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
            lines.push(format!("no catalog part named `{id}`"));
            let suggestion = did_you_mean(id, catalog.ids(), "");
            let suggestion = suggestion.trim();
            if !suggestion.is_empty() {
                hints.push(suggestion.to_string());
            }
        }
        let hint = if hints.is_empty() {
            "the catalog has no part with that id".into()
        } else {
            hints.join(" ")
        };
        return Some(Plan::Refuse(Refusal {
            code: "unknown_part",
            message: lines.join("\n"),
            hint,
        }));
    }
    if !versions.is_empty() {
        return Some(Plan::Refuse(Refusal {
            code: "unknown_version",
            message: versions.join("\n"),
            hint: String::new(),
        }));
    }
    None
}

struct Selection {
    chosen: BTreeMap<String, Version>,
    notes: Vec<String>,
    tests_may_change: bool,
}

fn select(catalog: &Catalog, specs: &[Spec], overwrite: bool) -> Result<Selection, Plan> {
    let mut queue = Vec::new();
    for spec in specs {
        let version = spec.version.unwrap_or_else(|| {
            catalog
                .newest(&spec.id)
                .expect("unknown ids are refused before select")
                .version
        });
        queue.push((spec.id.clone(), version, REQUEST.to_string()));
    }
    let mut demands: BTreeMap<String, Vec<Demand>> = BTreeMap::new();
    let mut walked = BTreeSet::new();
    while let Some((id, version, by)) = queue.pop() {
        demands
            .entry(id.clone())
            .or_default()
            .push(Demand { version, by });
        if !walked.insert((id.clone(), version)) {
            continue;
        }
        let entry = catalog.get(&id, version).ok_or_else(|| {
            Plan::Refuse(Refusal {
                code: "unknown_version",
                message: format!("the catalog has no `{id}@{version}`"),
                hint: String::new(),
            })
        })?;
        for (child, child_version) in &entry.uses {
            if catalog.get(child, *child_version).is_none() {
                return Err(Plan::Refuse(Refusal {
                    code: "unknown_version",
                    message: format!(
                        "`{id}` uses `{child}@{child_version}`, and the catalog has no `{child}@{child_version}`"
                    ),
                    hint: String::new(),
                }));
            }
            queue.push((child.clone(), *child_version, id.clone()));
        }
    }

    let mut chosen = BTreeMap::new();
    let mut notes = Vec::new();
    let mut tests_may_change = false;
    let mut clashes = Vec::new();
    for (id, group) in &demands {
        match choose(id, group, specs, overwrite) {
            Choose::Take(version) => {
                chosen.insert(id.clone(), version);
            }
            Choose::Newer(version, note) => {
                chosen.insert(id.clone(), version);
                notes.push(note);
            }
            Choose::Named(version) => {
                chosen.insert(id.clone(), version);
                tests_may_change = true;
            }
            Choose::Clash(clash) => clashes.push(clash),
        }
    }
    if !clashes.is_empty() {
        return Err(Plan::Refuse(Refusal {
            code: "part_conflict",
            message: clashes
                .iter()
                .map(|clash| clash.message.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            hint: clash_hint(specs, &clashes),
        }));
    }
    if tests_may_change {
        notes.push(TESTS_MAY_CHANGE.to_string());
    }
    Ok(Selection {
        chosen,
        notes,
        tests_may_change,
    })
}

enum Choose {
    Take(Version),
    Newer(Version, String),
    Named(Version),
    Clash(Clash),
}

struct Clash {
    message: String,
    cross_major: bool,
    id: String,
    versions: Vec<Version>,
}

fn choose(id: &str, group: &[Demand], specs: &[Spec], overwrite: bool) -> Choose {
    let mut versions: Vec<Version> = group.iter().map(|demand| demand.version).collect();
    versions.sort();
    versions.dedup();
    if versions.len() == 1 {
        return Choose::Take(versions[0]);
    }
    let message = format!("`{id}` is {}", describe(group));
    let same_major = versions
        .windows(2)
        .all(|pair| pair[0].major == pair[1].major);
    if same_major && overwrite {
        let newer = versions
            .iter()
            .copied()
            .max()
            .expect("at least two versions");
        let note = format!("took `{id}` {newer}, the newer of {}", describe(group));
        return Choose::Newer(newer, note);
    }
    if !same_major
        && overwrite
        && let Some(named) = explicit_version(specs, id)
    {
        return Choose::Named(named);
    }
    Choose::Clash(Clash {
        message,
        cross_major: !same_major,
        id: id.to_string(),
        versions,
    })
}

fn explicit_version(specs: &[Spec], id: &str) -> Option<Version> {
    let mut named: Vec<Version> = specs
        .iter()
        .filter(|spec| spec.id == id)
        .filter_map(|spec| spec.version)
        .collect();
    named.sort();
    named.dedup();
    if named.len() == 1 {
        Some(named[0])
    } else {
        None
    }
}

fn describe(group: &[Demand]) -> String {
    let mut versions: Vec<Version> = group.iter().map(|demand| demand.version).collect();
    versions.sort();
    versions.dedup();
    let phrases: Vec<String> = versions
        .into_iter()
        .map(|version| {
            let mut bys: Vec<&str> = group
                .iter()
                .filter(|demand| demand.version == version)
                .map(|demand| demand.by.as_str())
                .collect();
            bys.sort();
            bys.dedup();
            let who = join_and(&bys.iter().map(|by| who(by)).collect::<Vec<_>>());
            format!("`{version}` in {who}")
        })
        .collect();
    join_and(&phrases)
}

fn who(by: &str) -> String {
    if by == REQUEST {
        "the request".into()
    } else {
        format!("`{by}`")
    }
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        _ => {
            let (last, rest) = items.split_last().expect("at least three");
            format!("{}, and {last}", rest.join(", "))
        }
    }
}

fn clash_hint(specs: &[Spec], clashes: &[Clash]) -> String {
    let cross: Vec<&Clash> = clashes.iter().filter(|clash| clash.cross_major).collect();
    if cross.is_empty() {
        return format!("run `{}`", command_for(specs, &[], true));
    }
    if cross.len() == 1 {
        let clash = cross[0];
        let commands: Vec<String> = clash
            .versions
            .iter()
            .map(|version| {
                format!(
                    "`{}`",
                    command_for(specs, &[(clash.id.clone(), *version)], true)
                )
            })
            .collect();
        return format!("run {}", join_or(&commands));
    }
    "name one version of each id with `@<version> --overwrite`".into()
}

fn join_or(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} or {second}"),
        _ => {
            let (last, rest) = items.split_last().expect("at least three");
            format!("{}, or {last}", rest.join(", "))
        }
    }
}

/// `pins` replaces or appends `id@version` for ids that must be named.
fn command_for(specs: &[Spec], pins: &[(String, Version)], overwrite: bool) -> String {
    let mut args = Vec::new();
    let mut pending: BTreeSet<&str> = pins.iter().map(|(id, _)| id.as_str()).collect();
    for spec in specs {
        if let Some((_, version)) = pins.iter().find(|(id, _)| id == &spec.id) {
            args.push(format!("{}@{version}", spec.id));
            pending.remove(spec.id.as_str());
        } else {
            args.push(spec.raw.clone());
        }
    }
    for (id, version) in pins {
        if pending.remove(id.as_str()) {
            args.push(format!("{id}@{version}"));
        }
    }
    if overwrite {
        args.push("--overwrite".into());
    }
    format!("orbe add {}", args.join(" "))
}

struct Differ {
    line: String,
    id: String,
    version: Version,
    needs_explicit: bool,
}

fn plan_add(
    catalog: &Catalog,
    snapshot: &Snapshot,
    specs: &[Spec],
    selection: &Selection,
    overwrite: bool,
) -> Plan {
    let explicit: BTreeSet<&str> = specs
        .iter()
        .filter(|spec| spec.version.is_some())
        .map(|spec| spec.id.as_str())
        .collect();
    let mut write = Vec::new();
    let mut skip = Vec::new();
    let mut differs = Vec::new();
    let mut notes = selection.notes.clone();
    let mut tests_may_change = selection.tests_may_change;
    let mut ids: Vec<&String> = selection.chosen.keys().collect();
    ids.sort();
    for id in ids {
        let version = selection.chosen[id];
        let entry = catalog
            .get(id, version)
            .expect("select only keeps catalog versions");
        let Some(folder) = snapshot.folders.get(id) else {
            write.push((id.clone(), version));
            continue;
        };
        if folder.hash == entry.hash {
            skip.push((id.clone(), version));
            continue;
        }
        let crosses = folder
            .version
            .is_none_or(|copy| copy.major != version.major);
        let named = explicit.contains(id.as_str());
        if overwrite && (!crosses || named) {
            if crosses {
                tests_may_change = true;
            }
            write.push((id.clone(), version));
            continue;
        }
        differs.push(Differ {
            line: differ_line(id, folder, entry),
            id: id.clone(),
            version,
            needs_explicit: crosses,
        });
    }
    if !differs.is_empty() {
        let pins: Vec<(String, Version)> = differs
            .iter()
            .filter(|differ| differ.needs_explicit)
            .map(|differ| (differ.id.clone(), differ.version))
            .collect();
        return Plan::Refuse(Refusal {
            code: "part_differs",
            message: format!(
                "nothing was written\n{}",
                differs
                    .iter()
                    .map(|differ| differ.line.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            hint: format!("run `{}`", command_for(specs, &pins, true)),
        });
    }
    if tests_may_change && !notes.iter().any(|note| note == TESTS_MAY_CHANGE) {
        notes.push(TESTS_MAY_CHANGE.to_string());
    }
    let lock_text = lock_text(catalog, snapshot, &selection.chosen);
    let lock = (snapshot.lock_text.as_deref() != Some(lock_text.as_str())).then_some(lock_text);
    Plan::Add(AddPlan {
        write,
        skip,
        lock,
        notes,
    })
}

fn differ_line(id: &str, folder: &FolderSnap, entry: &CatalogEntry) -> String {
    let files = differing_names(folder, entry);
    let files = if files.is_empty() {
        "the folder".into()
    } else {
        files.join(", ")
    };
    match folder.version {
        Some(copy) if copy == entry.version => {
            format!("`{id}` is {copy} in the project and differs in {files}")
        }
        Some(copy) => format!(
            "`{id}` is {copy} in the project and {} in the catalog, and differs in {files}",
            entry.version
        ),
        None => format!("`{id}` has no version in the project, and differs in {files}"),
    }
}

fn differing_names(folder: &FolderSnap, entry: &CatalogEntry) -> Vec<String> {
    let mut names = Vec::new();
    for (rel, bytes) in &entry.files {
        match folder.files.get(rel) {
            Some(copy) if copy == bytes => {}
            Some(_) => names.push(format!("`{rel}`")),
            None => names.push(format!("`{rel}`, which is missing")),
        }
    }
    for rel in folder.files.keys() {
        if !entry.files.contains_key(rel) {
            names.push(format!("`{rel}`, which the catalog's copy does not have"));
        }
    }
    names
}

fn lock_text(catalog: &Catalog, snapshot: &Snapshot, chosen: &BTreeMap<String, Version>) -> String {
    let mut lines = snapshot.lock.clone();
    for (id, version) in chosen {
        let entry = catalog.get(id, *version).expect("chosen version exists");
        lines.insert(
            id.clone(),
            LockFields {
                id: id.clone(),
                version: version.to_string(),
                registry: catalog.registry().to_string(),
                hash: entry.hash.clone(),
            },
        );
    }
    project::render_lock(&lines)
}

fn plan_diff(catalog: &Catalog, snapshot: &Snapshot, selection: &Selection) -> Plan {
    let mut notes = selection.notes.clone();
    let mut files = String::new();
    let mut ids: Vec<&String> = selection.chosen.keys().collect();
    ids.sort();
    for id in ids {
        let version = selection.chosen[id];
        let entry = catalog.get(id, version).expect("chosen version exists");
        let folder = snapshot.folders.get(id);
        if folder.is_some_and(|folder| folder.hash == entry.hash) {
            continue;
        }
        let from = folder.and_then(|folder| folder.version);
        notes.push(change_note(id, from, version));
        files.push_str(&diff_files(id, folder, entry));
    }
    if files.is_empty() {
        notes.push("nothing would change".into());
    }
    Plan::Diff(DiffPlan { notes, files })
}

fn change_note(id: &str, from: Option<Version>, to: Version) -> String {
    match from {
        None => format!("`{id}` {to} is a new copy"),
        Some(from) if from == to => format!("`{id}` stays at {to}. the version is unchanged"),
        Some(from) if from.major != to.major => {
            format!("`{id}` {from} to {to}: a major version may change a passing test")
        }
        Some(from) if from.minor != to.minor => {
            format!("`{id}` {from} to {to}: a minor version cannot change a passing test")
        }
        Some(from) => {
            format!("`{id}` {from} to {to}: a patch version cannot change a passing test")
        }
    }
}

fn diff_files(id: &str, folder: Option<&FolderSnap>, entry: &CatalogEntry) -> String {
    let empty = BTreeMap::new();
    let have = folder.map(|folder| &folder.files).unwrap_or(&empty);
    let paths: BTreeSet<&str> = have
        .keys()
        .map(String::as_str)
        .chain(entry.files.keys().map(String::as_str))
        .collect();
    let mut out = String::new();
    for rel in paths {
        let path = format!("parts/{id}/{rel}");
        match (have.get(rel), entry.files.get(rel)) {
            (_, Some(_)) if folder.is_none() => out.push_str(&format!("{path} added\n")),
            (None, Some(_)) => out.push_str(&format!("{path} added\n")),
            (Some(_), None) => out.push_str(&format!("{path} removed\n")),
            (Some(old), Some(new)) if old == new => {}
            (Some(old), Some(new)) => out.push_str(&diff_one(&path, old, new)),
            (None, None) => {}
        }
    }
    out
}

fn diff_one(path: &str, old: &[u8], new: &[u8]) -> String {
    let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        return format!("{path} differs\n");
    };
    let body = line_diff(old, new);
    if body.is_empty() {
        format!("{path} differs\n")
    } else {
        format!("{path}\n{body}")
    }
}

/// Lines removed from the project copy, then lines the catalog copy would write.
fn line_diff(old: &str, new: &str) -> String {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let (n, m) = (old.len(), new.len());
    if n.saturating_mul(m) > 2_000_000 {
        return String::new();
    }
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = String::new();
    let mut i = 0;
    let mut j = 0;
    while i < n && j < m {
        if old[i] == new[j] {
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push('-');
            out.push_str(old[i]);
            out.push('\n');
            i += 1;
        } else {
            out.push('+');
            out.push_str(new[j]);
            out.push('\n');
            j += 1;
        }
    }
    while i < n {
        out.push('-');
        out.push_str(old[i]);
        out.push('\n');
        i += 1;
    }
    while j < m {
        out.push('+');
        out.push_str(new[j]);
        out.push('\n');
        j += 1;
    }
    out
}

fn plan_relock(catalog: &Catalog, snapshot: &Snapshot) -> Plan {
    let mut lines = BTreeMap::new();
    let mut modified = Vec::new();
    let mut problems = Vec::new();
    let mut sealed = Vec::new();
    for (id, folder) in &snapshot.folders {
        if catalog.newest(id).is_none() {
            continue;
        }
        let Some(version) = folder.version else {
            problems.push(format!("`{id}` has no version"));
            continue;
        };
        let Some(entry) = catalog.get(id, version) else {
            problems.push(format!("the catalog has no `{id}@{version}`"));
            continue;
        };
        if folder.hash != entry.hash && entry.sealed {
            let files = differing_names(folder, entry);
            let files = if files.is_empty() {
                "the folder".into()
            } else {
                files.join(", ")
            };
            problems.push(format!(
                "sealed part `{id}` differs from `{id}@{version}` in {files}"
            ));
            sealed.push(id.clone());
            continue;
        }
        let hash = if folder.hash == entry.hash {
            entry.hash.clone()
        } else {
            modified.push(id.clone());
            folder.hash.clone()
        };
        lines.insert(
            id.clone(),
            LockFields {
                id: id.clone(),
                version: version.to_string(),
                registry: catalog.registry().to_string(),
                hash,
            },
        );
    }
    if !problems.is_empty() {
        let hint = if sealed.is_empty() {
            String::new()
        } else {
            sealed
                .iter()
                .map(|id| {
                    format!(
                        "restore it with `orbe add {id} --overwrite`, or copy it to a path to make it your own"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let code = if sealed.is_empty() {
            "unknown_version"
        } else {
            "part_modified"
        };
        return Plan::Refuse(Refusal {
            code,
            message: problems.join("\n"),
            hint,
        });
    }
    let parts: Vec<String> = lines.keys().cloned().collect();
    let text = project::render_lock(&lines);
    let lock = (snapshot.lock_text.as_deref() != Some(text.as_str())).then_some(text);
    Plan::Relock(RelockPlan {
        lock,
        parts,
        modified,
    })
}

fn version_of(bytes: &[u8]) -> Option<Version> {
    let text = std::str::from_utf8(bytes).ok()?.to_string();
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new("part.jsonc"), text);
    let tree = syntax::parse(&sources, file, &mut Findings::default())?;
    tree.get("version")
        .and_then(Node::str)
        .and_then(Version::parse)
}

fn load(root: &Path) -> Result<Snapshot, RunError> {
    let mut folders = BTreeMap::new();
    let parts = root.join("parts");
    if parts.exists() {
        if !parts.is_dir() {
            return Err(io_at(&parts, io::Error::other("`parts` is not a folder")));
        }
        for entry in fs::read_dir(&parts).map_err(|error| io_at(&parts, error))? {
            let entry = entry.map_err(|error| io_at(&parts, error))?;
            let name = entry.file_name();
            let Some(id) = name.to_str() else {
                continue;
            };
            if id.starts_with('.') || !entry.path().is_dir() || !single_component(id) {
                continue;
            }
            let files = read_tree(&entry.path())?;
            folders.insert(id.to_string(), FolderSnap::from_files(files));
        }
    }
    let lock_path = root.join("parts.lock");
    let (lock_text, lock, lock_error) = if lock_path.is_file() {
        let text = fs::read_to_string(&lock_path).map_err(|error| io_at(&lock_path, error))?;
        let mut lines = BTreeMap::new();
        let mut error = None;
        for line in text.lines() {
            match project::lock_fields(line) {
                Ok(None) => {}
                Ok(Some(fields)) => {
                    lines.insert(fields.id.clone(), fields);
                }
                Err(()) => {
                    error = Some(format!(
                        "a parts.lock line is `<id> <version> <registry> sha256:<hash>`\n{line}"
                    ));
                    break;
                }
            }
        }
        (Some(text), lines, error)
    } else if lock_path.exists() {
        return Err(io_at(
            &lock_path,
            io::Error::other("`parts.lock` is not a file"),
        ));
    } else {
        (None, BTreeMap::new(), None)
    };
    Ok(Snapshot {
        folders,
        lock,
        lock_text,
        lock_error,
    })
}

fn read_tree(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, RunError> {
    let mut files = BTreeMap::new();
    read_tree_into(dir, dir, &mut files)?;
    Ok(files)
}

fn read_tree_into(
    root: &Path,
    dir: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), RunError> {
    for entry in fs::read_dir(dir).map_err(|error| io_at(dir, error))? {
        let entry = entry.map_err(|error| io_at(dir, error))?;
        let path = entry.path();
        if path.is_dir() {
            read_tree_into(root, &path, files)?;
        } else {
            let bytes = fs::read(&path).map_err(|error| io_at(&path, error))?;
            files.insert(crate::source::display(root, &path), bytes);
        }
    }
    Ok(())
}

fn single_component(id: &str) -> bool {
    let mut parts = Path::new(id).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(name)), None) => name.to_str() == Some(id),
        _ => false,
    }
}

fn write_plan(
    root: &Path,
    catalog: &Catalog,
    write: &[(String, Version)],
    lock: &Option<String>,
) -> Result<(), RunError> {
    if write.is_empty() && lock.is_none() {
        return Ok(());
    }
    for (id, _) in write {
        if !single_component(id) {
            return Err(RunError {
                message: format!("`{id}` is not a single folder name"),
            });
        }
    }
    let path = root.join(".orbe-add-staging");
    if path.exists() {
        fs::remove_dir_all(&path).map_err(|error| io_at(&path, error))?;
    }
    fs::create_dir(&path).map_err(|error| io_at(&path, error))?;
    let mut staging = Staging {
        path,
        moved: Vec::new(),
        keep: false,
    };
    for (id, version) in write {
        let entry = catalog
            .get(id, *version)
            .expect("the plan names a catalog version");
        write_files(&staging.path.join(id), &entry.files)?;
    }
    if !write.is_empty() {
        let parts = root.join("parts");
        fs::create_dir_all(&parts).map_err(|error| io_at(&parts, error))?;
    }
    for (id, _) in write {
        let dest = root.join("parts").join(id);
        if dest.exists() {
            let aside = staging.path.join(format!("old-{id}"));
            fs::rename(&dest, &aside).map_err(|error| io_at(&dest, error))?;
            staging.moved.push((aside, dest.clone()));
        }
        let staged = staging.path.join(id);
        fs::rename(&staged, &dest).map_err(|error| io_at(&dest, error))?;
    }
    if let Some(text) = lock {
        let tmp = staging.path.join("parts.lock");
        fs::write(&tmp, text).map_err(|error| io_at(&tmp, error))?;
        let dest = root.join("parts.lock");
        fs::rename(&tmp, &dest).map_err(|error| io_at(&dest, error))?;
    }
    staging.keep = true;
    Ok(())
}

fn write_files(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), RunError> {
    fs::create_dir_all(dir).map_err(|error| io_at(dir, error))?;
    for (rel, bytes) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent()
            && parent != dir
        {
            fs::create_dir_all(parent).map_err(|error| io_at(parent, error))?;
        }
        fs::write(&path, bytes).map_err(|error| io_at(&path, error))?;
    }
    Ok(())
}

/// Folders moved aside live here until the new copies and the lock are in place. Drop puts a
/// folder back when this write stops before the new copy is renamed into place.
struct Staging {
    path: PathBuf,
    moved: Vec<(PathBuf, PathBuf)>,
    keep: bool,
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.keep {
            for (aside, dest) in self.moved.iter().rev() {
                if dest.exists() {
                    continue;
                }
                if fs::rename(aside, dest).is_err() {
                    return;
                }
            }
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn io_at(path: &Path, error: io::Error) -> RunError {
    RunError {
        message: format!("cannot access {}: {error}", path.display()),
    }
}

fn summarize_add(plan: AddPlan) -> AddReply {
    let mut parts: Vec<Copied> = plan
        .write
        .into_iter()
        .map(|(id, version)| Copied {
            id,
            version: version.to_string(),
            wrote: true,
        })
        .chain(plan.skip.into_iter().map(|(id, version)| Copied {
            id,
            version: version.to_string(),
            wrote: false,
        }))
        .collect();
    parts.sort_by(|a, b| a.id.cmp(&b.id));
    AddReply {
        parts,
        lock_written: plan.lock.is_some(),
        notes: plan.notes,
    }
}

fn add_text(reply: &AddReply) -> String {
    let mut lines: Vec<String> = reply
        .parts
        .iter()
        .map(|part| {
            let verb = if part.wrote { "wrote" } else { "skipped" };
            format!("{verb} parts/{} {}", part.id, part.version)
        })
        .collect();
    lines.push(if reply.lock_written {
        "wrote parts.lock".into()
    } else {
        "parts.lock unchanged".into()
    });
    lines.extend(reply.notes.iter().cloned());
    lines.join("\n")
}

fn diff_text(reply: &DiffReply) -> String {
    let mut lines = reply.notes.clone();
    if !reply.files.is_empty() {
        lines.push(reply.files.trim_end().to_string());
    }
    lines.join("\n")
}

fn relock_text(reply: &RelockReply) -> String {
    let mut lines = vec![if reply.lock_written {
        "wrote parts.lock".into()
    } else {
        "parts.lock unchanged".into()
    }];
    for id in &reply.modified {
        lines.push(format!("`{id}` is modified"));
    }
    lines.extend(reply.notes.iter().cloned());
    lines.join("\n")
}

fn refuse_text(refusal: &Refusal) -> String {
    let mut lines = refusal.message.lines();
    let first = lines.next().unwrap_or("");
    let mut text = format!("error[{}]: {first}", refusal.code);
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    if !refusal.hint.is_empty() {
        text.push_str("\n  hint: ");
        text.push_str(&refusal.hint);
    }
    text
}

fn refuse_json(refusal: &Refusal) -> Value {
    if refusal.hint.is_empty() {
        json!({
            "ok": false,
            "error": { "code": refusal.code, "message": refusal.message }
        })
    } else {
        json!({
            "ok": false,
            "error": { "code": refusal.code, "message": refusal.message, "hint": refusal.hint }
        })
    }
}

fn versions(parts: &[Copied], wrote: bool) -> Vec<Value> {
    parts
        .iter()
        .filter(|part| part.wrote == wrote)
        .map(|part| json!({ "id": part.id, "version": part.version }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    fn files(body: &str) -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        files.insert("part.jsonc".into(), body.as_bytes().to_vec());
        files
    }

    fn leaf(id: &str, version: &str, title: &str) -> CatalogEntry {
        catalog::entry_from_files(
            id,
            files(&format!(
                "{{\n  \"format\": 1,\n  \"part\": \"{id}\",\n  \"version\": \"{version}\",\n  \"title\": \"{title}\",\n  \"model\": {{ \"kind\": \"x\" }}\n}}\n"
            )),
        )
        .unwrap()
    }

    fn board(id: &str, version: &str, uses: &[(&str, &str)]) -> CatalogEntry {
        let uses = uses
            .iter()
            .map(|(child, version)| format!("\"{child}\": \"{version}\""))
            .collect::<Vec<_>>()
            .join(", ");
        catalog::entry_from_files(
            id,
            files(&format!(
                "{{\n  \"format\": 1,\n  \"part\": \"{id}\",\n  \"version\": \"{version}\",\n  \"uses\": {{ {uses} }},\n  \"parts\": {{}}\n}}\n"
            )),
        )
        .unwrap()
    }

    fn catalog_of(entries: Vec<CatalogEntry>) -> Catalog {
        let mut catalog = Catalog::official();
        for entry in entries {
            catalog.insert(entry);
        }
        catalog
    }

    fn empty() -> Snapshot {
        Snapshot {
            folders: BTreeMap::new(),
            lock: BTreeMap::new(),
            lock_text: None,
            lock_error: None,
        }
    }

    fn with_folder(entry: &CatalogEntry) -> Snapshot {
        let mut snapshot = empty();
        snapshot.folders.insert(
            entry.id.clone(),
            FolderSnap::from_files(entry.files.clone()),
        );
        snapshot
    }

    fn ask(ids: &[&str], overwrite: bool, diff: bool) -> Request {
        Request {
            ids: ids.iter().map(|id| (*id).to_string()).collect(),
            overwrite,
            diff,
            relock: false,
        }
    }

    fn refusal(plan: Plan) -> Refusal {
        match plan {
            Plan::Refuse(refusal) => refusal,
            other => panic!("expected a refusal, got {}", name(&other)),
        }
    }

    fn added(plan: Plan) -> AddPlan {
        match plan {
            Plan::Add(plan) => plan,
            other => panic!("expected a write plan, got {}", name(&other)),
        }
    }

    fn name(plan: &Plan) -> &'static str {
        match plan {
            Plan::Add(_) => "add",
            Plan::Diff(_) => "diff",
            Plan::Relock(_) => "relock",
            Plan::Refuse(_) => "refuse",
            Plan::Usage(_) => "usage",
        }
    }

    fn wrote(plan: &AddPlan, id: &str) -> Option<Version> {
        plan.write
            .iter()
            .find(|(written, _)| written == id)
            .map(|(_, version)| *version)
    }

    #[test]
    fn closure_is_copied_and_a_second_plan_writes_nothing() {
        let child = leaf("led", "0.1.0", "lamp");
        let parent = board("board", "0.1.0", &[("led", "0.1.0")]);
        let catalog = catalog_of(vec![child.clone(), parent.clone()]);
        let first = added(plan(&catalog, &empty(), &ask(&["board"], false, false)));
        assert_eq!(wrote(&first, "board"), Some(parent.version));
        assert_eq!(wrote(&first, "led"), Some(child.version));
        let text = first.lock.clone().unwrap();
        assert_eq!(
            text,
            format!(
                "board 0.1.0 {} {}\nled 0.1.0 {} {}\n",
                catalog.registry(),
                parent.hash,
                catalog.registry(),
                child.hash
            )
        );

        let mut again = empty();
        again
            .folders
            .insert("board".into(), FolderSnap::from_files(parent.files));
        again
            .folders
            .insert("led".into(), FolderSnap::from_files(child.files));
        again.lock_text = Some(text);
        let second = added(plan(&catalog, &again, &ask(&["board"], false, false)));
        assert!(second.write.is_empty());
        assert!(second.lock.is_none());
        assert_eq!(second.skip.len(), 2);
    }

    #[test]
    fn a_local_edit_refuses_and_names_the_file() {
        let part = leaf("led", "0.1.0", "lamp");
        let catalog = catalog_of(vec![part.clone()]);
        let mut folder = part.files.clone();
        let edited = folder.get("part.jsonc").unwrap().clone();
        let edited = String::from_utf8(edited).unwrap().replace("lamp", "edited");
        folder.insert("part.jsonc".into(), edited.into_bytes());
        let mut snapshot = empty();
        snapshot
            .folders
            .insert("led".into(), FolderSnap::from_files(folder));
        let refusal = refusal(plan(&catalog, &snapshot, &ask(&["led"], false, false)));
        assert_eq!(refusal.code, "part_differs");
        assert!(
            refusal.message.contains("`part.jsonc`"),
            "{}",
            refusal.message
        );
        assert_eq!(refusal.hint, "run `orbe add led --overwrite`");
    }

    #[test]
    fn overwrite_across_a_major_version_needs_the_version_named() {
        let old = leaf("led", "0.1.0", "lamp");
        let new = leaf("led", "1.0.0", "lamp v2");
        let catalog = catalog_of(vec![old.clone(), new.clone()]);
        let snapshot = with_folder(&old);
        let refused = refusal(plan(
            &catalog,
            &snapshot,
            &Request {
                ids: vec!["led".into()],
                overwrite: true,
                diff: false,
                relock: false,
            },
        ));
        assert_eq!(refused.hint, "run `orbe add led@1.0.0 --overwrite`");
        assert!(refused.message.contains("nothing was written"));

        let planned = added(plan(&catalog, &snapshot, &ask(&["led@1.0.0"], true, false)));
        assert_eq!(wrote(&planned, "led"), Some(new.version));
        assert!(planned.notes.iter().any(|note| note == TESTS_MAY_CHANGE));
    }

    #[test]
    fn two_uses_of_one_id_stop_until_the_clash_is_named() {
        let old = leaf("r", "0.1.0", "old");
        let newer = leaf("r", "0.2.0", "newer");
        let major = leaf("r", "1.0.0", "major");
        let nano = board("nano", "0.1.0", &[("r", "0.1.0")]);
        let uno = board("uno", "0.1.0", &[("r", "0.2.0")]);
        let mega = board("mega", "0.1.0", &[("r", "1.0.0")]);
        let catalog = catalog_of(vec![
            old.clone(),
            newer.clone(),
            major.clone(),
            nano,
            uno,
            mega,
        ]);
        let refused = refusal(plan(
            &catalog,
            &empty(),
            &ask(&["nano", "uno"], false, false),
        ));
        assert_eq!(refused.code, "part_conflict");
        assert_eq!(
            refused.message,
            "`r` is `0.1.0` in `nano` and `0.2.0` in `uno`"
        );
        assert_eq!(refused.hint, "run `orbe add nano uno --overwrite`");

        let planned = added(plan(
            &catalog,
            &empty(),
            &ask(&["nano", "uno"], true, false),
        ));
        assert_eq!(wrote(&planned, "r"), Some(newer.version));
        assert!(
            planned
                .notes
                .iter()
                .any(|note| note.contains("the newer of")),
            "{:?}",
            planned.notes
        );

        let refused = refusal(plan(
            &catalog,
            &empty(),
            &ask(&["nano", "mega"], true, false),
        ));
        assert!(refused.message.contains("`nano`"), "{}", refused.message);
        assert!(refused.message.contains("`mega`"), "{}", refused.message);
        assert!(refused.message.contains("0.1.0"), "{}", refused.message);
        assert!(refused.message.contains("1.0.0"), "{}", refused.message);
        assert!(refused.hint.contains("r@0.1.0"), "{}", refused.hint);
        assert!(refused.hint.contains("r@1.0.0"), "{}", refused.hint);
        assert!(refused.hint.contains("--overwrite"), "{}", refused.hint);

        let planned = added(plan(
            &catalog,
            &empty(),
            &ask(&["nano", "mega", "r@1.0.0"], true, false),
        ));
        assert_eq!(wrote(&planned, "r"), Some(major.version));
        assert!(planned.notes.iter().any(|note| note == TESTS_MAY_CHANGE));

        let refused = refusal(plan(
            &catalog,
            &empty(),
            &ask(&["nano", "mega", "r@1.0.0"], false, false),
        ));
        assert_eq!(refused.code, "part_conflict");
    }

    #[test]
    fn an_unknown_id_suggests_a_catalog_id_and_a_registry_id_is_refused() {
        let catalog = catalog_of(vec![leaf("led-5mm", "0.1.0", "lamp")]);
        let refused = refusal(plan(&catalog, &empty(), &ask(&["led-5m"], false, false)));
        assert_eq!(refused.code, "unknown_part");
        assert!(refused.hint.contains("led-5mm"), "{}", refused.hint);
        let refused = refusal(plan(
            &catalog,
            &empty(),
            &ask(&["@acme/led-5mm"], false, false),
        ));
        assert_eq!(refused.code, "registry");
        assert!(
            refused
                .message
                .contains("v1 reads only the official catalog"),
            "{}",
            refused.message
        );
    }

    #[test]
    fn diff_states_whether_a_test_can_change_and_writes_nothing() {
        let patch_old = leaf("led", "0.1.0", "lamp");
        let patch = leaf("led", "0.1.1", "lamp patched");
        let minor = leaf("led", "0.2.0", "lamp minor");
        let major = leaf("led", "1.0.0", "lamp major");
        let catalog = catalog_of(vec![patch_old.clone(), patch, minor, major]);
        let snapshot = with_folder(&patch_old);

        let diff = match plan(&catalog, &snapshot, &ask(&["led@0.1.1"], false, true)) {
            Plan::Diff(diff) => diff,
            other => panic!("{}", name(&other)),
        };
        assert!(
            diff.notes
                .iter()
                .any(|note| note.contains("a patch version cannot change a passing test")),
            "{:?}",
            diff.notes
        );
        assert!(
            diff.files.contains("-  \"title\": \"lamp\""),
            "{}",
            diff.files
        );
        assert!(
            diff.files.contains("+  \"title\": \"lamp patched\""),
            "{}",
            diff.files
        );

        let diff = match plan(&catalog, &snapshot, &ask(&["led@0.2.0"], false, true)) {
            Plan::Diff(diff) => diff,
            other => panic!("{}", name(&other)),
        };
        assert!(
            diff.notes
                .iter()
                .any(|note| note.contains("a minor version cannot change a passing test")),
            "{:?}",
            diff.notes
        );

        let diff = match plan(&catalog, &snapshot, &ask(&["led@1.0.0"], false, true)) {
            Plan::Diff(diff) => diff,
            other => panic!("{}", name(&other)),
        };
        assert!(
            diff.notes
                .iter()
                .any(|note| note.contains("a major version may change a passing test")),
            "{:?}",
            diff.notes
        );
    }

    #[test]
    fn relock_refuses_a_sealed_edit_and_records_a_composite_edit() {
        let sealed = leaf("led", "0.1.0", "lamp");
        let open = board("board", "0.1.0", &[]);
        let catalog = catalog_of(vec![sealed.clone(), open.clone()]);

        let mut led_files = sealed.files.clone();
        let text = String::from_utf8(led_files["part.jsonc"].clone())
            .unwrap()
            .replace("lamp", "edited");
        led_files.insert("part.jsonc".into(), text.into_bytes());
        let mut snapshot = empty();
        snapshot
            .folders
            .insert("led".into(), FolderSnap::from_files(led_files));
        let refused = refusal(plan(
            &catalog,
            &snapshot,
            &Request {
                ids: vec![],
                overwrite: false,
                diff: false,
                relock: true,
            },
        ));
        assert_eq!(refused.code, "part_modified");
        assert!(
            refused.message.contains("`part.jsonc`"),
            "{}",
            refused.message
        );
        assert!(
            refused.hint.contains("orbe add led --overwrite"),
            "{}",
            refused.hint
        );

        let mut board_files = open.files.clone();
        board_files.insert("note.txt".into(), b"local".to_vec());
        let folder = FolderSnap::from_files(board_files);
        let hash = folder.hash.clone();
        assert_ne!(hash, open.hash);
        let mut snapshot = with_folder(&sealed);
        snapshot.folders.insert("board".into(), folder);
        snapshot.lock_text = Some("mangled\n".into());
        let planned = match plan(
            &catalog,
            &snapshot,
            &Request {
                ids: vec![],
                overwrite: false,
                diff: false,
                relock: true,
            },
        ) {
            Plan::Relock(plan) => plan,
            other => panic!("{}", name(&other)),
        };
        assert_eq!(planned.modified, vec!["board".to_string()]);
        let text = planned.lock.unwrap();
        assert!(text.contains(&hash), "{text}");
        assert!(text.contains(&sealed.hash), "{text}");
        assert!(text.starts_with("board "), "{text}");
    }

    #[test]
    fn a_bad_version_token_is_usage() {
        let catalog = catalog_of(vec![leaf("led", "0.1.0", "lamp")]);
        match plan(&catalog, &empty(), &ask(&["led@1"], false, false)) {
            Plan::Usage(message) => assert!(message.contains("three numbers"), "{message}"),
            other => panic!("{}", name(&other)),
        }
    }

    #[test]
    fn run_copies_a_real_part_and_a_second_run_keeps_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let request = Request {
            ids: vec!["led-5mm".into()],
            overwrite: false,
            diff: false,
            relock: false,
        };
        let reply = run(dir.path(), &request).unwrap();
        assert!(
            reply.text().contains("wrote parts/led-5mm 0.1.0"),
            "{}",
            reply.text()
        );
        let lock = fs::read(dir.path().join("parts.lock")).unwrap();
        let part = fs::read(dir.path().join("parts/led-5mm/part.jsonc")).unwrap();
        let again = run(dir.path(), &request).unwrap();
        assert!(
            again.text().contains("skipped parts/led-5mm 0.1.0"),
            "{}",
            again.text()
        );
        assert!(
            again.text().contains("parts.lock unchanged"),
            "{}",
            again.text()
        );
        assert_eq!(fs::read(dir.path().join("parts.lock")).unwrap(), lock);
        assert_eq!(
            fs::read(dir.path().join("parts/led-5mm/part.jsonc")).unwrap(),
            part
        );
        assert!(!dir.path().join(".orbe-add-staging").exists());
    }

    #[test]
    fn an_interrupted_add_puts_the_old_folder_back() {
        let dir = tempfile::tempdir().unwrap();
        let request = Request {
            ids: vec!["led-5mm".into()],
            overwrite: false,
            diff: false,
            relock: false,
        };
        run(dir.path(), &request).unwrap();
        let part = dir.path().join("parts/led-5mm");
        let staging = dir.path().join(".orbe-add-staging");
        fs::create_dir_all(&staging).unwrap();
        fs::rename(&part, staging.join("old-led-5mm")).unwrap();
        let reply = run(dir.path(), &request).unwrap();
        assert!(part.join("part.jsonc").is_file());
        assert!(
            reply
                .text()
                .contains("restored parts/led-5mm from an interrupted add"),
            "{}",
            reply.text()
        );
        assert!(
            reply.text().contains("skipped parts/led-5mm 0.1.0"),
            "{}",
            reply.text()
        );
        assert!(!staging.exists());
    }
}
