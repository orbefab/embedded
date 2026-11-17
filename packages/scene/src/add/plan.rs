//! What `orbe add` would do, as a pure function of the request, the catalog and a snapshot of
//! the project. It names the folders to write and the lock to write, or it refuses.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{Catalog, CatalogEntry, Version};
use crate::name::did_you_mean;
use crate::project::{self, LockFields};

use super::diff::diff_files;
use super::snapshot::{Folder, Snapshot, Target};
use super::{AddCode, Refusal, Request};

const TESTS_MAY_CHANGE: &str = "tests may change";

pub(super) enum Plan {
    Add(AddPlan),
    Diff(DiffPlan),
    Relock(RelockPlan),
    Refuse(Refusal),
    Usage(String),
}

pub(super) struct AddPlan {
    pub(super) write: Vec<Write>,
    pub(super) skip: Vec<(String, Version)>,
    pub(super) lock: Option<String>,
    pub(super) notes: Vec<String>,
}

/// A folder to write. `replacing` says what was at `parts/<id>` before, for the reply.
pub(super) struct Write {
    pub(super) id: String,
    pub(super) version: Version,
    pub(super) replacing: Option<String>,
}

pub(super) struct DiffPlan {
    pub(super) notes: Vec<String>,
    pub(super) files: String,
}

pub(super) struct RelockPlan {
    pub(super) lock: Option<String>,
    pub(super) parts: Vec<String>,
    pub(super) modified: Vec<String>,
}

struct Spec {
    id: String,
    version: Option<Version>,
    raw: String,
}

/// Who asked for a version of an id.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum By {
    Request,
    /// A part this add copies, through its `uses`.
    Part(String),
    /// A part already in the project, through the `uses` of its lock line's version.
    Project(String),
}

struct Demand {
    version: Version,
    by: By,
}

/// The usage error in a request, which no project state changes.
pub(super) fn usage(request: &Request) -> Option<String> {
    if request.relock {
        return (!request.ids.is_empty() || request.diff || request.overwrite)
            .then(|| "orbe add --relock takes no part ids, --diff, or --overwrite".to_string());
    }
    if request.ids.is_empty() {
        return Some("orbe add takes at least one part id, such as `orbe add led-5mm`".into());
    }
    parse_specs(&request.ids).err()
}

/// The ids whose `parts/<id>` the plan may write or compare: every catalog id the request
/// can reach through `uses`, at any version. `--relock` compares every catalog id.
pub(super) fn wanted(catalog: &Catalog, request: &Request) -> BTreeSet<String> {
    if request.relock {
        return catalog.ids().into_iter().map(str::to_string).collect();
    }
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
    if request.relock {
        return plan_relock(catalog, snapshot, add);
    }
    let specs = parse_specs(&request.ids).expect("usage checked the ids");
    if let Some(plan) = unknown_specs(catalog, &specs) {
        return plan;
    }
    if let Some(message) = snapshot.lock_error.clone() {
        return Plan::Refuse(Refusal {
            code: AddCode::LockSyntax,
            message,
            hint: format!("run `{add} --relock`"),
        });
    }
    let selection = match select(catalog, snapshot, &specs, request.overwrite, add) {
        Ok(selection) => selection,
        Err(plan) => return plan,
    };
    if request.diff {
        return plan_diff(catalog, snapshot, &selection);
    }
    plan_add(
        catalog,
        snapshot,
        &specs,
        &selection,
        request.overwrite,
        add,
    )
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
        Err(bad.join("\n"))
    }
}

/// `@registry/id` is a real id shape, and v1 has only the official catalog. Other missing
/// ids get a suggestion from this catalog.
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
            lines.push(format!("no catalog part named `{id}`"));
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
            "the catalog has no part with that id".into()
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
            code: AddCode::UnknownVersion,
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

/// An id named without a version takes the newest version within the major version of the
/// project's copy, so a plain `--overwrite` never crosses one. With no copy, the newest.
fn default_version(catalog: &Catalog, snapshot: &Snapshot, id: &str) -> Version {
    let newest = catalog
        .newest(id)
        .expect("unknown ids are refused first")
        .version;
    snapshot
        .folder(id)
        .and_then(|folder| folder.version)
        .and_then(|copy| catalog.newest_in_major(id, copy.major))
        .map_or(newest, |entry| entry.version)
}

