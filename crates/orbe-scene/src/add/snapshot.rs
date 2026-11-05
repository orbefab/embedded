//! What the plan sees of a project: `parts.lock`, and what sits at `parts/<id>` for each
//! catalog id the request can reach. Nothing else under `parts/` is read.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;

use crate::catalog::Version;
use crate::diagnostic::Findings;
use crate::project::{self, LockFields, hash_files};
use crate::source::Sources;
use crate::syntax::{self, Node};

use super::{RunError, io_at};

/// What sits at `parts/<id>`. A link is never followed, so replacing it never touches its
/// target.
pub(super) enum Target {
    Absent,
    Folder(Folder),
    /// A file, a link, or anything else that is not a folder, named for a message.
    Other(&'static str),
}

/// A folder's readable files, and the entries that are not plain readable files.
pub(super) struct Folder {
    pub(super) version: Option<Version>,
    pub(super) hash: String,
    pub(super) files: BTreeMap<String, Vec<u8>>,
    /// Entries that are links, unreadable, or neither file nor folder, as message phrases.
    pub(super) odd: Vec<String>,
}

impl Folder {
    pub(super) fn from_files(files: BTreeMap<String, Vec<u8>>) -> Folder {
        let version = files.get("part.jsonc").and_then(|bytes| version_of(bytes));
        let hash = hash_files(
            files
                .iter()
                .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        );
        Folder {
            version,
            hash,
            files,
            odd: Vec::new(),
        }
    }

    /// Whether the folder holds exactly these bytes.
    pub(super) fn is(&self, hash: &str) -> bool {
        self.odd.is_empty() && self.hash == hash
    }

    pub(super) fn has_part_file(&self) -> bool {
        self.files.contains_key("part.jsonc")
    }
}

pub(super) struct Snapshot {
    pub(super) targets: BTreeMap<String, Target>,
    pub(super) lock: BTreeMap<String, LockFields>,
    pub(super) lock_text: Option<String>,
    pub(super) lock_error: Option<String>,
}

impl Snapshot {
    pub(super) fn target(&self, id: &str) -> &Target {
        self.targets.get(id).unwrap_or(&Target::Absent)
    }

    pub(super) fn folder(&self, id: &str) -> Option<&Folder> {
        match self.target(id) {
            Target::Folder(folder) => Some(folder),
            _ => None,
        }
    }
}

/// Reads `parts.lock` and `parts/<id>` for each id. A folder that cannot be read whole is
/// still a target, one that differs.
pub(super) fn load(root: &Path, ids: &BTreeSet<String>) -> Result<Snapshot, RunError> {
    let parts = root.join("parts");
    if parts.exists() && !parts.is_dir() {
        return Err(io_at(&parts, io::Error::other("`parts` is not a folder")));
    }
    let mut targets = BTreeMap::new();
    for id in ids {
        targets.insert(id.clone(), target(&parts.join(id)));
    }
    let (lock_text, lock, lock_error) = read_lock(root)?;
    Ok(Snapshot {
        targets,
        lock,
        lock_text,
        lock_error,
    })
}

fn target(path: &Path) -> Target {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Target::Absent;
    };
    if meta.file_type().is_symlink() {
        return Target::Other("a link");
    }
    if meta.is_file() {
        return Target::Other("a file");
    }
    if !meta.is_dir() {
        return Target::Other("neither a file nor a folder");
    }
    let mut files = BTreeMap::new();
    let mut odd = Vec::new();
    read_tree(path, path, &mut files, &mut odd);
    let mut folder = Folder::from_files(files);
    folder.odd = odd;
    Target::Folder(folder)
}

fn read_tree(
    root: &Path,
    dir: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
    odd: &mut Vec<String>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            odd.push(format!(
                "`{}`, which cannot be read: {error}",
                rel(root, dir)
            ));
            return;
        }
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                odd.push(format!(
                    "`{}`, which cannot be read: {error}",
                    rel(root, dir)
                ));
                continue;
            }
        };
        let shown = rel(root, &path);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                odd.push(format!("`{shown}`, which is a link"));
            }
            Ok(meta) if meta.is_dir() => read_tree(root, &path, files, odd),
            Ok(meta) if meta.is_file() => match fs::read(&path) {
                Ok(bytes) => {
                    files.insert(shown, bytes);
                }
                Err(error) => odd.push(format!("`{shown}`, which cannot be read: {error}")),
            },
            Ok(_) => odd.push(format!("`{shown}`, which is not a file")),
            Err(error) => odd.push(format!("`{shown}`, which cannot be read: {error}")),
        }
    }
}

fn rel(root: &Path, path: &Path) -> String {
    let shown = crate::source::display(root, path);
    if shown.is_empty() { ".".into() } else { shown }
}

type Lock = (Option<String>, BTreeMap<String, LockFields>, Option<String>);

fn read_lock(root: &Path) -> Result<Lock, RunError> {
    let path = root.join("parts.lock");
    if !path.exists() {
        return Ok((None, BTreeMap::new(), None));
    }
    if !path.is_file() {
        return Err(io_at(&path, io::Error::other("`parts.lock` is not a file")));
    }
    let text = fs::read_to_string(&path).map_err(|error| io_at(&path, error))?;
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
    Ok((Some(text), lines, error))
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
