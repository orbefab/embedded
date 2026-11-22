//! `orbe add` decides what to copy, then writes it as one transaction.
//!
//! [`plan::plan`] is a pure function of the request, the catalog and a snapshot of the folders
//! it may write. [`txn::apply`] is the only code that writes part folders. It logs each step,
//! and a run that stops part way is rolled back or finished by the next run, which
//! [`journal::next`] decides from the log and the hashes on disk. [`held`] keeps a second run
//! out while one is at work.

mod at;
mod block;
mod held;
mod journal;
mod plan;
mod snapshot;
mod txn;

use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::catalog::Catalog;
use crate::project;

pub use block::BlockReply;
use held::Hold;
use plan::Plan;
#[cfg(test)]
use txn::Inject;
use txn::{Change, Fault, Stop};

/// What the CLI asked `orbe add` to do.
pub struct Request {
    pub ids: Vec<String>,
}

/// A reply the CLI prints as text or as one JSON envelope.
pub enum Reply {
    Add(AddReply),
    Block(BlockReply),
    Refuse(Refusal),
    Usage(String),
}

pub struct AddReply {
    pub parts: Vec<Copied>,
    pub notes: Vec<String>,
}

pub struct Copied {
    pub id: String,
    pub version: String,
    pub wrote: bool,
}

pub struct Refusal {
    pub code: AddCode,
    pub message: String,
    pub hint: String,
}

/// Each code a refused `orbe add` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddCode {
    Busy,
    Unreadable,
    NotAProject,
    SceneExists,
    FileExists,
    Symlink,
    Registry,
    UnknownPart,
    PartExists,
}

impl AddCode {
    pub fn as_str(self) -> &'static str {
        match self {
            AddCode::Busy => "busy",
            AddCode::Unreadable => "unreadable",
            AddCode::NotAProject => "not_a_project",
            AddCode::SceneExists => "scene_exists",
            AddCode::FileExists => "file_exists",
            AddCode::Symlink => "symlink",
            AddCode::Registry => "registry",
            AddCode::UnknownPart => "unknown_part",
            AddCode::PartExists => "part_exists",
        }
    }
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
            Reply::Block(reply) => block::text(reply),
            Reply::Refuse(refusal) => refuse_text(refusal),
            Reply::Usage(message) => format!("error[usage]: {message}"),
        }
    }

    pub fn json(&self) -> Value {
        match self {
            Reply::Add(reply) => json!({ "ok": true, "data": add_data(reply) }),
            Reply::Block(reply) => block::json(reply),
            Reply::Refuse(refusal) => refuse_json(refusal),
            Reply::Usage(message) => json!({
                "ok": false,
                "error": { "code": "usage", "message": message }
            }),
        }
    }
}

/// Copies parts into a project, or installs the one Block the request names. A refusal writes
/// nothing. `dir` is the folder `-C` named, as the user wrote it, so
/// hints can repeat it. With none, a folder inside another project must already be one. An
/// empty folder, and any folder with no project above it, is where a new project starts. An
/// id the catalog does not have is reported before either of those.
pub fn run(dir: Option<&Path>, request: &Request) -> Result<Reply, RunError> {
    run_with(dir, request, &mut |_| None)
}

fn run_with(dir: Option<&Path>, request: &Request, fault: &mut Fault) -> Result<Reply, RunError> {
    match block::named(dir, request) {
        Some(Ok(block)) => return block::install(dir, block, fault),
        Some(Err(message)) => return Ok(Reply::Usage(message)),
        None => {}
    }
    if let Some(message) = plan::usage(request) {
        return Ok(Reply::Usage(message));
    }
    if let Some(refusal) = plan::refuse_unknown(request) {
        return Ok(Reply::Refuse(refusal));
    }
    if let Some(refusal) = not_a_project(dir, request) {
        return Ok(Reply::Refuse(refusal));
    }
    run_in(dir, request, fault)
}

/// Adds the request's parts to the project in `dir`, which need not hold a scene yet.
fn run_in(dir: Option<&Path>, request: &Request, fault: &mut Fault) -> Result<Reply, RunError> {
    let root = dir.unwrap_or(Path::new("."));
    let add = project::add_command(dir);
    let again = Again::new(dir, request_command(&add, request));
    let _held = match held::hold(root) {
        Ok(held) => Some(held),
        Err(Hold::Busy) => return Ok(Reply::Refuse(busy(&again))),
        Err(Hold::Io(error)) => return Err(error),
    };
    let recovered = txn::recover(root, &again, fault).map_err(stop_error)?;
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
                lock: None,
            };
            let mut notes = txn::apply(root, &change, &again, fault).map_err(stop_error)?;
            notes.extend(planned.notes);
            Reply::Add(add_reply(planned.write, notes))
        }
        Plan::Refuse(refusal) => Reply::Refuse(refusal),
        Plan::Usage(message) => Reply::Usage(message),
    };
    Ok(with_notes(reply, recovered.lines))
}

/// The command a hint says to run again, and the folder `-C` named as the user wrote it, so a
/// path in a hint names the same entry from the folder the user ran in.
pub(super) struct Again {
    command: String,
    dir: Option<PathBuf>,
}

impl Again {
    fn new(dir: Option<&Path>, command: String) -> Again {
        let dir = dir.filter(|dir| !dir.as_os_str().is_empty()).map(|dir| {
            match dir.as_os_str().as_encoded_bytes().first() {
                Some(b'-') => Path::new(".").join(dir),
                _ => dir.to_path_buf(),
            }
        });
        Again { command, dir }
    }

