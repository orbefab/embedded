//! `orbe add` decides what to copy, then writes it as one transaction.
//!
//! [`plan::plan`] is a pure function of the request, the catalog and a snapshot of the folders
//! it may write or compare. [`txn::apply`] is the only code that writes part folders or
//! `parts.lock`. It logs each step, and a run that stops part way is rolled back or finished by
//! the next run, which [`journal::next`] decides from the log and the hashes on disk. [`held`]
//! keeps a second run out while one is at work.

mod at;
mod diff;
mod held;
mod journal;
mod plan;
mod snapshot;
mod txn;

use std::io;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::catalog::Catalog;
use crate::project;

use held::Hold;
use plan::Plan;
#[cfg(test)]
use txn::Inject;
use txn::{Change, Fault, Pending, Stop};

/// What the CLI asked `orbe add` to do.
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
    /// What was at `parts/<id>` before this add replaced it.
    pub replaced: Option<String>,
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

/// A project file could not be read or written. The message says what was rolled back. The
/// CLI exits 3.
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

/// Copies parts into a project and writes `parts.lock`. A refusal writes nothing. `dir` is the
/// folder `-C` named, as the user wrote it, so hints can repeat it. With none, the current
/// folder must look like a project.
pub fn run(dir: Option<&Path>, request: &Request) -> Result<Reply, RunError> {
    run_with(dir, request, &mut |_| None)
}

fn run_with(dir: Option<&Path>, request: &Request, fault: &mut Fault) -> Result<Reply, RunError> {
    if let Some(message) = plan::usage(request) {
        return Ok(Reply::Usage(message));
    }
    if let Some(refusal) = not_a_project(dir, request) {
        return Ok(Reply::Refuse(refusal));
    }
    let root = dir.unwrap_or(Path::new("."));
    let add = project::add_command(dir);
    let again = request_command(&add, request);
    let _held = match held::hold(root) {
        Ok(held) => Some(held),
        Err(Hold::Busy) => return Ok(Reply::Refuse(busy(&again))),
        Err(Hold::Io(_)) if request.diff => None,
        Err(Hold::Io(error)) => return Err(error),
    };
    let recovered = if request.diff {
        txn::Recovered {
            lines: pending_note(
                root,
                &request_command(
                    &add,
                    &Request {
                        ids: request.ids.clone(),
                        diff: false,
                        ..*request
                    },
                ),
            ),
            planned: Vec::new(),
        }
    } else {
        txn::recover(root, &again, fault).map_err(stop_error)?
    };
    let catalog = Catalog::builtin();
    let snapshot = snapshot::load(root, &plan::wanted(&catalog, request))?;
    let reply = match plan::plan(&catalog, &snapshot, request, &add) {
        Plan::Add(planned) => {
            let folders = planned
                .write
                .iter()
                .map(|write| {
                    let entry = catalog
                        .get(&write.id, write.version)
                        .expect("the plan names a catalog version");
                    (write.id.as_str(), &entry.files)
                })
                .collect();
            let change = Change {
                folders,
                lock: planned.lock.as_deref(),
            };
            let mut notes = txn::apply(root, &change, &again, fault).map_err(stop_error)?;
            notes.extend(planned.notes);
            Reply::Add(add_reply(
                planned.write,
                planned.skip,
                planned.lock.is_some(),
                notes,
            ))
        }
        Plan::Relock(planned) => {
            let change = Change {
                folders: Vec::new(),
                lock: planned.lock.as_deref(),
            };
            let notes = txn::apply(root, &change, &again, fault).map_err(stop_error)?;
            Reply::Relock(RelockReply {
                lock_written: planned.lock.is_some(),
                parts: planned.parts,
                modified: planned.modified,
                notes,
            })
        }
        Plan::Diff(planned) => Reply::Diff(DiffReply {
            notes: planned.notes,
            files: planned.files,
        }),
        Plan::Refuse(refusal) => Reply::Refuse(refusal),
        Plan::Usage(message) => Reply::Usage(message),
    };
    let mut notes = recovered.lines;
    let holes = txn::lock_holes(root, &recovered.planned);
    if !holes.is_empty() {
        let folders = holes
            .iter()
            .map(|id| format!("parts/{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        let ids = holes
            .iter()
            .map(|id| project::shell_word(id))
            .collect::<Vec<_>>()
            .join(" ");
        notes.push(format!(
            "parts.lock lists {}, and {folders} {} missing. run `{add} {ids}` to copy {} back",
            holes.join(", "),
            if holes.len() == 1 { "is" } else { "are" },
            if holes.len() == 1 { "it" } else { "them" },
        ));
    }
    Ok(with_notes(reply, notes))
}

fn stop_error(stop: Stop) -> RunError {
    match stop {
        Stop::Failed(error) => error,
        Stop::Killed => RunError {
            message: "killed".into(),
        },
    }
}

fn busy(again: &str) -> Refusal {
    Refusal {
        code: "busy",
        message: "another `orbe add` is at work in this project, and holds its lock".into(),
        hint: format!("wait for it to finish, then run `{again}`"),
    }
}

/// `--diff` writes nothing, so an interrupted add stays for the next writing run.
fn pending_note(root: &Path, next: &str) -> Vec<String> {
    let what = match txn::pending(root) {
        Pending::None => return Vec::new(),
        Pending::Leftover(name) => {
            return vec![format!(
                "an interrupted add left {name}. --diff leaves it, and the next `{next}` without --diff deletes it"
            )];
        }
        Pending::Uncommitted => "rolls it back".to_string(),
        Pending::Committed => "finishes it".to_string(),
        Pending::Clearing => "finishes deleting it".to_string(),
        Pending::Stops(steps) => {
            let stops = "names what it cannot prove the add made, and stops";
            if steps.is_empty() {
                stops.to_string()
            } else {
                format!("{}, then {stops}", steps.join(", "))
            }
        }
    };
    vec![format!(
        "an interrupted add is pending in parts/{}. --diff leaves it, and the next `{next}` without --diff {what}",
        txn::STAGE
    )]
}

/// With no `-C`, the current folder must hold `scene.jsonc` or `parts.lock`. Anywhere else,
/// `orbe add` would make a second project, so it names the folder above that is one, if any.
/// A folder `-C` names is trusted, since `orbe check` names it for a scene of any name.
fn not_a_project(dir: Option<&Path>, request: &Request) -> Option<Refusal> {
    if let Some(dir) = dir {
        return (!dir.is_dir()).then(|| Refusal {
            code: "not_a_project",
            message: format!("`{}` is not a folder", dir.display()),
            hint: String::new(),
        });
    }
    let here = Path::new(".");
    if is_project(here) {
        return None;
    }
    let hint = match project_above(here) {
        Some(dir) => format!(
            "run `{}`",
            request_command(&project::add_command(Some(&dir)), request)
        ),
        None => "run it in the project's folder, the one with scene.jsonc, or name that folder with `orbe add -C <folder>`".into(),
    };
    Some(Refusal {
        code: "not_a_project",
        message:
            "the current folder is not an orbe project. it has no scene.jsonc and no parts.lock"
                .into(),
        hint,
    })
}

fn is_project(dir: &Path) -> bool {
    dir.join("scene.jsonc").is_file() || dir.join("parts.lock").is_file()
}

fn project_above(root: &Path) -> Option<PathBuf> {
    let mut dir = root.to_path_buf();
    loop {
        dir = up(&dir);
        if is_project(&dir) {
            return Some(dir);
        }
        dir.canonicalize().ok()?.parent()?;
    }
}

/// The folder above, written relative to the same place as `dir`.
fn up(dir: &Path) -> PathBuf {
    if dir == Path::new(".") {
        return PathBuf::from("..");
    }
    match dir.components().next_back() {
        Some(Component::Normal(_)) => match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        },
        _ => dir.join(".."),
    }
}

fn request_command(add: &str, request: &Request) -> String {
    let mut args = vec![add.to_string()];
    args.extend(request.ids.iter().map(|id| project::shell_word(id)));
    for (on, flag) in [
        (request.overwrite, "--overwrite"),
        (request.diff, "--diff"),
        (request.relock, "--relock"),
    ] {
        if on {
            args.push(flag.into());
        }
    }
    args.join(" ")
}

fn with_notes(reply: Reply, lines: Vec<String>) -> Reply {
    if lines.is_empty() {
        return reply;
    }
    match reply {
        Reply::Add(mut reply) => {
            reply.notes.splice(0..0, lines);
            Reply::Add(reply)
        }
        Reply::Diff(mut reply) => {
            reply.notes.splice(0..0, lines);
            Reply::Diff(reply)
        }
        Reply::Relock(mut reply) => {
            reply.notes.splice(0..0, lines);
            Reply::Relock(reply)
        }
        Reply::Refuse(mut refusal) => {
            refusal.message.push('\n');
            refusal.message.push_str(&lines.join("\n"));
            Reply::Refuse(refusal)
        }
        Reply::Usage(message) => Reply::Usage(format!("{message}\n{}", lines.join("\n"))),
    }
}

fn single_component(id: &str) -> bool {
    let mut parts = Path::new(id).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(name)), None) => name.to_str() == Some(id),
        _ => false,
    }
}

