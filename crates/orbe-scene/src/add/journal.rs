//! The log of one `orbe add`, and what recovery does next.
//!
//! The log is append-only. Each line records a step that has completed, so it proves that step
//! and nothing more. Only the step after the last line can have run without its line, and
//! [`next`] settles that one by comparing hashes on disk with the hashes the log recorded.
//! Recovery logs its own steps the same way, so a recovery that stops part way is resumed.

use std::collections::BTreeMap;

use super::single_component;

pub(super) const HEADER: &str = "orbe add log 1";
pub(super) const STAGE: &str = ".orbe-add";

/// A fingerprint of whatever is at a path: a folder, a file or a link.
pub(super) type Hash = String;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Plan {
    pub(super) parts_created: bool,
    pub(super) lock: LockChange,
    pub(super) ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LockChange {
    Kept,
    /// There was no `parts.lock`, and the add writes one with this hash.
    Add(Hash),
    /// The add replaces `parts.lock` with one with this hash.
    Replace(Hash),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Record {
    /// The new copy is complete in `new/<id>`.
    Staged(String, Hash),
    /// The old `parts.lock` is copied into the staging folder.
    SavedLock(Hash),
    /// What was at `parts/<id>` is in `old/<id>`, or nothing was there.
    Aside(String, Option<Hash>),
    /// The new copy is at `parts/<id>`.
    Placed(String, Hash),
    LockWritten,
    Committed,
    /// After the commit, the copy `old/<id>` is deleted.
    Cleaned(String),
    /// Rollback moved the new copy from `parts/<id>` back into `new/<id>`.
    Unplaced(String),
    /// `old/<id>` is back at `parts/<id>`.
    Restored(String),
    /// `parts/<id>` changed after the add, and there was no other copy to choose, so it stays.
    Kept(String),
    LockRestored,
    /// `parts.lock` changed after the add, and there was no other copy to choose, so it stays.
    LockKept,
}

impl Record {
    pub(super) fn line(&self) -> String {
        match self {
            Record::Staged(id, hash) => format!("staged {id} {hash}"),
            Record::SavedLock(hash) => format!("saved-lock {hash}"),
            Record::Aside(id, Some(hash)) => format!("aside {id} {hash}"),
            Record::Aside(id, None) => format!("aside {id} -"),
            Record::Placed(id, hash) => format!("placed {id} {hash}"),
            Record::LockWritten => "lock-written".into(),
            Record::Committed => "committed".into(),
            Record::Cleaned(id) => format!("cleaned {id}"),
            Record::Unplaced(id) => format!("unplaced {id}"),
            Record::Restored(id) => format!("restored {id}"),
            Record::Kept(id) => format!("kept {id}"),
            Record::LockRestored => "lock-restored".into(),
            Record::LockKept => "lock-kept".into(),
        }
    }

    fn parse(line: &str) -> Option<Record> {
        let words: Vec<&str> = line.split(' ').collect();
        let id = |word: &str| single_component(word).then(|| word.to_string());
        Some(match words.as_slice() {
            ["staged", i, h] => Record::Staged(id(i)?, hash(h)?),
            ["saved-lock", h] => Record::SavedLock(hash(h)?),
            ["aside", i, "-"] => Record::Aside(id(i)?, None),
            ["aside", i, h] => Record::Aside(id(i)?, Some(hash(h)?)),
            ["placed", i, h] => Record::Placed(id(i)?, hash(h)?),
            ["lock-written"] => Record::LockWritten,
            ["committed"] => Record::Committed,
            ["cleaned", i] => Record::Cleaned(id(i)?),
            ["unplaced", i] => Record::Unplaced(id(i)?),
            ["restored", i] => Record::Restored(id(i)?),
            ["kept", i] => Record::Kept(id(i)?),
            ["lock-restored"] => Record::LockRestored,
            ["lock-kept"] => Record::LockKept,
            _ => return None,
        })
    }
}

fn hash(word: &str) -> Option<Hash> {
    let hex = word.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| word.to_string())
}

impl Plan {
    /// The first two lines of the log.
    pub(super) fn text(&self) -> String {
        let lock = match &self.lock {
            LockChange::Kept => "-".to_string(),
            LockChange::Add(hash) => format!("add:{hash}"),
            LockChange::Replace(hash) => format!("replace:{hash}"),
        };
        let parts = if self.parts_created {
            "created"
        } else {
            "existed"
        };
        format!("{HEADER}\nplan {parts} {lock} {}\n", self.ids.join(" "))
    }

    fn parse(line: &str) -> Option<Plan> {
        let mut words = line.split(' ');
        if words.next()? != "plan" {
            return None;
        }
        let parts_created = match words.next()? {
            "created" => true,
            "existed" => false,
            _ => return None,
        };
        let lock = match words.next()? {
            "-" => LockChange::Kept,
            word => match word.split_once(':')? {
                ("add", h) => LockChange::Add(hash(h)?),
                ("replace", h) => LockChange::Replace(hash(h)?),
                _ => return None,
            },
        };
        let ids = words
            .map(|word| single_component(word).then(|| word.to_string()))
            .collect::<Option<Vec<_>>>()?;
        Some(Plan {
            parts_created,
            lock,
            ids,
        })
    }
}

#[derive(Debug)]
pub(super) struct Log {
    pub(super) plan: Plan,
    pub(super) records: Vec<Record>,
}

pub(super) enum Read {
    Log(Log),
    /// The run stopped while it wrote the plan, before any other step.
    Torn,
    /// A line this orbe does not write.
    Unknown(String),
}

/// Only lines that end in a newline count. A run killed inside a write leaves at most a part of
/// its last line, and that step stays unproven.
pub(super) fn read(text: &str) -> Read {
    let lines: Vec<&str> = text
        .split_inclusive('\n')
        .filter_map(|line| line.strip_suffix('\n'))
        .collect();
    let start = format!("{HEADER}\nplan ");
    if lines.len() < 2 && (format!("{HEADER}\n").starts_with(text) || text.starts_with(&start)) {
        return Read::Torn;
    }
    if lines.first() != Some(&HEADER) {
        return Read::Unknown(lines.first().unwrap_or(&"").to_string());
    }
    let Some(plan) = lines.get(1).and_then(|line| Plan::parse(line)) else {
        return Read::Unknown(lines.get(1).unwrap_or(&"").to_string());
    };
    let mut records = Vec::new();
    for line in &lines[2..] {
        match Record::parse(line) {
            Some(record) => records.push(record),
            None => return Read::Unknown((*line).to_string()),
        }
    }
    Read::Log(Log { plan, records })
}

/// What the log proves about one id.
#[derive(Default)]
struct PartLog {
    staged: Option<Hash>,
    /// `None` until the step is logged. `Some(None)` when nothing was at `parts/<id>`.
    aside: Option<Option<Hash>>,
    placed: Option<Hash>,
    unplaced: bool,
    restored: bool,
    kept: bool,
    cleaned: bool,
}

impl PartLog {
    fn new_hash(&self) -> Option<&Hash> {
        self.placed.as_ref().or(self.staged.as_ref())
    }
}

#[derive(Default)]
struct LockLog {
    saved: Option<Hash>,
    written: bool,
    restored: bool,
    kept: bool,
}

impl Log {
    pub(super) fn committed(&self) -> bool {
        self.records.contains(&Record::Committed)
    }

    fn part(&self, id: &str) -> PartLog {
        let mut part = PartLog::default();
        for record in &self.records {
            match record {
                Record::Staged(i, h) if i == id => part.staged = Some(h.clone()),
                Record::Aside(i, h) if i == id => part.aside = Some(h.clone()),
                Record::Placed(i, h) if i == id => part.placed = Some(h.clone()),
                Record::Unplaced(i) if i == id => part.unplaced = true,
                Record::Restored(i) if i == id => part.restored = true,
                Record::Kept(i) if i == id => part.kept = true,
                Record::Cleaned(i) if i == id => part.cleaned = true,
                _ => {}
            }
        }
        part
    }

    fn lock(&self) -> LockLog {
        let mut lock = LockLog::default();
        for record in &self.records {
            match record {
                Record::SavedLock(h) => lock.saved = Some(h.clone()),
                Record::LockWritten => lock.written = true,
                Record::LockRestored => lock.restored = true,
                Record::LockKept => lock.kept = true,
                _ => {}
            }
        }
        lock
    }
}

/// What is on disk now, for the ids the plan names.
#[derive(Clone, Debug, Default)]
pub(super) struct Seen {
    pub(super) parts: BTreeMap<String, Here>,
    pub(super) lock: Option<Hash>,
    pub(super) saved_lock: Option<Hash>,
    /// Every name in the staging folder: `new/<name>`, `old/<name>`, or a name at its top.
    pub(super) names: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Here {
    /// The hash of `parts/<id>`, if anything is there.
    pub(super) dest: Option<Hash>,
    pub(super) new: bool,
    pub(super) old: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Effect {
    /// Only the record. The step already happened, or it changes nothing on disk.
    None,
    /// `parts/<id>` to `new/<id>`.
    Unplace(String),
    /// `old/<id>` to `parts/<id>`.
    Restore(String),
    /// Delete `old/<id>`.
    Delete(String),
    /// The saved lock back over `parts.lock`.
    LockBack,
    /// Delete the `parts.lock` the add wrote where there was none.
    LockRemove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Next {
    Act(Effect, Record),
    /// Every step is done. Delete what the log made, then the log.
    Clean,
    Stop(Vec<Problem>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    /// `parts/<id>` changed after the add, and `old/<id>` still holds the copy from before it.
    Conflict(String),
    /// The same, for `parts.lock` and its saved copy.
    LockConflict,
    /// Names in the staging folder that no step of the log made.
    Foreign(Vec<String>),
}

enum Decision {
    Act(Effect, Record),
    Conflict,
}

/// The next step of recovery, from the log and what is on disk. Before the commit it rolls the
/// add back, and after it it deletes the copies the add replaced. An id in conflict is never
/// moved. Recovery settles every other id first, then stops and names the conflicts.
pub(super) fn next(log: &Log, seen: &Seen) -> Next {
    let committed = log.committed();
    let mut problems = Vec::new();
    let mut first = None;
    for id in log.plan.ids.iter().rev() {
        let part = log.part(id);
        let here = seen.parts.get(id).cloned().unwrap_or_default();
        let decision = if committed {
            finish_part(id, &part, &here)
        } else {
            undo_part(id, &part, &here)
        };
        match decision {
            Some(Decision::Act(effect, record)) => {
                first.get_or_insert((effect, record));
            }
            Some(Decision::Conflict) => problems.push(Problem::Conflict(id.clone())),
            None => {}
        }
    }
    if !committed {
        match undo_lock(log, seen) {
            Some(Decision::Act(effect, record)) => {
                first.get_or_insert((effect, record));
            }
            Some(Decision::Conflict) => problems.push(Problem::LockConflict),
            None => {}
        }
    }
    if let Some((effect, record)) = first {
        return Next::Act(effect, record);
    }
    if !problems.is_empty() {
        return Next::Stop(problems);
    }
    let foreign = foreign(log, seen);
    if !foreign.is_empty() {
        return Next::Stop(vec![Problem::Foreign(foreign)]);
    }
    Next::Clean
}

fn undo_part(id: &str, part: &PartLog, here: &Here) -> Option<Decision> {
    if part.restored || part.kept {
        return None;
    }
    let act = |effect, record| Some(Decision::Act(effect, record));
    let new_hash = part.new_hash();
    match &here.dest {
        Some(dest) if part.aside.is_some() && !here.new && Some(dest) == new_hash => {
            act(Effect::Unplace(id.into()), Record::Unplaced(id.into()))
        }
        None if here.old => act(Effect::Restore(id.into()), Record::Restored(id.into())),
        None => None,
        Some(_) if here.old => Some(Decision::Conflict),
        Some(dest) => match &part.aside {
            Some(Some(before)) if dest == before => act(Effect::None, Record::Restored(id.into())),
            Some(Some(_)) => act(Effect::None, Record::Kept(id.into())),
            Some(None) if !here.new && new_hash.is_some() => {
                act(Effect::None, Record::Kept(id.into()))
            }
            _ => None,
        },
    }
}

fn finish_part(id: &str, part: &PartLog, here: &Here) -> Option<Decision> {
    let Some(Some(before)) = &part.aside else {
        return None;
    };
    if part.cleaned || part.restored {
        return None;
    }
    let act = |effect, record| Some(Decision::Act(effect, record));
    match (&here.dest, here.old) {
        (Some(dest), true) if Some(dest) == part.new_hash() => {
            act(Effect::Delete(id.into()), Record::Cleaned(id.into()))
        }
        (None, true) => act(Effect::Restore(id.into()), Record::Restored(id.into())),
        (Some(_), true) => Some(Decision::Conflict),
        (Some(dest), false) if dest == before => act(Effect::None, Record::Restored(id.into())),
        (_, false) => act(Effect::None, Record::Cleaned(id.into())),
    }
}

fn undo_lock(log: &Log, seen: &Seen) -> Option<Decision> {
    let (existed, new) = match &log.plan.lock {
        LockChange::Kept => return None,
        LockChange::Add(hash) => (false, hash),
        LockChange::Replace(hash) => (true, hash),
    };
    let lock = log.lock();
    if lock.restored || lock.kept {
        return None;
    }
    let act = |effect, record| Some(Decision::Act(effect, record));
    let saved = lock.saved.is_some() && seen.saved_lock == lock.saved;
    match &seen.lock {
        Some(now) if now == new => match (existed, saved) {
            (false, _) => act(Effect::LockRemove, Record::LockRestored),
            (true, true) => act(Effect::LockBack, Record::LockRestored),
            (true, false) => act(Effect::None, Record::LockKept),
        },
        _ if !lock.written => None,
        None if existed && saved => act(Effect::LockBack, Record::LockRestored),
        None => act(Effect::None, Record::LockRestored),
        Some(now) if existed && Some(now) == lock.saved.as_ref() => {
            act(Effect::None, Record::LockRestored)
        }
        Some(_) if existed && seen.saved_lock.is_some() => Some(Decision::Conflict),
        Some(_) => act(Effect::None, Record::LockKept),
    }
}

/// Names in the staging folder that the log did not make, which recovery never deletes.
fn foreign(log: &Log, seen: &Seen) -> Vec<String> {
    let planned = |id: &str| log.plan.ids.iter().any(|planned| planned == id);
    seen.names
        .iter()
        .filter(|name| {
            let ours = match name.split_once('/') {
                Some(("new", id)) => !log.committed() && planned(id),
                Some(("old", id)) => planned(id),
                Some(_) => false,
                None => match name.as_str() {
                    "log" | "new" | "old" => true,
                    "parts.lock" => matches!(log.plan.lock, LockChange::Replace(_)),
                    _ => false,
                },
            };
            !ours
        })
        .cloned()
        .collect()
}

/// What the log says recovery did, one phrase each, in the order it did it.
fn undone(log: &Log, records: &[Record]) -> Vec<String> {
    let mut seen_ids = Vec::new();
    let mut lines = Vec::new();
    for record in records {
        let line = match record {
            Record::Unplaced(id) | Record::Restored(id) | Record::Kept(id) => {
                if seen_ids.contains(id) {
                    continue;
                }
                seen_ids.push(id.clone());
                let part = log.part(id);
                if part.restored {
                    format!("put back parts/{id}")
                } else if part.unplaced {
                    format!("removed the new parts/{id}")
                } else {
                    format!("left parts/{id} as it is, since it changed after the add")
                }
            }
            Record::LockRestored => match log.plan.lock {
                LockChange::Add(_) => "removed the new parts.lock".into(),
                _ => "put back parts.lock".into(),
            },
            Record::LockKept => "left parts.lock as it is, since it changed after the add".into(),
            _ => continue,
        };
        lines.push(line);
    }
    lines
}

/// The ids the add placed, then found gone when it rolled back.
fn gone(log: &Log) -> Vec<String> {
    log.plan
        .ids
        .iter()
        .filter(|id| {
            let part = log.part(id);
            part.placed.is_some() && !part.unplaced && !part.restored && !part.kept
        })
        .map(|id| format!("parts/{id}"))
        .collect()
}

/// What recovery did, from the whole log, once it is done. `None` when the add never changed
/// the project.
pub(super) fn summary(log: &Log) -> Option<String> {
    if log.committed() {
        return Some(finished(log));
    }
    let mut lines = undone(log, &log.records);
    let gone = gone(log);
    if !gone.is_empty() {
        lines.push(format!(
            "{}, which it had placed, {} already gone",
            gone.join(", "),
            if gone.len() == 1 { "was" } else { "were" }
        ));
    }
    (!lines.is_empty()).then(|| lines.join(", "))
}

/// The steps this run's recovery took, for a message about a recovery that stopped.
pub(super) fn steps_taken(log: &Log, from: usize) -> Vec<String> {
    if log.committed() {
        let cleaned: Vec<String> = log.records[from.min(log.records.len())..]
            .iter()
            .filter_map(|record| match record {
                Record::Cleaned(id) => Some(format!("deleted parts/{STAGE}/old/{id}")),
                Record::Restored(id) => Some(format!("put back parts/{id}")),
                _ => None,
            })
            .collect();
        return cleaned;
    }
    undone(log, &log.records[from.min(log.records.len())..])
}

fn finished(log: &Log) -> String {
    let mut wrote: Vec<String> = log
        .plan
        .ids
        .iter()
        .filter(|id| log.part(id).placed.is_some())
        .map(|id| format!("parts/{id}"))
        .collect();
    if log.records.contains(&Record::LockWritten) {
        wrote.push("parts.lock".into());
    }
    let mut text = format!(
        "finished an interrupted add, which had written {}",
        wrote.join(", ")
    );
    let cleaned: Vec<String> = log
        .plan
        .ids
        .iter()
        .filter(|id| log.part(id).cleaned)
        .map(|id| format!("parts/{id}"))
        .collect();
    if !cleaned.is_empty() {
        text.push_str(&format!(
            ", and deleted the copies it replaced: {}",
            cleaned.join(", ")
        ));
    }
    for id in &log.plan.ids {
        if log.part(id).restored {
            text.push_str(&format!(
                ". parts/{id} was gone, so orbe put back the copy the add had replaced"
            ));
        }
    }
    text
}

/// One line per problem, saying what is on disk, then a hint. `again` is the command to rerun.
pub(super) fn problem_text(log: &Log, problems: &[Problem], again: &str) -> (String, String) {
    let committed = log.committed();
    let mut lines = Vec::new();
    let mut hints = Vec::new();
    for problem in problems {
        match problem {
            Problem::Conflict(id) if committed => {
                lines.push(format!(
                    "parts/{id} changed after the add replaced it, and the copy it replaced is in parts/{STAGE}/old/{id}. orbe left both"
                ));
                hints.push(keep_one(&format!("parts/{id}"), &format!("old/{id}")));
            }
            Problem::Conflict(id) => {
                lines.push(format!(
                    "parts/{id} changed after the interrupted add placed its new copy there, and the copy from before the add is in parts/{STAGE}/old/{id}. orbe left both"
                ));
                hints.push(keep_one(&format!("parts/{id}"), &format!("old/{id}")));
            }
            Problem::LockConflict => {
                lines.push(format!(
                    "parts.lock changed after the interrupted add wrote it, and the lock from before the add is in parts/{STAGE}/parts.lock. orbe left both"
                ));
                hints.push(keep_one("parts.lock", "parts.lock"));
            }
            Problem::Foreign(names) => {
                let names = names
                    .iter()
                    .map(|name| format!("parts/{STAGE}/{name}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!(
                    "{names} {} not made by an add, so orbe left parts/{STAGE} and its log",
                    if names.contains(", ") { "were" } else { "was" }
                ));
                hints.push(format!("move {names} out of parts/{STAGE}"));
            }
        }
    }
    hints.push(format!("then run `{again}` again"));
    (lines.join("\n"), hints.join(". "))
}

fn keep_one(path: &str, staged: &str) -> String {
    format!(
        "keep one: delete {path} to get back parts/{STAGE}/{staged}, or delete parts/{STAGE}/{staged} to keep {path} as it is"
    )
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
