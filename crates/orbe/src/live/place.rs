//! Where a session's process listens: the session's name, and the private folder that holds
//! one socket per session.

use std::fmt;
use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 1 to 64 ASCII letters, digits, `.`, `_` and `-`, starting with a letter or digit, so it is
/// one file name, never reads as a flag, and keeps a socket path under the system's limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name(String);

impl Name {
    pub fn parse(text: &str) -> Result<Name, String> {
        let allowed = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
        let starts = text.starts_with(|c: char| c.is_ascii_alphanumeric());
        if !starts || text.len() > 64 || !text.chars().all(allowed) {
            return Err(format!(
                "`{text}` is not a session name; use 1 to 64 letters, digits, `.`, `_` and `-`, starting with a letter or digit"
            ));
        }
        Ok(Name(text.to_string()))
    }

    /// The git worktree root above `cwd`, else the project folder (the nearest one with
    /// scene.jsonc or parts.lock), else `cwd`: its folder name and a short hash of its path.
    pub fn default_for(cwd: &Path) -> Name {
        let above = |marker: fn(&Path) -> bool| cwd.ancestors().find(|dir| marker(dir));
        let folder = above(|dir| dir.join(".git").exists())
            .or_else(|| {
                above(|dir| dir.join("scene.jsonc").is_file() || dir.join("parts.lock").is_file())
            })
            .unwrap_or(cwd);
        let file = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem: String = file
            .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .take(48)
            .collect();
        let hash = Sha256::digest(folder.as_os_str().as_encoded_bytes());
        let short: String = hash[..4].iter().map(|b| format!("{b:02x}")).collect();
        let stem = if stem.is_empty() { "orbe" } else { &stem };
        Name(format!("{stem}-{short}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The folder of this build's sessions: `$XDG_RUNTIME_DIR/orbe/<version>`, else
/// `<temp>/orbe-<uid>/<version>`. Each level is the user's own, with mode 0700.
#[derive(Clone)]
pub struct Place {
    dir: PathBuf,
}

impl Place {
    /// Finds the folder, creating what is missing. A refusal is its message and hint.
    pub fn find() -> Result<Place, (String, String)> {
        let base = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
            Some(runtime) if runtime.is_absolute() => runtime.join("orbe"),
            _ => std::env::temp_dir().join(format!("orbe-{}", rustix::process::getuid().as_raw())),
        };
        let dir = base.join(env!("CARGO_PKG_VERSION"));
        private(&base)?;
        private(&dir)?;
        Ok(Place { dir })
    }

    pub fn socket(&self, name: &Name) -> PathBuf {
        self.dir.join(format!("{name}.sock"))
    }

    /// What the session's process wrote to stderr, such as a panic, kept when it crashes.
    pub fn log(&self, name: &Name) -> PathBuf {
        self.dir.join(format!("{name}.log"))
    }

    /// Every session with a socket, live or left by a crash, sorted.
    pub fn names(&self) -> io::Result<Vec<Name>> {
        let mut names: Vec<Name> = fs::read_dir(&self.dir)?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file = entry.file_name();
                let stem = file.to_str()?.strip_suffix(".sock")?;
                Name::parse(stem).ok()
            })
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(names)
    }
}

/// Creates `dir` with mode 0700, or checks that the one there is a folder the user owns that
/// only the user can open.
fn private(dir: &Path) -> Result<(), (String, String)> {
    let shown = dir.display();
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err((
                format!("cannot create the session folder {shown}: {error}"),
                "set XDG_RUNTIME_DIR to a folder you own".into(),
            ));
        }
    }
    let meta = fs::metadata(dir).map_err(|error| {
        (
            format!("cannot read the session folder {shown}: {error}"),
            "set XDG_RUNTIME_DIR to a folder you own".to_string(),
        )
    })?;
    let uid = rustix::process::getuid().as_raw();
    if !meta.is_dir() {
        return Err((
            format!("the session folder {shown} is not a folder"),
            format!("remove {shown}, or set XDG_RUNTIME_DIR to a folder you own"),
        ));
    }
    if meta.uid() != uid {
        return Err((
            format!(
                "the session folder {shown} is owned by another user (uid {}), not you (uid {uid})",
                meta.uid()
            ),
            "set XDG_RUNTIME_DIR to a folder you own".into(),
        ));
    }
    if meta.mode() & 0o777 != 0o700 {
        return Err((
            format!(
                "the session folder {shown} has mode {:o}, so other users can reach its sockets",
                meta.mode() & 0o777
            ),
            format!("run `chmod 700 {shown}`"),
        ));
    }
    Ok(())
}
