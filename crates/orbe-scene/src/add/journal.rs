//! The log of one `orbe add`, and what recovery does next.
//!
//! The log is append-only. Each line records a step, with the hash of what it moves or writes, so
//! recovery can tell a path the add made from one it did not. Recovery acts only on names the
//! log made, and only while they hold the hash it recorded. [`next`] decides each step from the
//! log and the hashes on disk, and recovery logs its own steps the same way, so a recovery that
//! stops part way is resumed.

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
    /// What is at `parts/<id>`, logged before the add moves it to `old/<id>`, or nothing is there.
    Aside(String, Option<Hash>),
    /// The new copy is at `parts/<id>`.
    Placed(String, Hash),
    LockWritten,
    Committed,
    /// Rollback moved the new copy from `parts/<id>` back into `new/<id>`.
    Unplaced(String),
    /// `old/<id>` is back at `parts/<id>`.
    Restored(String),
    /// `parts/<id>` changed after the add, and there was no other copy to choose, so it stays.
    Kept(String),
    LockRestored,
    /// `parts.lock` changed after the add, and there was no other copy to choose, so it stays.
    LockKept,
    /// `parts.lock` is the one the add wrote, and the copy from before it is gone, so it stays.
    LockStays,
    /// Every path below the staging folder but the log, with what it held, logged before the
    /// first of them is deleted. Recovery deletes only these, and only while they hold it.
    Clearing(Vec<Entry>),
}

/// A path below the staging folder, such as `old/led-5mm/part.jsonc`, and what is there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) path: String,
    pub(super) kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Folder,
    File(Hash),
    Link(String),
    Other,
}

impl Entry {
    fn word(&self) -> String {
        let path = escape(&self.path);
        match &self.kind {
            Kind::Folder => format!("d:{path}"),
            Kind::File(hash) => format!("f:{path}:{hash}"),
            Kind::Link(target) => format!("l:{path}:{}", escape(target)),
            Kind::Other => format!("o:{path}"),
        }
    }

    fn parse(word: &str) -> Option<Entry> {
        let mut fields = word.splitn(3, ':');
        let (tag, path, rest) = (fields.next()?, fields.next()?, fields.next());
        let path = unescape(path)?;
        let below = path.split('/').all(|name| !matches!(name, "" | "." | ".."));
        if !below || path == "log" {
            return None;
        }
        let kind = match (tag, rest) {
            ("d", None) => Kind::Folder,
            ("f", Some(h)) => Kind::File(hash(h)?),
            ("l", Some(target)) => Kind::Link(unescape(target)?),
            ("o", None) => Kind::Other,
            _ => return None,
        };
        Some(Entry { path, kind })
    }
}

