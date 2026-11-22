//! What the plan sees of a project: what sits at `parts/<id>` for each catalog id the request
//! can reach. A folder that is already there is named from its own `part.jsonc` version.
//! Nothing else under `parts/` is read.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::catalog::Version;
use crate::diagnostic::Findings;
use crate::source::Sources;
use crate::syntax::{self, Node};

use super::{RunError, io_at};

/// What sits at `parts/<id>`. A link is never followed.
pub(super) enum Target {
    Absent,
    Folder(Folder),
    /// A file, a link, or anything else that is not a folder, named for a message.
    Other(&'static str),
}

/// A directory `add` tried to list, and the I/O error.
pub(super) struct Denied {
    pub(super) error: String,
    pub(super) permission: bool,
}

/// A folder that is already at `parts/<id>`. `version` is the copy's own, when `part.jsonc`
/// has one. No other file is read.
pub(super) struct Folder {
    pub(super) version: Option<Version>,
}

impl Folder {
    #[cfg(test)]
    pub(super) fn from_files(files: BTreeMap<String, Vec<u8>>) -> Folder {
        Folder {
            version: files.get("part.jsonc").and_then(|bytes| version_of(bytes)),
        }
    }
}

pub(super) struct Snapshot {
    pub(super) targets: BTreeMap<String, Target>,
    /// `parts/` exists and this process cannot list it. The ids under it were not read,
    /// so a missing folder is not the same thing as a part that is absent.
    pub(super) parts_denied: Option<Denied>,
    /// The project folder as the user named it, from the directory `orbe` ran in.
    pub(super) root: PathBuf,
}

impl Snapshot {
    pub(super) fn target(&self, id: &str) -> &Target {
        self.targets.get(id).unwrap_or(&Target::Absent)
    }
}

/// Reads `parts/<id>` for each id. A directory that cannot be listed is [`Denied`]. It is
/// not an absent part.
pub(super) fn load(root: &Path, ids: &BTreeSet<String>) -> Result<Snapshot, RunError> {
    let parts = root.join("parts");
    let parts_denied = match fs::metadata(&parts) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => Some(denied(error)),
        Ok(meta) if !meta.is_dir() => {
            return Err(io_at(&parts, io::Error::other("`parts` is not a folder")));
        }
        Ok(_) => match fs::read_dir(&parts) {
            Ok(_) => None,
            Err(error) => Some(denied(error)),
        },
    };
    let mut targets = BTreeMap::new();
    // A directory that cannot be listed hides every id. Walking them would record each
    // one as absent.
    if parts_denied.is_none() {
        for id in ids {
            targets.insert(id.clone(), target(&parts.join(id)));
        }
    }
    Ok(Snapshot {
        targets,
        parts_denied,
        root: root.to_path_buf(),
    })
}

fn denied(error: io::Error) -> Denied {
    Denied {
        permission: error.kind() == io::ErrorKind::PermissionDenied,
        error: error.to_string(),
    }
}

fn target(path: &Path) -> Target {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Target::Absent,
        // A failure other than "nothing is there" is not an absent part.
        Err(_) => return Target::Folder(Folder { version: None }),
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
    let version = fs::read(path.join("part.jsonc"))
        .ok()
        .and_then(|bytes| version_of(&bytes));
    Target::Folder(Folder { version })
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
