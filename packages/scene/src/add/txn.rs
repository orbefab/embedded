//! One `orbe add` is one transaction. New folders are staged inside `parts/.orbe-add/`, so every
//! rename stays on the filesystem that holds `parts/`. The originals move aside into
//! `parts/.orbe-add/old/` when one was already there, the new folders move in, and the log is
//! marked committed. Only then are those originals deleted. A lock is written only when the
//! change names one.
//!
//! Every step appends a line to `parts/.orbe-add/log`, and recovery ([`journal::next`]) acts
//! only on what that log proves. A run that stops before the commit is rolled back, by itself or
//! by the next run. A run that stops after it is finished by the next. Every move and delete
//! first checks that its path holds the hash the log recorded. Before the staging folder is
//! deleted, the log lists every path in it with its hash. Each path is deleted only while it
//! holds what that list says, the log goes last, and anything else stops the run before it
//! deletes a thing.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::at::{Folder, Found};
use super::journal::{
    self, Effect, Entry, Hash, Here, Kind, LockChange, Log, Next, Plan, Read, Record, Seen,
    Unlogged,
};
use super::{Again, RunError, io_at};
use crate::catalog::Catalog;
use crate::project::{hex, shell_path};

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

    fn below_stage(&self, rel: &OsStr) -> PathBuf {
        if rel.is_empty() {
            self.stage.clone()
        } else {
            self.stage.join(rel)
        }
    }
}

