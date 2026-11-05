//! One `orbe add` is one transaction. New folders are staged inside `parts/.orbe-add/`, so every
//! rename stays on the filesystem that holds `parts/`. The originals move aside into
//! `parts/.orbe-add/old/`, the new folders move in, the lock is replaced, and the log is marked
//! committed. Only then are the originals deleted.
//!
//! Every step appends a line to `parts/.orbe-add/log` once it has completed, and recovery
//! ([`journal::next`]) acts only on what that log proves. A run that stops before the commit is
//! rolled back, by itself or by the next run. A run that stops after it is finished by the next.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::journal::{self, Effect, Hash, Here, LockChange, Log, Next, Plan, Read, Record, Seen};
use super::{RunError, io_at};
use crate::project::hex;

pub(super) use journal::STAGE;

/// The new lock before it is renamed over `parts.lock`, beside it at the project root.
const LOCK_TMP: &str = ".parts.lock.orbe-add";
const LOG: &str = "log";
const SAVED_LOCK: &str = "parts.lock";

/// The folders to place, by id, and the new lock text when it changes.
pub(super) struct Change<'a> {
    pub(super) folders: Vec<(&'a str, &'a BTreeMap<String, Vec<u8>>)>,
    pub(super) lock: Option<&'a str>,
}

/// What a test injects at a point. `Kill` stops as a killed process would, with no undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code, reason = "only tests inject faults"))]
pub(super) enum Inject {
    Fail,
    Kill,
}

#[derive(Debug)]
pub(super) enum Stop {
    /// The message says what is on disk.
    Failed(RunError),
    Killed,
}

impl From<RunError> for Stop {
    fn from(error: RunError) -> Stop {
        Stop::Failed(error)
    }
}

/// Called before each change on disk, as `do <step>`, and before each log line, as
/// `log <line>`.
pub(super) type Fault<'a> = dyn FnMut(&str) -> Option<Inject> + 'a;

fn at(fault: &mut Fault, point: &str) -> Result<(), Stop> {
    match fault(point) {
        None => Ok(()),
        Some(Inject::Kill) => Err(Stop::Killed),
        Some(Inject::Fail) => Err(Stop::Failed(RunError {
            message: format!("injected failure at {point}"),
        })),
    }
}

struct Paths {
    root: PathBuf,
    parts: PathBuf,
    stage: PathBuf,
}

impl Paths {
    fn new(root: &Path) -> Paths {
        let parts = root.join("parts");
        Paths {
            root: root.to_path_buf(),
            stage: parts.join(STAGE),
            parts,
        }
    }

    fn new_copy(&self, id: &str) -> PathBuf {
        self.stage.join("new").join(id)
    }

    fn old_copy(&self, id: &str) -> PathBuf {
        self.stage.join("old").join(id)
    }

    fn dest(&self, id: &str) -> PathBuf {
        self.parts.join(id)
    }

    fn lock(&self) -> PathBuf {
        self.root.join("parts.lock")
    }

    fn lock_tmp(&self) -> PathBuf {
        self.root.join(LOCK_TMP)
    }

    fn log(&self) -> PathBuf {
        self.stage.join(LOG)
    }

    fn saved_lock(&self) -> PathBuf {
        self.stage.join(SAVED_LOCK)
    }
}

/// Writes the change as one transaction. On a failure it rolls back and says what it rolled
/// back. A failure after the commit only leaves the replaced copies, which the next run deletes,
/// so it returns a note instead.
pub(super) fn apply(
    root: &Path,
    change: &Change,
    again: &str,
    fault: &mut Fault,
) -> Result<Vec<String>, Stop> {
    let paths = Paths::new(root);
    if change.folders.is_empty() {
        if let Some(text) = change.lock {
            at(fault, "do write parts.lock")?;
            write_lock(&paths, text.as_bytes())?;
        }
        return Ok(Vec::new());
    }
    let plan = Plan {
        parts_created: !present(&paths.parts),
        lock: match change.lock {
            None => LockChange::Kept,
            Some(text) if present(&paths.lock()) => {
                LockChange::Replace(bytes_hash(text.as_bytes()))
            }
            Some(text) => LockChange::Add(bytes_hash(text.as_bytes())),
        },
        ids: change
            .folders
            .iter()
            .map(|(id, _)| (*id).to_string())
            .collect(),
    };
    match forward(&paths, change, &plan, fault) {
        Ok(()) => {}
        Err(Stop::Killed) => return Err(Stop::Killed),
        Err(Stop::Failed(error)) => return Err(roll_back(&paths, &plan, error, again, fault)),
    }
    match settle(&paths, again, fault) {
        Ok(_) => Ok(Vec::new()),
        Err(Stop::Killed) => Err(Stop::Killed),
        Err(Stop::Failed(error)) => Ok(vec![format!(
            "the add is written, and finishing it stopped: {error}"
        )]),
    }
}

