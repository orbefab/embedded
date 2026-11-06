//! One `orbe add` is one transaction. New folders are staged inside `parts/.orbe-add/`, so every
//! rename stays on the filesystem that holds `parts/`. The originals move aside into
//! `parts/.orbe-add/old/`, the new folders move in, the lock is replaced, and the log is marked
//! committed. Only then are the originals deleted.
//!
//! Every step appends a line to `parts/.orbe-add/log`, and recovery ([`journal::next`]) acts
//! only on what that log proves. A run that stops before the commit is rolled back, by itself or
//! by the next run. A run that stops after it is finished by the next. Every move and delete
//! first checks that its path holds the hash the log recorded. Before the staging folder is
//! deleted, the log lists every path in it with its hash. Each path is deleted only while it
//! holds what that list says, the log goes last, and anything else stops the run before it
//! deletes a thing.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::journal::{
    self, Effect, Entry, Hash, Here, Kind, LockChange, Log, Next, Plan, Read, Record, Seen,
};
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
    match settle_stage(&paths, again, fault) {
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
        let before = fingerprint(&dest);
        append(paths, &Record::Aside((*id).into(), before.clone()), fault)?;
        if before.is_some() {
            at(fault, &format!("do aside {id}"))?;
            mv(&dest, &paths.old_copy(id))?;
        }
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
    let settled = settle_stage(paths, again, fault);
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
    planned: Vec<String>,
}

#[derive(Default)]
pub(super) struct Recovered {
    pub(super) lines: Vec<String>,
    pub(super) planned: Vec<String>,
}

/// Rolls back an add that stopped before its commit, or finishes one that stopped after it.
pub(super) fn recover(root: &Path, again: &str, fault: &mut Fault) -> Result<Recovered, Stop> {
    let done = settle_stage(&Paths::new(root), again, fault)?;
    Ok(done.map_or_else(Recovered::default, |done| Recovered {
        lines: vec![done.line],
        planned: done.planned,
    }))
}