fn select(
    catalog: &Catalog,
    snapshot: &Snapshot,
    specs: &[Spec],
    overwrite: bool,
    add: &str,
) -> Result<Selection, Plan> {
    let mut queue = Vec::new();
    for spec in specs {
        let version = spec
            .version
            .unwrap_or_else(|| default_version(catalog, snapshot, &spec.id));
        queue.push((spec.id.clone(), version, By::Request));
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
                code: AddCode::UnknownVersion,
                message: format!("the catalog has no `{id}@{version}`"),
                hint: String::new(),
            })
        })?;
        for (child, child_version) in &entry.uses {
            if catalog.get(child, *child_version).is_none() {
                return Err(Plan::Refuse(Refusal {
                    code: AddCode::UnknownVersion,
                    message: format!(
                        "`{id}` uses `{child}@{child_version}`, and the catalog has no `{child}@{child_version}`"
                    ),
                    hint: String::new(),
                }));
            }
            queue.push((child.clone(), *child_version, By::Part(id.clone())));
        }
    }
    project_demands(catalog, snapshot, &mut demands);

    let mut chosen = BTreeMap::new();
    let mut notes = Vec::new();
    let mut tests_may_change = false;
    let mut clashes = Vec::new();
    let mut kept = BTreeSet::new();
    for (id, group) in &demands {
        let demanded = group
            .iter()
            .map(|demand| demand.version)
            .max()
            .expect("a demand");
        // A lock-matching copy newer than every version this add would write is not
        // replaced, named or pinned. The folder differs from its lock line, or the line
        // is from another registry: that stays an overwrite, as before.
        if let Some(held) = lock_matching_newer(catalog, snapshot, id, demanded) {
            notes.push(format!(
                "left `{id}` at {held}. this project needs a newer orbe for it"
            ));
            kept.insert(id.clone());
            continue;
        }
        let held = snapshot
            .lock
            .get(id)
            .and_then(|line| Version::parse(&line.version));
        match choose(id, group, specs, overwrite, held) {
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
        let mut message = clashes
            .iter()
            .map(|clash| clash.message.clone())
            .collect::<Vec<_>>()
            .join("\n");
        if !notes.is_empty() {
            message.push('\n');
            message.push_str(&notes.join("\n"));
        }
        return Err(Plan::Refuse(Refusal {
            code: AddCode::PartConflict,
            message,
            hint: clash_hint(specs, &clashes, add, &kept),
        }));
    }
    Ok(Selection {
        chosen,
        notes,
        tests_may_change,
    })
}

/// The version of `id` in the project when the folder matches its own official lock line
/// and that version is newer than `than`. This binary would otherwise write `than` or
/// older over it. A folder that does not match the lock line is the user's edit. A line
/// from another registry is not this copy.
fn lock_matching_newer(
    catalog: &Catalog,
    snapshot: &Snapshot,
    id: &str,
    than: Version,
) -> Option<Version> {
    let folder = snapshot.folder(id)?;
    let line = snapshot.lock.get(id)?;
    if line.registry != catalog.registry() || !folder.is(&line.hash) {
        return None;
    }
    let held = Version::parse(&line.version)?;
    (folder.version == Some(held) && held > than).then_some(held)
}