fn forward(paths: &Paths, change: &Change, plan: &Plan, fault: &mut Fault) -> Result<(), Stop> {
    at(fault, "do start the log")?;
    fs::create_dir_all(&paths.parts).map_err(|error| io_at(&paths.parts, error))?;
    fs::create_dir(&paths.stage).map_err(|error| io_at(&paths.stage, error))?;
    write_synced(&paths.log(), plan.text().as_bytes())?;
    sync_dir(&paths.parts);
    for dir in [paths.stage.join("new"), paths.stage.join("old")] {
        fs::create_dir(&dir).map_err(|error| io_at(&dir, error))?;
    }
    let mut staged = BTreeMap::new();
    for (id, files) in &change.folders {
        at(fault, &format!("do stage {id}"))?;
        let dir = paths.new_copy(id);
        write_files(&dir, files)?;
        let hash = fingerprint(&dir).unwrap_or_default();
        staged.insert(*id, hash.clone());
        append(paths, &Record::Staged((*id).into(), hash), fault)?;
    }
    if matches!(plan.lock, LockChange::Replace(_)) {
        at(fault, "do save parts.lock")?;
        let bytes = fs::read(paths.lock()).map_err(|error| io_at(&paths.lock(), error))?;
        write_synced(&paths.saved_lock(), &bytes)?;
        append(paths, &Record::SavedLock(bytes_hash(&bytes)), fault)?;
    }
    for (id, _) in &change.folders {
        let dest = paths.dest(id);
        let before = if present(&dest) {
            at(fault, &format!("do aside {id}"))?;
            mv(&dest, &paths.old_copy(id))?;
            fingerprint(&paths.old_copy(id))
        } else {
            None
        };
        append(paths, &Record::Aside((*id).into(), before), fault)?;
        at(fault, &format!("do place {id}"))?;
        mv(&paths.new_copy(id), &dest)?;
        append(
            paths,
            &Record::Placed((*id).into(), staged[id].clone()),
            fault,
        )?;
    }
    if let Some(text) = change.lock {
        at(fault, "do write parts.lock")?;
        write_lock(paths, text.as_bytes())?;
        append(paths, &Record::LockWritten, fault)?;
    }
    append(paths, &Record::Committed, fault)
}

/// A step of the add failed. Recovery undoes what the log proves, and the message says what is
/// on disk afterwards.
fn roll_back(paths: &Paths, plan: &Plan, error: RunError, again: &str, fault: &mut Fault) -> Stop {
    let settled = settle(paths, again, fault);
    if plan.parts_created && !present(&paths.stage) {
        let _ = fs::remove_dir(&paths.parts);
    }
    let message = match settled {
        Err(Stop::Killed) => return Stop::Killed,
        Err(Stop::Failed(also)) => format!("{error}\nthe rollback stopped: {also}"),
        Ok(Some(Done {
            undone: Some(lines),
            kept: false,
            ..
        })) => format!("{error}\nrolled back, so the project is as it was: {lines}"),
        Ok(Some(Done {
            undone: Some(lines),
            ..
        })) => format!("{error}\nrolled back: {lines}"),
        Ok(_) => format!("{error}\nnothing was written"),
    };
    Stop::Failed(RunError { message })
}

/// What a finished recovery did.
struct Done {
    /// One line for the reply.
    line: String,
    /// What a rollback undid, when it undid anything.
    undone: Option<String>,
    /// Whether it left a changed copy in place.
    kept: bool,
}