fn settle_stage(paths: &Paths, again: &str, fault: &mut Fault) -> Result<Option<Done>, Stop> {
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
            planned: Vec::new(),
        }));
    }
    if !fs::symlink_metadata(&paths.stage).is_ok_and(|meta| meta.is_dir()) {
        return Err(halt(format!(
            "parts/{STAGE} is not a folder, so no add made it. move it out of parts/, then run `{again}` again"
        )));
    }
    let bytes = match fs::symlink_metadata(paths.log()) {
        Ok(meta) if meta.is_file() => {
            Some(fs::read(paths.log()).map_err(|error| io_at(&paths.log(), error))?)
        }
        Ok(_) => {
            return Err(halt(format!(
                "parts/{STAGE}/log is not a file, so no add wrote it, and orbe left parts/{STAGE} as it is\n  hint: move parts/{STAGE}/log out of parts/{STAGE}, then run `{again}` again"
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_at(&paths.log(), error).into()),
    };
    let text = bytes
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    let mut log = match text.as_deref().map(journal::read) {
        Some(Read::Log(log)) => log,
        Some(Read::Unknown(line)) => {
            return Err(halt(format!(
                "parts/{STAGE}/log is not a log this orbe writes, `{line}`, so orbe left parts/{STAGE} as it is\n  hint: move parts/{STAGE} out of parts/ to keep what it holds, then run `{again}` again"
            )));
        }
        Some(Read::UnknownStep(line)) => {
            return Err(halt(format!(
                "parts/{STAGE}/log has a line this orbe does not write, `{line}`, so orbe left parts/{STAGE} as it is\n  hint: delete that line and the lines after it from parts/{STAGE}/log, then run `{again}` again. orbe then settles the add from the steps before it"
            )));
        }
        None | Some(Read::Torn) => return without_log(paths, again, fault).map(Some),
    };
    drop_torn_line(paths, bytes.as_deref().unwrap_or_default(), fault)?;
    let start = log.records.len();
    if log.clearing().is_some() {
        return clear(paths, &log, start, again, fault).map(Some);
    }
    loop {
        let seen = observe(paths, &log.plan);
        match journal::next(&log, &seen) {
            Next::Act(effect, record) => {
                if effect != Effect::None {
                    at(fault, &format!("do {}", record.line()))?;
                    execute(paths, &effect).map_err(|error| stopped(&log, start, error, again))?;
                }
                append(paths, &record, fault).map_err(|stop| match stop {
                    Stop::Failed(error) => stopped(&log, start, error, again),
                    killed => killed,
                })?;
                log.records.push(record);
            }
            Next::Clean => {
                let io = |stop| io_stop(&log, start, stop, again);
                at(fault, "do clean").map_err(io)?;
                if present(&paths.lock_tmp()) {
                    remove_any(&paths.lock_tmp()).map_err(|error| io(error.into()))?;
                }
                let (entries, _) = contents(&paths.stage).map_err(|error| io(error.into()))?;
                let record = Record::Clearing(entries);
                append(paths, &record, fault).map_err(io)?;
                log.records.push(record);
                return clear(paths, &log, start, again, fault).map(Some);
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

/// A run killed inside an append leaves part of a line, which proves nothing. It goes before
/// the next append, so that append starts a line of its own.
fn drop_torn_line(paths: &Paths, bytes: &[u8], fault: &mut Fault) -> Result<(), Stop> {
    if bytes.last().is_none_or(|last| *last == b'\n') {
        return Ok(());
    }
    at(fault, "do drop the torn log line")?;
    let keep = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let path = paths.log();
    OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|file| {
            file.set_len(keep as u64)?;
            file.sync_all()
        })
        .map_err(|error| io_at(&path, error).into())
}

/// Deletes what the clearing record lists, each path only while it holds what the record says,
/// files and links first and then folders, deepest first. The log goes last, then the folder.
/// Anything else below the folder stops the run before it deletes a thing, and is named.
fn clear(
    paths: &Paths,
    log: &Log,
    start: usize,
    again: &str,
    fault: &mut Fault,
) -> Result<Done, Stop> {
    let listed = log.clearing().unwrap_or_default();
    let io = |stop| io_stop(log, start, stop, again);
    let (now, unlisted) = unlisted(&paths.stage, listed).map_err(|error| io(error.into()))?;
    if !unlisted.is_empty() {
        return Err(halt(unlisted_text(&unlisted, again)));
    }
    let (mut folders, rest): (Vec<&Entry>, Vec<&Entry>) =
        now.iter().partition(|entry| entry.kind == Kind::Folder);
    folders.sort_by(|a, b| b.path.cmp(&a.path));
    for entry in rest.into_iter().chain(folders) {
        let path = paths.stage.join(&entry.path);
        at(fault, &format!("do delete parts/{STAGE}/{}", entry.path)).map_err(io)?;
        let removed = if entry.kind == Kind::Folder {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        };
        removed.map_err(|error| io(io_at(&path, error).into()))?;
    }
    at(fault, &format!("do delete parts/{STAGE}/{LOG}")).map_err(io)?;
    fs::remove_file(paths.log()).map_err(|error| io(io_at(&paths.log(), error).into()))?;
    fs::remove_dir(&paths.stage).map_err(|error| io(io_at(&paths.stage, error).into()))?;
    if log.plan.parts_created && !log.committed() {
        let _ = fs::remove_dir(&paths.parts);
    }
    let replaced: Vec<String> = log
        .plan
        .ids
        .iter()
        .filter(|id| listed.iter().any(|entry| entry.path == format!("old/{id}")))
        .cloned()
        .collect();
    let mut done = done(log, &replaced);
    if done.undone.is_none()
        && !log.committed()
        && !lock_holes(paths.root.as_path(), &log.plan.ids).is_empty()
    {
        done.line = "deleted the staging folder of an interrupted add".into();
    }
    Ok(done)
}

/// What is below `stage` now, and the paths of it `listed` does not hold.
fn unlisted(stage: &Path, listed: &[Entry]) -> Result<(Vec<Entry>, Vec<String>), RunError> {
    let (now, mut unlisted) = contents(stage)?;
    unlisted.extend(
        now.iter()
            .filter(|entry| !listed.contains(entry))
            .map(|entry| entry.path.clone()),
    );
    Ok((now, unlisted))
}

fn unlisted_text(names: &[String], again: &str) -> String {
    let mut tops: Vec<&String> = names
        .iter()
        .filter(|name| {
            !names
                .iter()
                .any(|other| name.starts_with(&format!("{other}/")))
        })
        .collect();
    tops.sort();
    let listed = tops
        .iter()
        .map(|name| format!("parts/{STAGE}/{name}"))
        .collect::<Vec<_>>()
        .join(", ");
    let them = if tops.len() == 1 { "it" } else { "them" };
    format!(
        "{listed} {} not on the list the add made before it began to delete parts/{STAGE}, so orbe cannot tell that the add made {them}. orbe deleted nothing, and left parts/{STAGE} and its log\n  hint: move {listed} out of parts/{STAGE} to keep {them}, then run `{again}` again",
        if tops.len() == 1 { "is" } else { "are" }
    )
}

/// Every path below `stage` but its log, and the names that are not UTF-8, which no list holds.
/// A link is listed, never followed.
fn contents(stage: &Path) -> Result<(Vec<Entry>, Vec<String>), RunError> {
    fn walk(
        dir: &Path,
        rel: &str,
        entries: &mut Vec<Entry>,
        odd: &mut Vec<String>,
    ) -> Result<(), RunError> {
        for item in fs::read_dir(dir).map_err(|error| io_at(dir, error))? {
            let item = item.map_err(|error| io_at(dir, error))?;
            let path = item.path();
            let name = item.file_name();
            let Some(name) = name.to_str() else {
                odd.push(format!("{rel}{}", name.to_string_lossy()));
                continue;
            };
            let rel_path = format!("{rel}{name}");
            if rel_path == LOG {
                continue;
            }
            let meta = fs::symlink_metadata(&path).map_err(|error| io_at(&path, error))?;
            let kind = if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).map_err(|error| io_at(&path, error))?;
                let Some(target) = target.to_str() else {
                    odd.push(rel_path);
                    continue;
                };
                Kind::Link(target.to_string())
            } else if meta.is_dir() {
                walk(&path, &format!("{rel_path}/"), entries, odd)?;
                Kind::Folder
            } else if meta.is_file() {
                let bytes = fs::read(&path).map_err(|error| io_at(&path, error))?;
                Kind::File(bytes_hash(&bytes))
            } else {
                Kind::Other
            };
            entries.push(Entry {
                path: rel_path,
                kind,
            });
        }
        Ok(())
    }
    let mut entries = Vec::new();
    let mut odd = Vec::new();
    walk(stage, "", &mut entries, &mut odd)?;
    Ok((entries, odd))
}

/// The ids among `ids` that `parts.lock` lists and that have no folder.
pub(super) fn lock_holes(root: &Path, ids: &[String]) -> Vec<String> {
    let paths = Paths::new(root);
    let Ok(text) = fs::read_to_string(paths.lock()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let id = line.split_whitespace().next()?;
            (ids.iter().any(|planned| planned == id) && !present(&paths.dest(id)))
                .then(|| id.to_string())
        })
        .collect()
}

fn done(log: &Log, replaced: &[String]) -> Done {
    let summary = journal::summary(log, replaced);
    let kept = log.records.iter().any(|record| {
        matches!(
            record,
            Record::Kept(_) | Record::LockKept | Record::LockStays
        )
    });
    if log.committed() {
        return Done {
            line: summary.unwrap_or_default(),
            undone: None,
            kept,
            planned: log.plan.ids.clone(),
        };
    }
    let line = match &summary {
        Some(lines) => format!("rolled back an interrupted add: {lines}"),
        None => "deleted the staging folder of an interrupted add, which leaves the project as it was before that add"
            .into(),
    };
    Done {
        line,
        undone: summary,
        kept,
        planned: log.plan.ids.clone(),
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
        planned: Vec::new(),
    })
}

fn halt(message: String) -> Stop {
    Stop::Failed(RunError { message })
}

fn io_stop(log: &Log, start: usize, stop: Stop, again: &str) -> Stop {
    match stop {
        Stop::Failed(error) => stopped(log, start, error, again),
        killed => killed,
    }
}

/// An I/O error part way through recovery. The log keeps every step done so far.
fn stopped(log: &Log, start: usize, error: RunError, again: &str) -> Stop {
    let taken = journal::steps_taken(log, start);
    let mut message = format!("{error}\norbe stopped there, and parts/{STAGE} keeps its log");
    if !taken.is_empty() {
        message.push_str(&format!(". before it stopped, orbe {}", taken.join(", ")));
    }
    message.push_str(&format!(
        "\n  hint: fix the error above, then run `{again}` again. orbe picks up from the log"
    ));
    halt(message)
}

fn observe(paths: &Paths, plan: &Plan) -> Seen {
    let odd: Vec<String> = ["new", "old"]
        .into_iter()
        .filter(|name| {
            fs::symlink_metadata(paths.stage.join(name)).is_ok_and(|meta| !meta.is_dir())
        })
        .map(str::to_string)
        .collect();
    let inside = |dir: &str, id: &str| {
        if odd.iter().any(|odd| odd == dir) {
            None
        } else {
            fingerprint(&paths.stage.join(dir).join(id))
        }
    };
    Seen {
        parts: plan
            .ids
            .iter()
            .map(|id| {
                let here = Here {
                    dest: fingerprint(&paths.dest(id)),
                    new: inside("new", id),
                    old: inside("old", id),
                };
                (id.clone(), here)
            })
            .collect(),
        lock: fingerprint(&paths.lock()),
        saved_lock: fingerprint(&paths.saved_lock()),
        names: names(&paths.stage),
        odd,
    }
}

/// Every name in the staging folder, with the names inside `new/` and `old/` when they are
/// folders. A link is listed, never followed.
fn names(stage: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for name in entries(stage) {
        let folder = fs::symlink_metadata(stage.join(&name)).is_ok_and(|meta| meta.is_dir());
        if (name == "new" || name == "old") && folder {
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

/// Each effect first checks its paths hold what [`journal::next`] saw, without following links.
fn execute(paths: &Paths, effect: &Effect) -> Result<(), RunError> {
    match effect {
        Effect::None => Ok(()),
        Effect::Unplace(id, hash) => {
            gate(&paths.dest(id), Some(hash))?;
            gate(&paths.new_copy(id), None)?;
            let new = paths.stage.join("new");
            if !present(&new) {
                fs::create_dir(&new).map_err(|error| io_at(&new, error))?;
            }
            mv(&paths.dest(id), &paths.new_copy(id))
        }
        Effect::Restore(id, hash) => {
            gate(&paths.old_copy(id), Some(hash))?;
            gate(&paths.dest(id), None)?;
            mv(&paths.old_copy(id), &paths.dest(id))
        }
        Effect::LockBack(saved, live) => {
            gate(&paths.saved_lock(), Some(saved))?;
            gate(&paths.lock(), live.as_ref())?;
            let bytes =
                fs::read(paths.saved_lock()).map_err(|error| io_at(&paths.saved_lock(), error))?;
            write_lock(paths, &bytes)
        }
        Effect::LockRemove(hash) => {
            gate(&paths.lock(), Some(hash))?;
            fs::remove_file(paths.lock()).map_err(|error| io_at(&paths.lock(), error))
        }
    }
}

fn gate(path: &Path, want: Option<&Hash>) -> Result<(), RunError> {
    if fingerprint(path).as_ref() == want {
        return Ok(());
    }
    Err(RunError {
        message: format!(
            "{} changed while orbe was at work, so orbe left it",
            path.display()
        ),
    })
}

/// What the next writing run does with what an interrupted add left. `--diff` writes nothing,
/// so it only says this.
pub(super) enum Pending {
    None,
    /// A file or folder that holds nothing of the add's, which the next run deletes.
    Leftover(String),
    Uncommitted,
    Committed,
    /// The next run finishes deleting the staging folder.
    Clearing,
    /// The next run names what it cannot prove the add made, and stops.
    Stops,
}

/// The same checks [`settle_stage`] makes before it changes anything.
pub(super) fn pending(root: &Path) -> Pending {
    let paths = Paths::new(root);
    if !present(&paths.stage) {
        return if present(&paths.lock_tmp()) {
            Pending::Leftover(LOCK_TMP.into())
        } else {
            Pending::None
        };
    }
    if !fs::symlink_metadata(&paths.stage).is_ok_and(|meta| meta.is_dir()) {
        return Pending::Stops;
    }
    let read = match fs::symlink_metadata(paths.log()) {
        Ok(meta) if meta.is_file() => journal::read(&log_text(&paths.log())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Read::Torn,
        _ => return Pending::Stops,
    };
    match read {
        Read::Log(log) => match log.clearing() {
            Some(listed) => match unlisted(&paths.stage, listed) {
                Ok((_, unlisted)) if unlisted.is_empty() => Pending::Clearing,
                _ => Pending::Stops,
            },
            None if journal::stops(&log, &observe(&paths, &log.plan)) => Pending::Stops,
            None if log.committed() => Pending::Committed,
            None => Pending::Uncommitted,
        },
        Read::Torn if names(&paths.stage).iter().all(|name| name == LOG) => {
            Pending::Leftover(format!("parts/{STAGE}"))
        }
        _ => Pending::Stops,
    }
}

/// The text of a log that is a file. A link is never read.
fn log_text(path: &Path) -> String {
    let bytes = fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.is_file())
        .then(|| fs::read(path).ok())
        .flatten()
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
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
