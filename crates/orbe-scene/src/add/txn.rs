//! One `orbe add` is one transaction. New folders are staged inside `parts/.orbe-add/`, so every
//! rename stays on the filesystem that holds `parts/`. A journal lists the planned moves before
//! the first one. The originals move aside, the new folders move in, the lock is replaced, and
//! the journal is marked committed. Only then are the originals deleted.
//!
//! A run that stops before the mark is rolled back, by this run or by the next. A run that
//! stops after it is finished by the next. A user's folder is deleted only after the mark.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::{RunError, io_at};

/// The staging folder, inside `parts/`.
pub(super) const STAGE: &str = ".orbe-add";
/// The new lock before it is renamed over `parts.lock`, beside it at the project root.
const LOCK_TMP: &str = ".parts.lock.orbe-add";
const JOURNAL: &str = "journal";
const COMMITTED: &str = "committed";
const SAVED_LOCK: &str = "parts.lock";
const HEADER: &str = "orbe add journal 1";

/// The folders to place, by id, and the new lock text when it changes.
pub(super) struct Change<'a> {
    pub(super) folders: Vec<(&'a str, &'a BTreeMap<String, Vec<u8>>)>,
    pub(super) lock: Option<&'a str>,
}

/// One step of [`apply`], in order. A fault hook sees each step's index before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Stage,
    SaveLock,
    Journal,
    Aside(usize),
    Place(usize),
    WriteLock,
    RenameLock,
    Commit,
    Cleanup,
}

/// What a test injects before a step. `Kill` stops as a killed process would, with no undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code, reason = "only tests inject faults"))]
pub(super) enum Inject {
    Fail,
    Kill,
}

pub(super) enum Stop {
    /// The step failed, and the message says what was rolled back.
    Failed(RunError),
    Killed,
}

pub(super) fn steps(change: &Change) -> Vec<Step> {
    let mut steps = Vec::new();
    if !change.folders.is_empty() {
        steps.push(Step::Stage);
        if change.lock.is_some() {
            steps.push(Step::SaveLock);
        }
        steps.push(Step::Journal);
        for index in 0..change.folders.len() {
            steps.push(Step::Aside(index));
            steps.push(Step::Place(index));
        }
    }
    if change.lock.is_some() {
        steps.push(Step::WriteLock);
        steps.push(Step::RenameLock);
    }
    if !change.folders.is_empty() {
        steps.push(Step::Commit);
        steps.push(Step::Cleanup);
    }
    steps
}

/// The lock as it was before the add, as the journal records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockBefore {
    /// The add does not change the lock.
    Kept,
    /// There was no lock.
    Absent,
    /// The old lock is saved in the staging folder.
    Saved,
}

#[derive(Debug, PartialEq, Eq)]
struct Journal {
    parts_created: bool,
    lock: LockBefore,
    ids: Vec<String>,
}

impl Journal {
    fn text(&self) -> String {
        let mut text = format!(
            "{HEADER}\nparts {}\nlock {}\n",
            if self.parts_created {
                "created"
            } else {
                "existed"
            },
            match self.lock {
                LockBefore::Kept => "kept",
                LockBefore::Absent => "absent",
                LockBefore::Saved => "saved",
            }
        );
        for id in &self.ids {
            text.push_str(&format!("part {id}\n"));
        }
        text
    }