/// Rolls back an add that stopped before its commit, or finishes one that stopped after it.
/// Returns one line saying what it did, or nothing when no add was pending.
pub(super) fn recover(root: &Path, again: &str, fault: &mut Fault) -> Result<Vec<String>, Stop> {
    let done = settle(&Paths::new(root), again, fault)?;
    Ok(done.map(|done| done.line).into_iter().collect())
}

fn settle(paths: &Paths, again: &str, fault: &mut Fault) -> Result<Option<Done>, Stop> {
    if !present(&paths.stage) {
        if !present(&paths.lock_tmp()) {
            return Ok(None);
        }
        at(fault, &format!("do delete {LOCK_TMP}"))?;
        remove_any(&paths.lock_tmp())?;
        return Ok(Some(Done {
            line: format!("deleted {LOCK_TMP}, a lock an interrupted add had not put in place"),
            undone: None,
            kept: false,
        }));
    }
    if !fs::symlink_metadata(&paths.stage).is_ok_and(|meta| meta.is_dir()) {
        return Err(halt(format!(
            "parts/{STAGE} is not a folder, so no add made it. move it out of parts/, then run `{again}` again"
        )));
    }
    let text = match fs::read(paths.log()) {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_at(&paths.log(), error).into()),
    };
    let mut log = match text.as_deref().map(journal::read) {
        Some(Read::Log(log)) => log,
        Some(Read::Unknown(line)) => {
            return Err(halt(format!(
                "parts/{STAGE}/log has a line this orbe does not write, `{line}`, so orbe left parts/{STAGE} as it is. move the folders in parts/{STAGE}/old back to parts/ by hand, then delete parts/{STAGE}"
            )));
        }
        None | Some(Read::Torn) => return without_log(paths, again, fault).map(Some),
    };
    let start = log.records.len();
    loop {
        let seen = observe(paths, &log.plan);
        match journal::next(&log, &seen) {
            Next::Act(effect, record) => {
                if effect != Effect::None {
                    at(fault, &format!("do {}", record.line()))?;
                    execute(paths, &effect).map_err(|error| stopped(&log, start, error))?;
                }
                append(paths, &record, fault).map_err(|stop| match stop {
                    Stop::Failed(error) => stopped(&log, start, error),
                    killed => killed,
                })?;
                log.records.push(record);
            }
            Next::Clean => {
                at(fault, "do clean")?;
                let done = done(&log);
                clean(paths, &log).map_err(|error| stopped(&log, start, error))?;
                return Ok(Some(done));
            }
            Next::Stop(problems) => {
                let (lines, hint) = journal::problem_text(&log, &problems, again);
                let taken = journal::steps_taken(&log, start);
                let mut message = lines;
                if !taken.is_empty() {
                    message.push_str(&format!("\nbefore it stopped, orbe {}", taken.join(", ")));
                }
                message.push_str(&format!("\n  hint: {hint}"));
                return Err(halt(message));
            }
        }
    }
}

fn done(log: &Log) -> Done {
    let summary = journal::summary(log);
    let kept = log
        .records
        .iter()
        .any(|record| matches!(record, Record::Kept(_) | Record::LockKept));
    if log.committed() {
        return Done {
            line: summary.unwrap_or_default(),
            undone: None,
            kept,
        };
    }
    let line = match &summary {
        Some(lines) => format!("rolled back an interrupted add: {lines}"),
        None => "deleted the staging folder of an interrupted add that had not changed the project"
            .into(),
    };
    Done {
        line,
        undone: summary,
        kept,
    }
}