    /// `rel`, a path from the project folder, as one shell word for the same path from the
    /// current folder.
    pub(super) fn path(&self, rel: impl AsRef<OsStr>) -> String {
        let rel = Path::new(rel.as_ref());
        let path = match &self.dir {
            Some(dir) => dir.join(rel),
            None => rel.to_path_buf(),
        };
        project::shell_path(path.as_os_str())
    }
}

impl std::fmt::Display for Again {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.command)
    }
}

fn stop_error(stop: Stop) -> RunError {
    match stop {
        Stop::Failed(error) => error,
        Stop::Killed => RunError {
            message: "killed".into(),
        },
    }
}

fn busy(again: &Again) -> Refusal {
    Refusal {
        code: AddCode::Busy,
        message: "another `orbe add` is at work in this project, and holds its lock".into(),
        hint: format!("wait for it to finish, then run `{again}`"),
    }
}

/// With no `-C`, a folder inside another project must hold `scene.jsonc` or a `parts/`
/// folder. Anywhere else, `orbe add` would make a second project, so it names the folder
/// above that is one. A folder with no project above it is where a new project starts,
/// including an empty one. A folder `-C` names is trusted, since `orbe check` names it for
/// a scene of any name.
fn not_a_project(dir: Option<&Path>, request: &Request) -> Option<Refusal> {
    if let Some(dir) = dir {
        return (!dir.is_dir()).then(|| Refusal {
            code: AddCode::NotAProject,
            message: format!("`{}` is not a folder", dir.display()),
            hint: String::new(),
        });
    }
    let here = Path::new(".");
    if is_project(here) {
        return None;
    }
    let above = project_above(here)?;
    let hint = format!(
        "run `{}`",
        request_command(&project::add_command(Some(&above)), request)
    );
    Some(Refusal {
        code: AddCode::NotAProject,
        message:
            "the current folder is not an orbe project. it has no scene.jsonc and no parts folder"
                .into(),
        hint,
    })
}

fn is_project(dir: &Path) -> bool {
    dir.join("scene.jsonc").is_file() || dir.join("parts").is_dir()
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
    project::format_add(add, &request.ids).expect("a request names an id")
}

/// The command a missing-part hint prints for `ids`. It is the command planning an add of
/// those ids would print. A folder already there makes that command the one `part_exists`
/// names, so retrying runs the whole add. `None` when the folders cannot be read. A plan
/// that would write names the requested ids, not every child.
pub(crate) fn repair_command(root: &Path, add: &str, ids: &[String]) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let request = Request { ids: ids.to_vec() };
    if plan::usage(&request).is_some() {
        return None;
    }
    let catalog = Catalog::builtin();
    let wanted = plan::wanted(&catalog, &request);
    let snapshot = snapshot::load(root, &wanted).ok()?;
    match plan::plan(&catalog, &snapshot, &request, add) {
        Plan::Add(_) => project::add_writing(add, ids),
        Plan::Refuse(refusal) => command_in(&refusal.hint),
        _ => None,
    }
}

/// The first backquoted `orbe add` in `text` that names an id.
fn command_in(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(at) = rest.find("`orbe add") {
        rest = &rest[at + 1..];
        let end = rest.find('`')?;
        let command = &rest[..end];
        rest = &rest[end + 1..];
        let names_id = command.split_whitespace().any(|word| {
            !word.starts_with('-') && word != "orbe" && word != "add" && !word.starts_with('\'')
        });
        if names_id {
            return Some(command.to_string());
        }
    }
    None
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
        Reply::Block(mut reply) => {
            reply.parts.notes.splice(0..0, lines);
            Reply::Block(reply)
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

fn add_reply(write: Vec<plan::Write>, notes: Vec<String>) -> AddReply {
    let mut parts: Vec<Copied> = write
        .into_iter()
        .map(|write| Copied {
            id: write.id,
            version: write.version.to_string(),
            wrote: true,
        })
        .collect();
    parts.sort_by(|a, b| a.id.cmp(&b.id));
    AddReply { parts, notes }
}

fn add_data(reply: &AddReply) -> Value {
    json!({
        "written": versions(&reply.parts),
        "notes": reply.notes,
    })
}

fn add_text(reply: &AddReply) -> String {
    let mut lines: Vec<String> = reply
        .parts
        .iter()
        .map(|part| format!("wrote parts/{} {}", part.id, part.version))
        .collect();
    lines.extend(reply.notes.iter().cloned());
    lines.join("\n")
}

fn refuse_text(refusal: &Refusal) -> String {
    let mut lines = refusal.message.lines();
    let first = lines.next().unwrap_or("");
    let mut text = format!("error[{}]: {first}", refusal.code.as_str());
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
            "error": { "code": refusal.code.as_str(), "message": refusal.message }
        })
    } else {
        json!({
            "ok": false,
            "error": { "code": refusal.code.as_str(), "message": refusal.message, "hint": refusal.hint }
        })
    }
}

fn versions(parts: &[Copied]) -> Vec<Value> {
    parts
        .iter()
        .map(|part| json!({ "id": part.id, "version": part.version }))
        .collect()
}

#[cfg(test)]
#[path = "txn_tests.rs"]
mod txn_tests;