/// Writes the change as one transaction. On a failure it rolls back and says what it rolled
/// back. A failure after the commit only leaves the replaced copies, which the next run deletes,
/// so it returns a note instead.
pub(super) fn apply(
    root: &Path,
    change: &Change,
    again: &Again,
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
        let hash = proof(&dir).map_err(cannot_read)?.unwrap_or_default();
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
        let before = proof(&dest).map_err(cannot_read)?;
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
fn roll_back(
    paths: &Paths,
    plan: &Plan,
    error: RunError,
    again: &Again,
    fault: &mut Fault,
) -> Stop {
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
}

#[derive(Default)]
pub(super) struct Recovered {
    pub(super) lines: Vec<String>,
}

/// Rolls back an add that stopped before its commit, or finishes one that stopped after it.
pub(super) fn recover(root: &Path, again: &Again, fault: &mut Fault) -> Result<Recovered, Stop> {
    let done = settle_stage(&Paths::new(root), again, fault)?;
    Ok(done.map_or_else(Recovered::default, |done| Recovered {
        lines: vec![done.line],
    }))
}

fn settle_stage(paths: &Paths, again: &Again, fault: &mut Fault) -> Result<Option<Done>, Stop> {
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
            "parts/{STAGE} is not a folder, so no add made it\n  hint: move {} out of {}, then run `{again}` again",
            again.path(format!("parts/{STAGE}")),
            again.path("parts")
        )));
    }
    let bytes = match fs::symlink_metadata(paths.log()) {
        Ok(meta) if meta.is_file() => {
            Some(fs::read(paths.log()).map_err(|error| io_at(&paths.log(), error))?)
        }
        Ok(_) => {
            return Err(halt(format!(
                "parts/{STAGE}/log is not a file, so no add wrote it, and orbe left parts/{STAGE} as it is\n  hint: move {} out of {}, then run `{again}` again",
                again.path(format!("parts/{STAGE}/{LOG}")),
                again.path(format!("parts/{STAGE}"))
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
                "parts/{STAGE}/log is not a log this orbe writes, `{line}`, so orbe left parts/{STAGE} as it is\n  hint: move {} out of {} to keep what it holds, then run `{again}` again",
                again.path(format!("parts/{STAGE}")),
                again.path("parts")
            )));
        }
        Some(Read::UnknownStep(line)) => {
            return Err(halt(format!(
                "parts/{STAGE}/log has a line this orbe does not write, `{line}`, so orbe left parts/{STAGE} as it is\n  hint: delete that line and the lines after it from {}, then run `{again}` again. orbe then settles the add from the steps before it",
                again.path(format!("parts/{STAGE}/{LOG}"))
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
        let seen = observe(paths, &log);
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
                let entries = contents(paths).map_err(|error| io(error.into()))?;
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

/// Deletes what the clearing record lists, files and links first and then folders, deepest
/// first. The log goes last, then the folder. Anything else below the folder stops the run
/// before it deletes a thing, and is named. A path that changed since the list stops the run
/// right before its own delete.
fn clear(
    paths: &Paths,
    log: &Log,
    start: usize,
    again: &Again,
    fault: &mut Fault,
) -> Result<Done, Stop> {
    let io = |path: &Path, error| io_stop(log, start, io_at(path, error).into(), again);
    let stage = &Folder::open(&paths.stage).map_err(|error| io(&paths.stage, error))?;
    let parts = &fs::metadata(&paths.parts).map_err(|error| io(&paths.parts, error))?;
    let now = delete_listed(paths, stage, log, start, again, fault).map_err(|stop| {
        match (&stop, swapped(paths, parts, stage, again)) {
            (Stop::Failed(_), Some(text)) => halt(text),
            _ => stop,
        }
    })?;
    let last = |error: RunError| halt(log_gone_text(stage, &error, again));
    at(fault, &format!("do delete parts/{STAGE}")).map_err(|stop| match stop {
        Stop::Failed(error) => last(error),
        killed => killed,
    })?;
    if let Some(text) = swapped(paths, parts, stage, again) {
        return Err(halt(text));
    }
    fs::remove_dir(&paths.stage).map_err(|error| last(io_at(&paths.stage, error)))?;
    if log.plan.parts_created && !log.committed() {
        let _ = fs::remove_dir(&paths.parts);
    }
    let replaced: Vec<String> = journal::set_aside(log)
        .into_iter()
        .filter(|id| now.iter().any(|entry| *entry.path == *format!("old/{id}")))
        .collect();
    Ok(done(paths, log, &replaced))
}

/// Deletes every path below `stage` the clearing record lists, and then the log, through the
/// folders orbe opened. It returns what was there before.
fn delete_listed(
    paths: &Paths,
    stage: &Folder,
    log: &Log,
    start: usize,
    again: &Again,
    fault: &mut Fault,
) -> Result<Vec<Entry>, Stop> {
    let listed = log.clearing().unwrap_or_default();
    let io = |stop| io_stop(log, start, stop, again);
    let (now, unlisted) = unlisted(paths, stage, listed).map_err(|error| io(error.into()))?;
    if !unlisted.is_empty() {
        return Err(halt(unlisted_text(&unlisted, again)));
    }
    let (mut folders, rest): (Vec<&Entry>, Vec<&Entry>) =
        now.iter().partition(|entry| entry.kind == Kind::Folder);
    folders.sort_by(|a, b| b.path.cmp(&a.path));
    for entry in rest.into_iter().chain(folders) {
        let shown = paths.below_stage(&entry.path);
        at(
            fault,
            &format!("do delete parts/{STAGE}/{}", entry.path.display()),
        )
        .map_err(io)?;
        let (folder, name) = match stage.parent_of(&entry.path) {
            Ok(found) => found,
            Err((_, error)) if error.kind() == io::ErrorKind::NotFound => continue,
            Err((on_the_way, error)) if moved(&error) => {
                return Err(halt(changed_text(on_the_way, again)));
            }
            Err((_, error)) => return Err(io(io_at(&shown, error).into())),
        };
        match folder.find(name).map(|found| kind(&found)) {
            Ok(kind) if kind == entry.kind => {}
            Ok(_) => return Err(halt(changed_text(&entry.path, again))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io(io_at(&shown, error).into())),
        }
        if entry.kind == Kind::Folder {
            folder.remove_folder(name)
        } else {
            folder.remove_file(name)
        }
        .map_err(|error| io(io_at(&shown, error).into()))?;
    }
    at(fault, &format!("do delete parts/{STAGE}/{LOG}")).map_err(io)?;
    stage
        .remove_file(OsStr::new(LOG))
        .map_err(|error| io(io_at(&paths.log(), error).into()))?;
    Ok(now)
}

/// What to say when `parts/.orbe-add` no longer leads to the folder orbe opened and deleted in,
/// because it, or `parts`, was moved or swapped for a link. A hint then names only that path,
/// never one through it. `None` while it still leads there. `parts` is what `parts` was then.
fn swapped(paths: &Paths, parts: &fs::Metadata, stage: &Folder, again: &Again) -> Option<String> {
    let same = |now: &fs::Metadata| now.dev() == parts.dev() && now.ino() == parts.ino();
    if !fs::metadata(&paths.parts).is_ok_and(|now| same(&now)) {
        return Some(format!(
            "parts is not the folder orbe was deleting parts/{STAGE} in, so orbe stopped there\n  hint: put the project's parts folder back at {}, then run `{again}` again",
            again.path("parts")
        ));
    }
    match fs::symlink_metadata(&paths.stage) {
        Ok(meta) if meta.is_dir() && stage.is(&meta) => None,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(format!(
            "parts/{STAGE} was moved while orbe deleted what the add left in it, so orbe stopped there\n  hint: run `{again}` again"
        )),
        _ => Some(format!(
            "parts/{STAGE} was replaced while orbe deleted what the add left in it, and what is there now is not the folder orbe was deleting, so orbe stopped there and left it\n  hint: move {} out of {} to keep it, then run `{again}` again",
            again.path(format!("parts/{STAGE}")),
            again.path("parts")
        )),
    }
}

/// A folder on the way to a path was swapped for something else, such as a link.
fn moved(error: &io::Error) -> bool {
    matches!(
        rustix::io::Errno::from_io_error(error),
        Some(rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR)
    )
}

fn changed_text(rel: &OsStr, again: &Again) -> String {
    let path = journal::staged_paths(again, &[rel]);
    format!(
        "{path} changed after orbe listed it to delete, so orbe left it. orbe stopped there, and parts/{STAGE} keeps its log\n  hint: move {path} out of {} to keep it, then run `{again}` again",
        again.path(format!("parts/{STAGE}"))
    )
}

/// The last delete, the staging folder itself, failed after its log was gone.
fn log_gone_text(stage: &Folder, error: &RunError, again: &Again) -> String {
    let names = stage.names().unwrap_or_default();
    let left = if names.is_empty() {
        "is left as an empty folder".to_string()
    } else {
        format!(
            "now holds {}, which orbe did not list",
            journal::staged_paths(again, &names)
        )
    };
    format!(
        "{error}\norbe had deleted its log and everything else in parts/{STAGE}, which {left}\n  hint: fix the error above, then run `{again}` again"
    )
}

/// What is below the staging folder now, and the paths of it `listed` does not hold.
fn unlisted(
    paths: &Paths,
    stage: &Folder,
    listed: &[Entry],
) -> Result<(Vec<Entry>, Vec<OsString>), RunError> {
    let now = walk(paths, stage)?;
    let unlisted = now
        .iter()
        .filter(|entry| !listed.contains(entry))
        .map(|entry| entry.path.clone())
        .collect();
    Ok((now, unlisted))
}

fn unlisted_text(names: &[OsString], again: &Again) -> String {
    let mut tops: Vec<&OsString> = names
        .iter()
        .filter(|name| {
            !names.iter().any(|other| {
                let other = other.as_bytes();
                name.as_bytes().starts_with(other)
                    && name.as_bytes().get(other.len()) == Some(&b'/')
            })
        })
        .collect();
    tops.sort();
    let listed = journal::staged_paths(again, &tops);
    let them = if tops.len() == 1 { "it" } else { "them" };
    format!(
        "{listed} {} not on the list the add made before it began to delete parts/{STAGE}, so orbe cannot tell that the add made {them}. orbe deleted nothing, and left parts/{STAGE} and its log\n  hint: move {listed} out of {} to keep {them}, then run `{again}` again",
        if tops.len() == 1 { "is" } else { "are" },
        again.path(format!("parts/{STAGE}"))
    )
}

/// Every path below `stage` but its log. A link is listed, never followed.
fn contents(paths: &Paths) -> Result<Vec<Entry>, RunError> {
    let stage = Folder::open(&paths.stage).map_err(|error| io_at(&paths.stage, error))?;
    walk(paths, &stage)
}

fn walk(paths: &Paths, stage: &Folder) -> Result<Vec<Entry>, RunError> {
    let found = stage
        .walk(Some(OsStr::new(LOG)))
        .map_err(|(rel, error)| io_at(&paths.below_stage(&rel), error))?;
    Ok(found
        .into_iter()
        .map(|(path, found)| Entry {
            path,
            kind: kind(&found),
        })
        .collect())
}

fn kind(found: &Found) -> Kind {
    match found {
        Found::Folder => Kind::Folder,
        Found::File(bytes) => Kind::File(bytes_hash(bytes)),
        Found::Link(target) => Kind::Link(target.clone()),
        Found::Other => Kind::Other,
    }
}

/// `replaced` are the ids whose old copies this run deleted.
fn done(paths: &Paths, log: &Log, replaced: &[String]) -> Done {
    let kept = log.records.iter().any(|record| {
        matches!(
            record,
            Record::Kept(_) | Record::LockKept | Record::LockStays
        )
    });
    if log.committed() {
        let (ids, lock) = journal::written(log);
        let mut wrote: Vec<String> = ids
            .iter()
            .filter(|(id, placed)| fingerprint(&paths.dest(id)).as_ref() == Some(placed))
            .map(|(id, _)| format!("parts/{id}"))
            .collect();
        if lock.is_some_and(|hash| fingerprint(&paths.lock()).as_ref() == Some(hash)) {
            wrote.push("parts.lock".into());
        }
        return Done {
            line: journal::finished(log, &wrote, replaced)
                .unwrap_or_else(|| "deleted the staging folder of an interrupted add".into()),
            undone: None,
            kept,
        };
    }
    let summary = journal::summary(log);
    let line = match &summary {
        Some(lines) => format!("rolled back an interrupted add: {lines}"),
        None => "deleted the staging folder of an interrupted add, which leaves the project as it was before that add"
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
fn without_log(paths: &Paths, again: &Again, fault: &mut Fault) -> Result<Done, Stop> {
    let names = names(&paths.stage);
    if names.iter().any(|name| name != LOG) {
        let listed = journal::staged_paths(again, &names);
        let stage = again.path(format!("parts/{STAGE}"));
        return Err(halt(format!(
            "parts/{STAGE} has no log, so orbe cannot tell what the add that left it changed, and it left {listed} as they are. that add may have written folders in parts/\n  hint: move anything you want to keep out of {stage}, delete {stage}, then run `{again}` again"
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

fn io_stop(log: &Log, start: usize, stop: Stop, again: &Again) -> Stop {
    match stop {
        Stop::Failed(error) => stopped(log, start, error, again),
        killed => killed,
    }
}

/// An I/O error part way through recovery. The log keeps every step done so far.
fn stopped(log: &Log, start: usize, error: RunError, again: &Again) -> Stop {
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

fn observe(paths: &Paths, log: &Log) -> Seen {
    let plan = &log.plan;
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
        lock: None,
        saved_lock: None,
        names: names(&paths.stage),
        odd,
        unlogged_is_prefix: unlogged_is_prefix(paths, log),
    }
}

/// Whether the write after the log's last line holds a start of what the add writes there:
/// the catalog's files for the id, or the live `parts.lock` it copies. A kill part way leaves
/// a start of them.
fn unlogged_is_prefix(paths: &Paths, log: &Log) -> bool {
    match log.unlogged() {
        Some(Unlogged::New(id)) => {
            let catalog = Catalog::builtin();
            catalog
                .versions_of(&id)
                .into_iter()
                .filter_map(|version| catalog.get(&id, version))
                .any(|entry| starts_of(&paths.new_copy(&id), &entry.files))
        }
        Some(Unlogged::SavedLock) => {
            let saved =
                Folder::open(&paths.stage).and_then(|stage| stage.find(OsStr::new(SAVED_LOCK)));
            match (saved, fs::read(paths.lock())) {
                (Ok(Found::File(saved)), Ok(live)) => live.starts_with(&saved),
                _ => false,
            }
        }
        None => false,
    }
}

/// Whether the folder at `dir` holds only starts of `files`, by their paths below it.
fn starts_of(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> bool {
    let Some(found) = Folder::open(dir)
        .ok()
        .and_then(|folder| folder.walk(None).ok())
    else {
        return false;
    };
    found.iter().all(|(path, found)| {
        let Some(path) = path.to_str() else {
            return false;
        };
        match found {
            Found::File(bytes) => files
                .get(path)
                .is_some_and(|whole| whole.starts_with(bytes.as_slice())),
            Found::Folder => files
                .keys()
                .any(|file| file.starts_with(&format!("{path}/"))),
            _ => false,
        }
    })
}

/// Every name in the staging folder, with the names inside `new/` and `old/` when they are
/// folders. A link is listed, never followed.
fn names(stage: &Path) -> Vec<OsString> {
    let mut names = Vec::new();
    for name in entries(stage) {
        let folder = fs::symlink_metadata(stage.join(&name)).is_ok_and(|meta| meta.is_dir());
        if (name == "new" || name == "old") && folder {
            names.extend(entries(&stage.join(&name)).into_iter().map(|inner| {
                let mut path = name.clone();
                path.push("/");
                path.push(inner);
                path
            }));
        }
        names.push(name);
    }
    names.sort();
    names
}

fn entries(dir: &Path) -> Vec<OsString> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| Some(entry.ok()?.file_name()))
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
            shell_path(path.as_os_str())
        ),
    })
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
/// nothing is there. Anything below it that cannot be read gives [`UNREADABLE`], which no log
/// line can hold, so it never matches a hash the log recorded.
pub(super) fn fingerprint(path: &Path) -> Option<Hash> {
    proof(path).unwrap_or_else(|_| Some(UNREADABLE.into()))
}

const UNREADABLE: &str = "unreadable";

fn cannot_read(path: PathBuf) -> RunError {
    RunError {
        message: format!(
            "cannot read {}, so orbe could not tell later that it is unchanged",
            shell_path(path.as_os_str())
        ),
    }
}

/// The hash [`fingerprint`] gives, or the first path below `path` it cannot read.
fn proof(path: &Path) -> Result<Option<Hash>, PathBuf> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(path.into()),
    };
    let mut lines = Vec::new();
    describe(path, ".", &meta, &mut lines)?;
    Ok(Some(digest(lines)))
}

fn describe(
    path: &Path,
    rel: &str,
    meta: &fs::Metadata,
    lines: &mut Vec<String>,
) -> Result<(), PathBuf> {
    let cannot = || path.to_path_buf();
    let kind = meta.file_type();
    if kind.is_symlink() {
        let target = fs::read_link(path).map_err(|_| cannot())?;
        lines.push(format!("link {rel} {}", target.display()));
    } else if kind.is_dir() {
        lines.push(format!("dir {rel}"));
        for entry in fs::read_dir(path).map_err(|_| cannot())? {
            let entry = entry.map_err(|_| cannot())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if rel == "." {
                name
            } else {
                format!("{rel}/{name}")
            };
            let meta = fs::symlink_metadata(entry.path()).map_err(|_| entry.path())?;
            describe(&entry.path(), &child, &meta, lines)?;
        }
    } else if kind.is_file() {
        lines.push(file_line(rel, &fs::read(path).map_err(|_| cannot())?));
    } else {
        lines.push(format!("other {rel}"));
    }
    Ok(())
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
            shell_path(from.as_os_str()),
            shell_path(to.as_os_str())
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