fn io_at(path: &Path, error: io::Error) -> RunError {
    RunError {
        message: format!(
            "cannot access {}: {error}",
            project::shell_path(path.as_os_str())
        ),
    }
}

fn add_reply(
    write: Vec<plan::Write>,
    skip: Vec<(String, crate::catalog::Version)>,
    lock_written: bool,
    notes: Vec<String>,
) -> AddReply {
    let mut parts: Vec<Copied> = write
        .into_iter()
        .map(|write| Copied {
            id: write.id,
            version: write.version.to_string(),
            wrote: true,
            replaced: write.replacing,
        })
        .chain(skip.into_iter().map(|(id, version)| Copied {
            id,
            version: version.to_string(),
            wrote: false,
            replaced: None,
        }))
        .collect();
    parts.sort_by(|a, b| a.id.cmp(&b.id));
    AddReply {
        parts,
        lock_written,
        notes,
    }
}

fn add_text(reply: &AddReply) -> String {
    let mut lines: Vec<String> = reply
        .parts
        .iter()
        .map(|part| match (&part.replaced, part.wrote) {
            (Some(was), _) => format!("wrote parts/{} {}, replacing {was}", part.id, part.version),
            (None, true) => format!("wrote parts/{} {}", part.id, part.version),
            (None, false) => format!("skipped parts/{} {}", part.id, part.version),
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
        .map(|part| match &part.replaced {
            Some(was) => json!({ "id": part.id, "version": part.version, "replaced": was }),
            None => json!({ "id": part.id, "version": part.version }),
        })
        .collect()
}

#[cfg(test)]
#[path = "txn_tests.rs"]
mod txn_tests;