    fn parse(text: &str) -> Option<Journal> {
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let parts_created = match lines.next()? {
            "parts created" => true,
            "parts existed" => false,
            _ => return None,
        };
        let lock = match lines.next()? {
            "lock kept" => LockBefore::Kept,
            "lock absent" => LockBefore::Absent,
            "lock saved" => LockBefore::Saved,
            _ => return None,
        };
        let mut ids = Vec::new();
        for line in lines {
            let id = line.strip_prefix("part ")?;
            if !super::single_component(id) {
                return None;
            }
            ids.push(id.to_string());
        }
        Some(Journal {
            parts_created,
            lock,
            ids,
        })
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
}

/// Writes the change as one transaction. On a failure it rolls back and says what it rolled
/// back. A failure after the commit mark only leaves the replaced copies to delete, which the
/// next run does, so it returns a note instead.
pub(super) fn apply(
    root: &Path,
    change: &Change,
    fault: &mut dyn FnMut(usize) -> Option<Inject>,
) -> Result<Vec<String>, Stop> {
    let paths = Paths::new(root);
    let journal = Journal {
        parts_created: !change.folders.is_empty() && !paths.parts.exists(),
        lock: match change.lock {
            None => LockBefore::Kept,
            Some(_) if paths.lock().exists() => LockBefore::Saved,
            Some(_) => LockBefore::Absent,
        },
        ids: change
            .folders
            .iter()
            .map(|(id, _)| (*id).to_string())
            .collect(),
    };
    let mut journaled = false;
    for (index, step) in steps(change).into_iter().enumerate() {
        let result = match fault(index) {
            Some(Inject::Kill) => return Err(Stop::Killed),
            Some(Inject::Fail) => Err(RunError {
                message: format!("injected failure at {step:?}"),
            }),
            None => run_step(&paths, change, &journal, step),
        };
        let Err(error) = result else {
            journaled |= step == Step::Journal;
            continue;
        };
        if step == Step::Cleanup {
            return Ok(vec![format!(
                "the copies this add replaced stay in parts/{STAGE}, because {error}. the next `orbe add` deletes them"
            )]);
        }
        if !journaled {
            let undone = discard(&paths, journal.parts_created);
            return Err(Stop::Failed(RunError {
                message: match undone {
                    Ok(()) => format!("{error}\nnothing was written"),
                    Err(also) => format!("{error}\nnothing was written, and {also}"),
                },
            }));
        }
        let message = match roll_back(&paths, &journal) {
            Ok(lines) if lines.is_empty() => format!("{error}\nnothing was written"),
            Ok(lines) => format!(
                "{error}\nrolled back, so the project is as it was: {}",
                lines.join(", ")
            ),
            Err(also) => format!(
                "{error}\nthe rollback stopped: {also}\nthe journal in parts/{STAGE} stays, and the next `orbe add` rolls it back"
            ),
        };
        return Err(Stop::Failed(RunError { message }));
    }
    Ok(Vec::new())
}

fn run_step(paths: &Paths, change: &Change, journal: &Journal, step: Step) -> Result<(), RunError> {
    match step {
        Step::Stage => {
            for dir in [paths.stage.join("new"), paths.stage.join("old")] {
                fs::create_dir_all(&dir).map_err(|error| io_at(&dir, error))?;
            }
            for (id, files) in &change.folders {
                write_files(&paths.new_copy(id), files)?;
            }
            Ok(())
        }
        Step::SaveLock => {
            if journal.lock == LockBefore::Saved {
                let saved = paths.stage.join(SAVED_LOCK);
                fs::copy(paths.lock(), &saved).map_err(|error| io_at(&saved, error))?;
            }
            Ok(())
        }
        Step::Journal => {
            let tmp = paths.stage.join("journal.tmp");
            write_synced(&tmp, journal.text().as_bytes())?;
            mv(&tmp, &paths.stage.join(JOURNAL))
        }
        Step::Aside(index) => {
            let id = change.folders[index].0;
            let dest = paths.dest(id);
            if present(&dest) {
                mv(&dest, &paths.old_copy(id))?;
            }
            Ok(())
        }
        Step::Place(index) => {
            let id = change.folders[index].0;
            mv(&paths.new_copy(id), &paths.dest(id))
        }
        Step::WriteLock => write_synced(
            &paths.root.join(LOCK_TMP),
            change.lock.unwrap_or_default().as_bytes(),
        ),
        Step::RenameLock => mv(&paths.root.join(LOCK_TMP), &paths.lock()),
        Step::Commit => {
            let tmp = paths.stage.join("committed.tmp");
            write_synced(&tmp, b"committed\n")?;
            mv(&tmp, &paths.stage.join(COMMITTED))
        }
        Step::Cleanup => finish(paths).map(|_| ()),
    }
}

/// Before the journal exists nothing has moved, so the staging folder holds only new copies
/// and a copy of the lock, and the lock's temporary file is the only other write.
fn discard(paths: &Paths, parts_created: bool) -> Result<(), RunError> {
    let tmp = paths.root.join(LOCK_TMP);
    if present(&tmp) {
        fs::remove_file(&tmp).map_err(|error| io_at(&tmp, error))?;
    }
    if present(&paths.stage) {
        fs::remove_dir_all(&paths.stage).map_err(|error| io_at(&paths.stage, error))?;
    }
    if parts_created {
        let _ = fs::remove_dir(&paths.parts);
    }
    Ok(())
}

/// An add that a previous run left in `parts/.orbe-add`, before this run plans anything.
pub(super) enum Pending {
    None,
    /// Stopped before the commit mark. The next writing run rolls it back.
    Uncommitted,
    /// Stopped after it. The next writing run deletes the copies it replaced.
    Committed,
}

pub(super) fn pending(root: &Path) -> Pending {
    let paths = Paths::new(root);
    if !present(&paths.stage) && !present(&root.join(LOCK_TMP)) {
        Pending::None
    } else if present(&paths.stage.join(COMMITTED)) && present(&paths.stage.join(JOURNAL)) {
        Pending::Committed
    } else {
        Pending::Uncommitted
    }
}

/// Rolls back an add that stopped before its commit mark, or finishes one that stopped after
/// it. Returns what it did, one line each, or nothing when no add was pending.
pub(super) fn recover(root: &Path) -> Result<Vec<String>, RunError> {
    let paths = Paths::new(root);
    let mut lines = Vec::new();
    let tmp = root.join(LOCK_TMP);
    if !present(&paths.stage) {
        if present(&tmp) {
            fs::remove_file(&tmp).map_err(|error| io_at(&tmp, error))?;
            lines.push(format!(
                "deleted {LOCK_TMP}, a lock an interrupted add had not put in place"
            ));
        }
        return Ok(lines);
    }
    let journal_path = paths.stage.join(JOURNAL);
    if !present(&journal_path) {
        if has_entries(&paths.stage.join("old"))? {
            return Err(RunError {
                message: format!(
                    "parts/{STAGE}/old holds folders and there is no journal, so orbe will not touch them. move each back to parts/ by hand, then delete parts/{STAGE}"
                ),
            });
        }
        fs::remove_dir_all(&paths.stage).map_err(|error| io_at(&paths.stage, error))?;
        lines
            .push("deleted the staging folder of an interrupted add that had moved nothing".into());
        return Ok(lines);
    }
    let text = fs::read_to_string(&journal_path).map_err(|error| io_at(&journal_path, error))?;
    let Some(journal) = Journal::parse(&text) else {
        return Err(RunError {
            message: format!(
                "parts/{STAGE}/journal is not a journal this orbe writes, so orbe will not touch parts/{STAGE}. move the folders in parts/{STAGE}/old back by hand, then delete parts/{STAGE}"
            ),
        });
    };
    if present(&paths.stage.join(COMMITTED)) {
        let replaced = finish(&paths)?;
        lines.push(if replaced.is_empty() {
            "finished an interrupted add".into()
        } else {
            format!(
                "finished an interrupted add, and deleted the copies it replaced: {}",
                replaced
                    .iter()
                    .map(|id| format!("parts/{id}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
        return Ok(lines);
    }
    let undone = roll_back(&paths, &journal)?;
    lines.push(if undone.is_empty() {
        "rolled back an interrupted add that had changed nothing".into()
    } else {
        format!("rolled back an interrupted add: {}", undone.join(", "))
    });
    Ok(lines)
}

/// Undoes the journal's moves in reverse. A folder is only ever renamed here, never deleted,
/// until every saved original is back in place.
fn roll_back(paths: &Paths, journal: &Journal) -> Result<Vec<String>, RunError> {
    let mut lines = Vec::new();
    for id in journal.ids.iter().rev() {
        let dest = paths.dest(id);
        let new = paths.new_copy(id);
        let old = paths.old_copy(id);
        let placed = !present(&new) && present(&dest);
        if placed {
            mv(&dest, &new)?;
        }
        if present(&old) {
            if present(&dest) {
                return Err(RunError {
                    message: format!(
                        "parts/{id} is in the way of the saved copy at parts/{STAGE}/old/{id}, so it stays there"
                    ),
                });
            }
            mv(&old, &dest)?;
            lines.push(format!("put back parts/{id}"));
        } else if placed {
            lines.push(format!("removed the new parts/{id}"));
        }
    }
    match journal.lock {
        LockBefore::Kept => {}
        LockBefore::Absent => {
            if present(&paths.lock()) {
                fs::remove_file(paths.lock()).map_err(|error| io_at(&paths.lock(), error))?;
                lines.push("removed the new parts.lock".into());
            }
        }
        LockBefore::Saved => {
            let saved_path = paths.stage.join(SAVED_LOCK);
            let saved = fs::read(&saved_path).map_err(|error| io_at(&saved_path, error))?;
            if fs::read(paths.lock()).ok().as_deref() != Some(saved.as_slice()) {
                let tmp = paths.root.join(LOCK_TMP);
                write_synced(&tmp, &saved)?;
                mv(&tmp, &paths.lock())?;
                lines.push("put back parts.lock".into());
            }
        }
    }
    let tmp = paths.root.join(LOCK_TMP);
    if present(&tmp) {
        fs::remove_file(&tmp).map_err(|error| io_at(&tmp, error))?;
    }
    if has_entries(&paths.stage.join("old"))? {
        return Err(RunError {
            message: format!("parts/{STAGE}/old still holds folders, so it stays"),
        });
    }
    fs::remove_dir_all(&paths.stage).map_err(|error| io_at(&paths.stage, error))?;
    if journal.parts_created && !has_entries(&paths.parts)? {
        let _ = fs::remove_dir(&paths.parts);
    }
    Ok(lines)
}

/// Deletes what a committed add replaced, then the staging folder. The journal and the mark
/// go last, so a stop part way through is finished by the next run. Returns the ids whose
/// replaced copies it deleted.
fn finish(paths: &Paths) -> Result<Vec<String>, RunError> {
    let old = paths.stage.join("old");
    let mut replaced = Vec::new();
    if present(&old) {
        for entry in fs::read_dir(&old).map_err(|error| io_at(&old, error))? {
            let entry = entry.map_err(|error| io_at(&old, error))?;
            replaced.push(entry.file_name().to_string_lossy().to_string());
        }
        replaced.sort();
        for id in &replaced {
            remove_any(&old.join(id))?;
        }
    }
    for name in [
        "old",
        "new",
        SAVED_LOCK,
        "journal.tmp",
        "committed.tmp",
        JOURNAL,
        COMMITTED,
    ] {
        let path = paths.stage.join(name);
        if present(&path) {
            remove_any(&path)?;
        }
    }
    fs::remove_dir(&paths.stage).map_err(|error| io_at(&paths.stage, error))?;
    Ok(replaced)
}

fn write_files(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), RunError> {
    fs::create_dir_all(dir).map_err(|error| io_at(dir, error))?;
    for (rel, bytes) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| io_at(parent, error))?;
        }
        fs::write(&path, bytes).map_err(|error| io_at(&path, error))?;
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), RunError> {
    let mut file = fs::File::create(path).map_err(|error| io_at(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_at(path, error))
}

fn mv(from: &Path, to: &Path) -> Result<(), RunError> {
    fs::rename(from, to).map_err(|error| RunError {
        message: format!(
            "cannot move {} to {}: {error}",
            from.display(),
            to.display()
        ),
    })
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

fn has_entries(dir: &Path) -> Result<bool, RunError> {
    match fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_at(dir, error)),
    }
}