/// A name in a log word, with `%`, `:`, spaces and every byte outside printable ASCII as `%XX`.
fn escape(name: &str) -> String {
    let mut out = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_graphic() && byte != b'%' && byte != b':' {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn unescape(word: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut rest = word.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        if byte == b'%' {
            let digits = std::str::from_utf8(after.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(digits, 16).ok()?);
            rest = &after[2..];
        } else {
            bytes.push(byte);
            rest = after;
        }
    }
    String::from_utf8(bytes).ok()
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
            Record::Unplaced(id) => format!("unplaced {id}"),
            Record::Restored(id) => format!("restored {id}"),
            Record::Kept(id) => format!("kept {id}"),
            Record::LockRestored => "lock-restored".into(),
            Record::LockKept => "lock-kept".into(),
            Record::LockStays => "lock-stays".into(),
            Record::Clearing(entries) => {
                entries.iter().fold("clearing".to_string(), |line, entry| {
                    format!("{line} {}", entry.word())
                })
            }
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
            ["unplaced", i] => Record::Unplaced(id(i)?),
            ["restored", i] => Record::Restored(id(i)?),
            ["kept", i] => Record::Kept(id(i)?),
            ["lock-restored"] => Record::LockRestored,
            ["lock-kept"] => Record::LockKept,
            ["lock-stays"] => Record::LockStays,
            ["clearing", entries @ ..] => Record::Clearing(
                entries
                    .iter()
                    .map(|word| Entry::parse(word))
                    .collect::<Option<_>>()?,
            ),
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
    /// A header or plan this orbe does not write.
    Unknown(String),
    /// A step this orbe does not write, after a plan it reads.
    UnknownStep(String),
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
            None => return Read::UnknownStep((*line).to_string()),
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

/// A write the add logs only once it is whole, so a run killed inside it leaves part of it and
/// no line.
#[derive(PartialEq, Eq)]
enum Unlogged {
    New(String),
    SavedLock,
}

impl Log {
    pub(super) fn committed(&self) -> bool {
        self.records.contains(&Record::Committed)
    }

    /// What the clearing record lists, once recovery has begun to delete the staging folder.
    pub(super) fn clearing(&self) -> Option<&[Entry]> {
        self.records.iter().find_map(|record| match record {
            Record::Clearing(entries) => Some(entries.as_slice()),
            _ => None,
        })
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
                Record::LockKept | Record::LockStays => lock.kept = true,
                _ => {}
            }
        }
        lock
    }

    /// The write that follows the last line, when the log stops inside staging: the next
    /// `new/<id>`, or the saved lock after the last one.
    fn unlogged(&self) -> Option<Unlogged> {
        let staged = self
            .records
            .iter()
            .map(|record| match record {
                Record::Staged(id, _) => Some(id),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        if staged.len() > self.plan.ids.len()
            || staged.iter().zip(&self.plan.ids).any(|(a, b)| *a != b)
        {
            return None;
        }
        match self.plan.ids.get(staged.len()) {
            Some(id) => Some(Unlogged::New(id.clone())),
            None if matches!(self.plan.lock, LockChange::Replace(_)) => Some(Unlogged::SavedLock),
            None => None,
        }
    }
}

/// What is on disk now, for the ids the plan names. Every hash is of what sits at the path, a
/// link included, and no link is followed to get it.
#[derive(Clone, Debug, Default)]
pub(super) struct Seen {
    pub(super) parts: BTreeMap<String, Here>,
    pub(super) lock: Option<Hash>,
    pub(super) saved_lock: Option<Hash>,
    /// Every name in the staging folder: `new/<name>`, `old/<name>`, or a name at its top.
    pub(super) names: Vec<String>,
    /// `new` or `old` when it is there and is not a folder. Nothing inside it is read.
    pub(super) odd: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Here {
    pub(super) dest: Option<Hash>,
    pub(super) new: Option<Hash>,
    pub(super) old: Option<Hash>,
}

/// A change on disk, with the hash each path it moves or replaces must have right before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Effect {
    /// Only the record. The step already happened, or it changes nothing on disk.
    None,
    /// `parts/<id>`, while it has this hash, to `new/<id>`.
    Unplace(String, Hash),
    /// `old/<id>`, while it has this hash, to `parts/<id>`.
    Restore(String, Hash),
    /// The saved lock, while it has the first hash, over `parts.lock`, while that has the second
    /// hash or is absent.
    LockBack(Hash, Option<Hash>),
    /// Delete `parts.lock` while it has this hash, the one the add wrote where there was none.
    LockRemove(Hash),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Next {
    Act(Effect, Record),
    /// Every step is done, and everything left in the staging folder is what the log made.
    /// Delete the folder.
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
    /// Names the log made that no longer have the hash it recorded.
    Changed(Vec<String>),
    /// Before the commit, `parts/<id>` holds the add's new copy, and the copy it replaced is not
    /// in `old/<id>` to put back.
    Missing(String),
}

enum Decision {
    Act(Effect, Record),
    Conflict,
    Missing,
}

/// The next step of recovery, from the log and what is on disk. Before the commit it rolls the
/// add back, and after it it keeps the new copies. Recovery touches only names the log made,
/// and only while they have the hash it recorded, so a stray or changed name stops it before
/// it moves anything. An id in conflict is never moved. Recovery settles every other id first,
/// then stops and names the conflicts.
pub(super) fn next(log: &Log, seen: &Seen) -> Next {
    let (first, problems) = steps(log, seen);
    if let Some((effect, record)) = first {
        return Next::Act(effect, record);
    }
    if !problems.is_empty() {
        return Next::Stop(problems);
    }
    Next::Clean
}

/// Whether recovery from here stops on a problem before it is done, as [`next`] decides it.
pub(super) fn stops(log: &Log, seen: &Seen) -> bool {
    !steps(log, seen).1.is_empty()
}

/// The first step recovery takes, and the problems it stops on once every other step is done.
fn steps(log: &Log, seen: &Seen) -> (Option<(Effect, Record)>, Vec<Problem>) {
    let strays = strays(log, seen);
    if !strays.is_empty() {
        return (None, strays);
    }
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
            Some(Decision::Missing) => problems.push(Problem::Missing(id.clone())),
            None => {}
        }
    }
    if !committed {
        match undo_lock(log, seen) {
            Some(Decision::Act(effect, record)) => {
                first.get_or_insert((effect, record));
            }
            Some(_) => problems.push(Problem::LockConflict),
            None => {}
        }
    }
    (first, problems)
}

fn undo_part(id: &str, part: &PartLog, here: &Here) -> Option<Decision> {
    if part.restored || part.kept {
        return None;
    }
    let aside = part.aside.as_ref()?;
    let new = part.new_hash();
    let act = |effect, record| Some(Decision::Act(effect, record));
    let Some(dest) = &here.dest else {
        return match (aside, &here.old) {
            _ if part.placed.is_some() && !part.unplaced && here.new.as_ref() == new => {
                act(Effect::None, Record::Unplaced(id.into()))
            }
            (Some(before), Some(_)) => act(
                Effect::Restore(id.into(), before.clone()),
                Record::Restored(id.into()),
            ),
            _ => None,
        };
    };
    if Some(dest) == new && here.new.is_none() {
        return match (aside, &here.old) {
            (Some(_), None) => Some(Decision::Missing),
            _ => act(
                Effect::Unplace(id.into(), dest.clone()),
                Record::Unplaced(id.into()),
            ),
        };
    }
    match (aside, &here.old) {
        (_, Some(_)) => Some(Decision::Conflict),
        (Some(before), None) if dest == before => None,
        (Some(_), None) => act(Effect::None, Record::Kept(id.into())),
        (None, None) if here.new.is_none() => act(Effect::None, Record::Kept(id.into())),
        (None, None) => None,
    }
}

/// After the commit, the copy in `old/<id>` goes with the staging folder only while the new
/// copy is still at `parts/<id>`.
fn finish_part(id: &str, part: &PartLog, here: &Here) -> Option<Decision> {
    let Some(Some(before)) = &part.aside else {
        return None;
    };
    if part.restored {
        return None;
    }
    let act = |effect, record| Some(Decision::Act(effect, record));
    match (&here.dest, &here.old) {
        (Some(dest), Some(_)) if Some(dest) == part.new_hash() => None,
        (None, Some(_)) => act(
            Effect::Restore(id.into(), before.clone()),
            Record::Restored(id.into()),
        ),
        (Some(_), Some(_)) => Some(Decision::Conflict),
        (Some(dest), None) if dest == before => act(Effect::None, Record::Restored(id.into())),
        (_, None) => None,
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
    let saved = lock
        .saved
        .as_ref()
        .filter(|saved| seen.saved_lock.as_ref() == Some(saved));
    match (&seen.lock, saved) {
        (Some(now), _) if now == new && !existed => {
            act(Effect::LockRemove(new.clone()), Record::LockRestored)
        }
        (Some(now), Some(saved)) if now == new => act(
            Effect::LockBack(saved.clone(), Some(new.clone())),
            Record::LockRestored,
        ),
        (Some(now), None) if now == new => act(Effect::None, Record::LockStays),
        _ if !lock.written => None,
        (None, Some(saved)) => act(Effect::LockBack(saved.clone(), None), Record::LockRestored),
        (None, None) if existed => act(Effect::None, Record::LockKept),
        (None, None) => None,
        (Some(now), _) if Some(now) == lock.saved.as_ref() => {
            act(Effect::None, Record::LockRestored)
        }
        (Some(_), Some(_)) => Some(Decision::Conflict),
        (Some(_), None) => act(Effect::None, Record::LockKept),
    }
}

/// Names in the staging folder that no step of the log made, then names it made that no
/// longer hold what it recorded.
fn strays(log: &Log, seen: &Seen) -> Vec<Problem> {
    let mut foreign = Vec::new();
    let mut changed = Vec::new();
    for name in &seen.names {
        match made(log, seen, name) {
            Made::Yes => {}
            Made::No => foreign.push(name.clone()),
            Made::Changed => changed.push(name.clone()),
        }
    }
    let mut problems = Vec::new();
    if !foreign.is_empty() {
        problems.push(Problem::Foreign(foreign));
    }
    if !changed.is_empty() {
        problems.push(Problem::Changed(changed));
    }
    problems
}

enum Made {
    Yes,
    No,
    Changed,
}

fn made(log: &Log, seen: &Seen, name: &str) -> Made {
    let planned = |id: &str| log.plan.ids.iter().any(|planned| planned == id);
    let here = |id: &str| seen.parts.get(id).cloned().unwrap_or_default();
    let holds = |now: Option<Hash>, recorded: &Hash| {
        if now.as_ref() == Some(recorded) {
            Made::Yes
        } else {
            Made::Changed
        }
    };
    match name.split_once('/') {
        Some(("new", id)) if planned(id) && !log.committed() => match log.part(id).staged {
            Some(staged) => holds(here(id).new, &staged),
            None if log.unlogged() == Some(Unlogged::New(id.into())) => Made::Yes,
            None => Made::No,
        },
        Some(("old", id)) if planned(id) => {
            let part = log.part(id);
            match &part.aside {
                Some(Some(before)) if !part.restored && !part.kept => holds(here(id).old, before),
                _ => Made::No,
            }
        }
        Some(_) => Made::No,
        None => match name {
            "log" => Made::Yes,
            "new" | "old" if !seen.odd.iter().any(|odd| odd == name) => Made::Yes,
            "parts.lock" if matches!(log.plan.lock, LockChange::Replace(_)) => {
                match &log.lock().saved {
                    Some(saved) => holds(seen.saved_lock.clone(), saved),
                    None if log.unlogged() == Some(Unlogged::SavedLock) => Made::Yes,
                    None => Made::No,
                }
            }
            _ => Made::No,
        },
    }
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
            Record::LockStays => format!(
                "left the parts.lock the add wrote, since the copy from before it was not in parts/{STAGE}"
            ),
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

/// What recovery did, from the whole log, once it is done. `replaced` are the ids whose old
/// copies go with the staging folder after the commit. `None` when the project is as it was
/// before the add.
pub(super) fn summary(log: &Log, replaced: &[String]) -> Option<String> {
    if log.committed() {
        return Some(finished(log, replaced));
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
    let records = &log.records[from.min(log.records.len())..];
    if log.committed() {
        return records
            .iter()
            .filter_map(|record| match record {
                Record::Restored(id) => Some(format!("put back parts/{id}")),
                _ => None,
            })
            .collect();
    }
    undone(log, records)
}

fn finished(log: &Log, replaced: &[String]) -> String {
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
    if !replaced.is_empty() {
        let replaced: Vec<String> = replaced.iter().map(|id| format!("parts/{id}")).collect();
        text.push_str(&format!(
            ", and deleted the old {} of {} it replaced",
            if replaced.len() == 1 {
                "copy"
            } else {
                "copies"
            },
            replaced.join(", ")
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
                let names = staged_paths(names);
                lines.push(format!(
                    "{names} {} not made by an add, so orbe left parts/{STAGE} and its log",
                    if names.contains(", ") { "were" } else { "was" }
                ));
                hints.push(format!("move {names} out of parts/{STAGE}"));
            }
            Problem::Changed(names) => {
                for name in names {
                    lines.push(changed_line(name));
                }
                hints.push(format!(
                    "move {} out of parts/{STAGE} to keep {}",
                    staged_paths(names),
                    if names.len() == 1 { "it" } else { "them" }
                ));
            }
            Problem::Missing(id) => {
                lines.push(format!(
                    "parts/{id} holds the new copy the interrupted add placed, and the copy it replaced is not in parts/{STAGE}/old/{id}, so orbe cannot put that back. orbe left parts/{id}"
                ));
                hints.push(format!(
                    "delete parts/{id}, whose files are the catalog's, to finish the rollback"
                ));
            }
        }
    }
    hints.push(format!("then run `{again}` again"));
    (lines.join("\n"), hints.join(". "))
}

fn staged_paths(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("parts/{STAGE}/{name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn changed_line(name: &str) -> String {
    let path = format!("parts/{STAGE}/{name}");
    match name.split_once('/') {
        Some(("old", id)) => format!(
            "{path} changed after the add moved the copy from parts/{id} there. orbe left it, and parts/{id} as it is"
        ),
        Some((_, id)) => {
            format!("{path} changed after the add wrote its new copy of {id} there. orbe left it")
        }
        None => format!(
            "{path} changed after the add saved the old parts.lock there. orbe left it, and parts.lock as it is"
        ),
    }
}

fn keep_one(path: &str, staged: &str) -> String {
    format!(
        "keep one: delete {path} to get back parts/{STAGE}/{staged}, or delete parts/{STAGE}/{staged} to keep {path} as it is"
    )
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