/// A staging folder with no log. A run writes the log before anything else goes in it and
/// deletes the log after everything else, so a folder that holds more was not left by a run.
/// An empty one is left either before the log or after a finished run, so it claims neither.
fn without_log(paths: &Paths, again: &str, fault: &mut Fault) -> Result<Done, Stop> {
    let names = names(&paths.stage);
    if names.iter().any(|name| name != LOG) {
        let listed = names
            .iter()
            .map(|name| format!("parts/{STAGE}/{name}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(halt(format!(
            "parts/{STAGE} has no log, so orbe cannot tell what the add that left it changed, and it left {listed} as they are. that add may have written folders in parts/ and parts.lock\n  hint: move anything you want to keep out of parts/{STAGE}, delete parts/{STAGE}, then run `{again}` again"
        )));
    }
    at(fault, &format!("do delete parts/{STAGE}"))?;
    let line = if names.is_empty() {
        format!("deleted parts/{STAGE}, an empty folder an earlier add left")
    } else {
        remove_any(&paths.log())?;
        format!("deleted parts/{STAGE}, which an interrupted add left before it changed anything")
    };
    fs::remove_dir(&paths.stage).map_err(|error| io_at(&paths.stage, error))?;
    Ok(Done {
        line,
        undone: None,
        kept: false,
    })
}

fn halt(message: String) -> Stop {
    Stop::Failed(RunError { message })
}

/// An I/O error part way through recovery. The log keeps every step done so far.
fn stopped(log: &Log, start: usize, error: RunError) -> Stop {
    let taken = journal::steps_taken(log, start);
    let mut message = format!("{error}\norbe stopped there, and parts/{STAGE} keeps its log");
    if !taken.is_empty() {
        message.push_str(&format!(". before it stopped, orbe {}", taken.join(", ")));
    }
    message.push_str(". the next `orbe add` picks up from there");
    halt(message)
}

fn observe(paths: &Paths, plan: &Plan) -> Seen {
    Seen {
        parts: plan
            .ids
            .iter()
            .map(|id| {
                let here = Here {
                    dest: fingerprint(&paths.dest(id)),
                    new: present(&paths.new_copy(id)),
                    old: present(&paths.old_copy(id)),
                };
                (id.clone(), here)
            })
            .collect(),
        lock: fingerprint(&paths.lock()),
        saved_lock: fingerprint(&paths.saved_lock()),
        names: names(&paths.stage),
    }
}

/// Every name in the staging folder, with the names inside `new/` and `old/`.
fn names(stage: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for name in entries(stage) {
        if (name == "new" || name == "old") && stage.join(&name).is_dir() {
            names.extend(
                entries(&stage.join(&name))
                    .into_iter()
                    .map(|inner| format!("{name}/{inner}")),
            );
        }
        names.push(name);
    }
    names.sort();
    names
}

fn entries(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
        .collect()
}

fn execute(paths: &Paths, effect: &Effect) -> Result<(), RunError> {
    match effect {
        Effect::None => Ok(()),
        Effect::Unplace(id) => mv(&paths.dest(id), &paths.new_copy(id)),
        Effect::Restore(id) => mv(&paths.old_copy(id), &paths.dest(id)),
        Effect::Delete(id) => remove_any(&paths.old_copy(id)),
        Effect::LockBack => {
            let saved = paths.saved_lock();
            let bytes = fs::read(&saved).map_err(|error| io_at(&saved, error))?;
            write_lock(paths, &bytes)
        }
        Effect::LockRemove => remove_any(&paths.lock()),
    }
}

/// Deletes what the log made, the log last, then the staging folder. Only names the log made
/// are here, since [`journal::next`] stops on any other.
fn clean(paths: &Paths, log: &Log) -> Result<(), RunError> {
    let mut paths_to_delete = vec![paths.lock_tmp()];
    if !log.committed() {
        paths_to_delete.extend(log.plan.ids.iter().map(|id| paths.new_copy(id)));
    }
    paths_to_delete.push(paths.saved_lock());
    for path in paths_to_delete {
        if present(&path) {
            remove_any(&path)?;
        }
    }
    for dir in [paths.stage.join("new"), paths.stage.join("old")] {
        if present(&dir) {
            fs::remove_dir(&dir).map_err(|error| io_at(&dir, error))?;
        }
    }
    remove_any(&paths.log())?;
    fs::remove_dir(&paths.stage).map_err(|error| io_at(&paths.stage, error))?;
    if log.plan.parts_created && !log.committed() {
        let _ = fs::remove_dir(&paths.parts);
    }
    Ok(())
}

/// `--diff` writes nothing, so an interrupted add stays for the next writing run.
pub(super) enum Pending {
    None,
    Uncommitted,
    Committed,
    Unclear,
}