/// A part already in the project keeps needing the versions its `uses` names. Its lock line
/// gives its version, and the catalog gives that version's `uses`. Only ids this add touches
/// get a demand, and a part this add copies again speaks through its new version instead.
fn project_demands(
    catalog: &Catalog,
    snapshot: &Snapshot,
    demands: &mut BTreeMap<String, Vec<Demand>>,
) {
    let touched: BTreeSet<String> = demands.keys().cloned().collect();
    for line in snapshot.lock.values() {
        if touched.contains(&line.id) {
            continue;
        }
        let Some(entry) = Version::parse(&line.version).and_then(|v| catalog.get(&line.id, v))
        else {
            continue;
        };
        for (child, version) in &entry.uses {
            if let Some(group) = demands.get_mut(child) {
                group.push(Demand {
                    version: *version,
                    by: By::Project(line.id.clone()),
                });
            }
        }
    }
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

/// `held` is the version of `id` in the project's lock. When it is already the newer of two
/// versions in one major version, an earlier `--overwrite` settled the clash, and it stays
/// settled, unless the request names a version of `id`.
fn choose(
    id: &str,
    group: &[Demand],
    specs: &[Spec],
    overwrite: bool,
    held: Option<Version>,
) -> Choose {
    let mut versions: Vec<Version> = group.iter().map(|demand| demand.version).collect();
    versions.sort();
    versions.dedup();
    if versions.len() == 1 {
        return Choose::Take(versions[0]);
    }
    let same_major = versions
        .windows(2)
        .all(|pair| pair[0].major == pair[1].major);
    let newer = *versions.last().expect("at least two versions");
    if same_major && overwrite {
        let note = format!("took `{id}` {newer}, the newer of {}", describe(group));
        return Choose::Newer(newer, note);
    }
    let named = specs
        .iter()
        .any(|spec| spec.id == id && spec.version.is_some());
    if same_major && held == Some(newer) && !named {
        let note = format!(
            "kept `{id}` {newer}, which the project holds, the newer of {}",
            describe(group)
        );
        return Choose::Newer(newer, note);
    }
    if !same_major
        && overwrite
        && let Some(named) = explicit_version(specs, id)
    {
        return Choose::Named(named);
    }
    Choose::Clash(Clash {
        message: format!("`{id}` is {}", describe(group)),
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
            let mut bys: Vec<&By> = group
                .iter()
                .filter(|demand| demand.version == version)
                .map(|demand| &demand.by)
                .collect();
            bys.sort();
            bys.dedup();
            let who = join_and(&bys.iter().map(|by| who(by)).collect::<Vec<_>>());
            format!("`{version}` in {who}")
        })
        .collect();
    join_and(&phrases)
}

fn who(by: &By) -> String {
    match by {
        By::Request => "the request".into(),
        By::Part(id) => format!("`{id}`"),
        By::Project(id) => format!("`{id}`, which the project holds"),
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

fn clash_hint(specs: &[Spec], clashes: &[Clash], add: &str, leave: &BTreeSet<String>) -> String {
    let cross: Vec<&Clash> = clashes.iter().filter(|clash| clash.cross_major).collect();
    if cross.is_empty() {
        let command = command_for(add, specs, &[], true, leave).or_else(|| {
            let specs = clashes
                .iter()
                .filter(|clash| !leave.contains(&clash.id))
                .map(|clash| clash.id.clone())
                .collect::<Vec<_>>();
            project::add_writing(add, &specs, true)
        });
        return run_hint(command);
    }
    if cross.len() == 1 {
        let clash = cross[0];
        let commands: Vec<String> = clash
            .versions
            .iter()
            .filter_map(|version| {
                command_for(add, specs, &[(clash.id.clone(), *version)], true, leave)
            })
            .map(|command| format!("`{command}`"))
            .collect();
        if commands.is_empty() {
            return "this project needs a newer orbe".into();
        }
        return format!("run {}", join_or(&commands));
    }
    "name one version of each id with `@<version> --overwrite`".into()
}

fn run_hint(command: Option<String>) -> String {
    match command {
        Some(command) => format!("run `{command}`"),
        None => "this project needs a newer orbe".into(),
    }
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

/// The ids a clash command names: the request's ids, without a lock-matching copy newer
/// than this binary, with `id@version` where `pins` says the major version has to be named.
/// Empty is not a command.
fn command_for(
    add: &str,
    specs: &[Spec],
    pins: &[(String, Version)],
    overwrite: bool,
    leave: &BTreeSet<String>,
) -> Option<String> {
    let mut args = Vec::new();
    let mut pending: BTreeSet<&str> = pins
        .iter()
        .filter(|(id, _)| !leave.contains(id))
        .map(|(id, _)| id.as_str())
        .collect();
    for spec in specs {
        if leave.contains(&spec.id) {
            continue;
        }
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
    project::add_writing(add, &args, overwrite)
}

/// How `parts/<id>` differs from the version the add would write.
struct Differ {
    line: String,
    /// What the reply says it replaced, under `--overwrite`.
    was: String,
    /// The copy is in another major version, so `--overwrite` needs the version named.
    crosses: bool,
}

fn plan_add(
    catalog: &Catalog,
    snapshot: &Snapshot,
    specs: &[Spec],
    selection: &Selection,
    overwrite: bool,
    add: &str,
) -> Plan {
    let explicit: BTreeSet<&str> = specs
        .iter()
        .filter(|spec| spec.version.is_some())
        .map(|spec| spec.id.as_str())
        .collect();
    let mut write = Vec::new();
    let mut skip = Vec::new();
    let mut refused: Vec<(String, Version, Differ)> = Vec::new();
    let mut hint_specs = Vec::new();
    let mut notes = selection.notes.clone();
    let mut tests_may_change = selection.tests_may_change;
    for (id, &version) in &selection.chosen {
        let entry = catalog
            .get(id, version)
            .expect("select only keeps catalog versions");
        let lock_hash = snapshot.lock.get(id).map(|line| line.hash.as_str());
        let Some(differ) = differ(catalog, snapshot.target(id), entry, lock_hash) else {
            if matches!(snapshot.target(id), Target::Absent) {
                write.push(Write {
                    id: id.clone(),
                    version,
                    replacing: None,
                });
                hint_specs.push(id.clone());
            } else {
                skip.push((id.clone(), version));
                // The folder is the catalog's, but the lock line may still name another
                // registry or version. The command that finishes this add has to name it.
                if !lock_matches(catalog, snapshot, id, entry) {
                    hint_specs.push(id.clone());
                }
            }
            continue;
        };
        let named = explicit.contains(id.as_str());
        let spec = if differ.crosses {
            format!("{id}@{version}")
        } else {
            id.clone()
        };
        if overwrite && (!differ.crosses || named) {
            tests_may_change |= differ.crosses;
            write.push(Write {
                id: id.clone(),
                version,
                replacing: Some(differ.was),
            });
            hint_specs.push(spec);
            continue;
        }
        refused.push((id.clone(), version, differ));
        hint_specs.push(spec);
    }
    if !refused.is_empty() {
        let lines: Vec<&str> = refused
            .iter()
            .map(|(_, _, differ)| differ.line.as_str())
            .collect();
        let mut message = format!("nothing was written\n{}", lines.join("\n"));
        if !notes.is_empty() {
            message.push('\n');
            message.push_str(&notes.join("\n"));
        }
        // The same ids this loop would write, including a child the request did not name.
        // A kept newer copy is not among them. It stays in the note above.
        let hint = match project::add_writing(add, &hint_specs, true) {
            Some(command) => format!("run `{command}`"),
            None if notes.iter().any(|note| note.contains("newer orbe")) => {
                "this project needs a newer orbe".into()
            }
            None => String::new(),
        };
        return Plan::Refuse(Refusal {
            code: AddCode::PartDiffers,
            message,
            hint,
        });
    }
    for spec in specs.iter().filter(|spec| spec.version.is_none()) {
        let Some(&chosen) = selection.chosen.get(&spec.id) else {
            continue;
        };
        let newest = catalog.newest(&spec.id).expect("a catalog id").version;
        if newest.major != chosen.major {
            let command = project::add_writing(add, &[format!("{}@{newest}", spec.id)], true)
                .expect("the id is named");
            notes.push(format!(
                "the catalog also has `{id}` {newest}, a new major version. `{command}` takes it, and tests may change",
                id = spec.id
            ));
        }
    }
    if tests_may_change {
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

/// The lock line already records the catalog copy `entry`, so writing it would change nothing.
fn lock_matches(catalog: &Catalog, snapshot: &Snapshot, id: &str, entry: &CatalogEntry) -> bool {
    snapshot.lock.get(id).is_some_and(|line| {
        line.version == entry.version.to_string()
            && line.registry == catalog.registry()
            && line.hash == entry.hash
    })
}

/// `None` when `parts/<id>` is absent or already holds exactly `entry`.
fn differ(
    catalog: &Catalog,
    target: &Target,
    entry: &CatalogEntry,
    lock_hash: Option<&str>,
) -> Option<Differ> {
    let id = &entry.id;
    let folder = match target {
        Target::Absent => return None,
        Target::Other(what) => {
            return Some(Differ {
                line: format!("parts/{id} is {what}, not a part folder"),
                was: (*what).to_string(),
                crosses: false,
            });
        }
        Target::Folder(folder) if folder.is(&entry.hash) => return None,
        Target::Folder(folder) => folder,
    };
    if !folder.has_part_file() {
        return Some(Differ {
            line: format!("parts/{id} is a folder with no part.jsonc"),
            was: "a folder with no part.jsonc".into(),
            crosses: false,
        });
    }
    let Some(copy) = folder.version else {
        return Some(Differ {
            line: format!(
                "`{id}` has no version in the project, and differs in {}",
                differing_names(folder, entry)
            ),
            was: "a copy with no version".into(),
            crosses: false,
        });
    };
    let crosses = copy.major != entry.version.major;
    let original = catalog.get(id, copy);
    let clean = original.is_some_and(|original| folder.is(&original.hash));
    // A folder that matches its lock line is the version the lock records, even when this
    // binary does not carry that version. "with local edits" is only a folder the lock
    // does not hash.
    let matches_lock = lock_hash.is_some_and(|hash| folder.is(hash));
    let locked = clean || matches_lock;
    let edits = differing_names(folder, original.unwrap_or(entry));
    let was = if locked {
        copy.to_string()
    } else {
        format!("{copy} with local edits")
    };
    let line = match (copy == entry.version, locked) {
        (true, _) => format!("`{id}` is {copy} in the project and differs in {edits}"),
        (false, true) => format!(
            "`{id}` is {copy} in the project, {} in the catalog",
            entry.version
        ),
        (false, false) => format!(
            "`{id}` is {copy} in the project, {} in the catalog, and the copy differs from {copy} in {edits}",
            entry.version
        ),
    };
    Some(Differ { line, was, crosses })
}

fn differing_names(folder: &Folder, entry: &CatalogEntry) -> String {
    let mut names = Vec::new();
    for (rel, bytes) in &entry.files {
        match folder.files.get(rel) {
            Some(copy) if copy == bytes => {}
            Some(_) => names.push(format!("`{rel}`")),
            None if folder
                .odd
                .iter()
                .any(|odd| odd.starts_with(&format!("`{rel}`"))) => {}
            None => names.push(format!("`{rel}`, which is missing")),
        }
    }
    for rel in folder.files.keys() {
        if !entry.files.contains_key(rel) {
            names.push(format!("`{rel}`, which the catalog's copy does not have"));
        }
    }
    names.extend(folder.odd.iter().cloned());
    if names.is_empty() {
        "the folder".into()
    } else {
        names.join(", ")
    }
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
    for (id, &version) in &selection.chosen {
        let entry = catalog.get(id, version).expect("chosen version exists");
        match snapshot.target(id) {
            Target::Folder(folder) if folder.is(&entry.hash) => {}
            Target::Folder(folder) => {
                notes.push(change_note(id, folder.version, version));
                files.push_str(&diff_files(id, Some(&folder.files), entry));
            }
            Target::Absent => {
                notes.push(change_note(id, None, version));
                files.push_str(&diff_files(id, None, entry));
            }
            Target::Other(what) => {
                notes.push(format!(
                    "parts/{id} is {what}. `--overwrite` replaces it with `{id}` {version}"
                ));
                files.push_str(&diff_files(id, None, entry));
            }
        }
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

fn plan_relock(catalog: &Catalog, snapshot: &Snapshot, add: &str) -> Plan {
    let mut lines = BTreeMap::new();
    let mut modified = Vec::new();
    let mut problems = Vec::new();
    let mut sealed = Vec::new();
    for (id, target) in &snapshot.targets {
        let Target::Folder(folder) = target else {
            continue;
        };
        let Some(version) = folder.version else {
            problems.push(format!("`{id}` has no version"));
            continue;
        };
        let Some(entry) = catalog.get(id, version) else {
            problems.push(format!("the catalog has no `{id}@{version}`"));
            continue;
        };
        let clean = folder.is(&entry.hash);
        if !folder.odd.is_empty() {
            problems.push(format!(
                "parts/{id} holds {}, which a lock line cannot cover",
                folder.odd.join(", ")
            ));
            sealed.push(id.clone());
            continue;
        }
        if !clean && entry.sealed {
            problems.push(format!(
                "sealed part `{id}` differs from `{id}@{version}` in {}",
                differing_names(folder, entry)
            ));
            sealed.push(id.clone());
            continue;
        }
        let hash = if clean {
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
        let hint = sealed
            .iter()
            .map(|id| {
                let command = project::add_writing(add, std::slice::from_ref(id), true)
                    .expect("the id is named");
                format!("restore it with `{command}`, or copy it to a path to make it your own")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let code = if sealed.is_empty() {
            AddCode::UnknownVersion
        } else {
            AddCode::PartModified
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

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