pub(super) fn pending(root: &Path) -> Pending {
    let paths = Paths::new(root);
    if !present(&paths.stage) {
        return if present(&paths.lock_tmp()) {
            Pending::Uncommitted
        } else {
            Pending::None
        };
    }
    let text = fs::read(paths.log()).unwrap_or_default();
    match journal::read(&String::from_utf8_lossy(&text)) {
        Read::Log(log) if log.committed() => Pending::Committed,
        Read::Log(_) => Pending::Uncommitted,
        _ => Pending::Unclear,
    }
}

fn append(paths: &Paths, record: &Record, fault: &mut Fault) -> Result<(), Stop> {
    let line = record.line();
    at(fault, &format!("log {line}"))?;
    let path = paths.log();
    let mut file = OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(|error| io_at(&path, error))?;
    file.write_all(format!("{line}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| io_at(&path, error).into())
}

/// A hash of whatever is at `path`, which never follows a link and never fails. `None` when
/// nothing is there.
pub(super) fn fingerprint(path: &Path) -> Option<Hash> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(_) => return Some(digest(vec!["unreadable .".into()])),
    };
    let mut lines = Vec::new();
    describe(path, ".", &meta, &mut lines);
    Some(digest(lines))
}

fn describe(path: &Path, rel: &str, meta: &fs::Metadata, lines: &mut Vec<String>) {
    let kind = meta.file_type();
    if kind.is_symlink() {
        let target = fs::read_link(path).map(|target| target.display().to_string());
        lines.push(format!("link {rel} {}", target.unwrap_or_default()));
    } else if kind.is_dir() {
        lines.push(format!("dir {rel}"));
        let Ok(entries) = fs::read_dir(path) else {
            lines.push(format!("unreadable {rel}"));
            return;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                lines.push(format!("unreadable {rel}"));
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if rel == "." {
                name
            } else {
                format!("{rel}/{name}")
            };
            match fs::symlink_metadata(entry.path()) {
                Ok(meta) => describe(&entry.path(), &child, &meta, lines),
                Err(_) => lines.push(format!("unreadable {child}")),
            }
        }
    } else if kind.is_file() {
        match fs::read(path) {
            Ok(bytes) => lines.push(file_line(rel, &bytes)),
            Err(_) => lines.push(format!("unreadable {rel}")),
        }
    } else {
        lines.push(format!("other {rel}"));
    }
}

fn file_line(rel: &str, bytes: &[u8]) -> String {
    format!("file {rel} {}", hex(&Sha256::digest(bytes)))
}

fn digest(mut lines: Vec<String>) -> Hash {
    lines.sort();
    format!(
        "sha256:{}",
        hex(&Sha256::digest(lines.join("\n").as_bytes()))
    )
}

/// The fingerprint of a file holding these bytes.
fn bytes_hash(bytes: &[u8]) -> Hash {
    digest(vec![file_line(".", bytes)])
}

fn write_lock(paths: &Paths, bytes: &[u8]) -> Result<(), RunError> {
    write_synced(&paths.lock_tmp(), bytes)?;
    mv(&paths.lock_tmp(), &paths.lock())
}

fn write_files(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), RunError> {
    fs::create_dir_all(dir).map_err(|error| io_at(dir, error))?;
    for (rel, bytes) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| io_at(parent, error))?;
        }
        write_synced(&path, bytes)?;
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), RunError> {
    let mut file = File::create(path).map_err(|error| io_at(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_at(path, error))
}

/// A rename, with both folders synced so the move is on disk before its log line.
fn mv(from: &Path, to: &Path) -> Result<(), RunError> {
    fs::rename(from, to).map_err(|error| RunError {
        message: format!(
            "cannot move {} to {}: {error}",
            from.display(),
            to.display()
        ),
    })?;
    for dir in [from.parent(), to.parent()].into_iter().flatten() {
        sync_dir(dir);
    }
    Ok(())
}

/// Best effort: some platforms cannot open a folder to sync it.
fn sync_dir(dir: &Path) {
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
}

/// A link is removed, never followed.
fn remove_any(path: &Path) -> Result<(), RunError> {
    let meta = fs::symlink_metadata(path).map_err(|error| io_at(path, error))?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|error| io_at(path, error))
}

fn present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
