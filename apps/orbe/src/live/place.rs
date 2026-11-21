//! Where a session's process listens: the session's name, and the private folder that holds
//! one socket per session.

use std::fmt;
use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 1 to 64 ASCII letters, digits, `.`, `_` and `-`, starting with a letter or digit, so it is
/// one file name and never reads as a flag.
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

    /// The project folder above `cwd` (the nearest one with scene.jsonc or parts.lock), else
    /// the git worktree root, else `cwd`: its folder name and a short hash of its path.
    pub fn default_for(cwd: &Path) -> Name {
        let above = |marker: fn(&Path) -> bool| cwd.ancestors().find(|dir| marker(dir));
        let folder =
            above(|dir| dir.join("scene.jsonc").is_file() || dir.join("parts.lock").is_file())
                .or_else(|| above(|dir| dir.join(".git").exists()))
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

/// The folder of this user's sessions: `$XDG_RUNTIME_DIR/orbe`, else `<temp>/orbe-<uid>`.
/// It is the user's own, with mode 0700. Every version of orbe uses this one folder.
#[derive(Clone)]
pub struct Place {
    dir: PathBuf,
}

impl Place {
    /// Finds the folder, creating what is missing. A refusal is its message and hint.
    pub fn find() -> Result<Place, (String, String)> {
        let dir = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
            Some(runtime) if runtime.is_absolute() => runtime.join("orbe"),
            _ => std::env::temp_dir().join(format!("orbe-{}", rustix::process::getuid().as_raw())),
        };
        private(&dir)?;
        Ok(Place { dir })
    }

    /// The session's socket path, refused when it is longer than the system can bind.
    pub fn socket(&self, name: &Name) -> Result<PathBuf, (String, String)> {
        let socket = self.dir.join(format!("{name}.sock"));
        let length = socket.as_os_str().len();
        if length > SOCKET_PATH {
            return Err((
                format!(
                    "the socket path {} is {length} bytes, and the system takes at most {SOCKET_PATH}",
                    socket.display()
                ),
                "use a shorter `--session` name, or set XDG_RUNTIME_DIR to a shorter folder".into(),
            ));
        }
        Ok(socket)
    }

    /// What the session's process wrote to stderr, such as a panic, kept when it crashes.
    pub fn log(&self, name: &Name) -> PathBuf {
        self.dir.join(format!("{name}.log"))
    }

    /// The orbe version that started the session, one line, written when that process binds.
    pub fn version(&self, name: &Name) -> PathBuf {
        self.dir.join(format!("{name}.version"))
    }

    /// The log one start of the session's process writes until it holds the socket, so a
    /// start that loses to another never writes in the session's log.
    pub fn start_log(&self, name: &Name, id: u64) -> PathBuf {
        self.dir.join(format!("{name}.log.{id}"))
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

    /// Removes the start logs no process holds, of the sessions `of` picks, and returns them
    /// sorted. The client that makes a start log locks it before the process starts, and the
    /// process holds that lock for as long as it runs.
    pub fn remove_left_logs(&self, of: impl Fn(&Name) -> bool) -> Vec<(Name, PathBuf)> {
        use rustix::fs::{FlockOperation, Mode, OFlags};
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut left: Vec<(Name, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file = entry.file_name();
                let (stem, id) = file.to_str()?.rsplit_once(".log.")?;
                id.parse::<u64>().ok()?;
                let name = Name::parse(stem).ok().filter(|name| of(name))?;
                let path = entry.path();
                let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
                let fd = rustix::fs::open(&path, flags, Mode::empty()).ok()?;
                rustix::fs::flock(&fd, FlockOperation::NonBlockingLockExclusive).ok()?;
                fs::remove_file(&path).ok()?;
                Some((name, path))
            })
            .collect();
        left.sort_by(|a, b| a.1.cmp(&b.1));
        left
    }

    /// `<name>.log.crashed`, the output of a session that ended without closing. The next crash
    /// replaces it. The name is not a session's log, socket, version or start log: a session
    /// may contain `.`, and a start log's id is a number.
    pub fn crashed_log(&self, name: &Name) -> PathBuf {
        self.dir.join(format!("{name}.log.crashed"))
    }

    /// Moves `<name>.log` onto the crash log whenever that log is there, empty or not, so a
    /// crash that wrote nothing replaces an older crash's output. When `<name>.log` is missing,
    /// an earlier command already took this crash's log and the crash log stays. An empty crash
    /// log is removed. The path when the crash log then holds output.
    pub fn take_crash_log(&self, name: &Name) -> Option<PathBuf> {
        let log = self.log(name);
        let crashed = self.crashed_log(name);
        if fs::symlink_metadata(&log).is_ok() {
            let _ = fs::rename(&log, &crashed);
        }
        if holds_output(&crashed) {
            Some(crashed)
        } else {
            let _ = fs::remove_file(&crashed);
            None
        }
    }
}

pub(super) fn holds_output(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

/// The longest socket path: `sun_path`, less its closing NUL.
#[cfg(target_os = "linux")]
const SOCKET_PATH: usize = 107;
#[cfg(not(target_os = "linux"))]
const SOCKET_PATH: usize = 103;

/// Creates `dir` with mode 0700, or checks that the one there is a folder, not a symlink, that
/// the user owns and only the user can open. Inside a sticky temp folder no other user can
/// then rename or replace it.
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
    let meta = fs::symlink_metadata(dir).map_err(|error| {
        (
            format!("cannot read the session folder {shown}: {error}"),
            "set XDG_RUNTIME_DIR to a folder you own".to_string(),
        )
    })?;
    let uid = rustix::process::getuid().as_raw();
    if meta.is_symlink() {
        return Err((
            format!("the session folder {shown} is a symlink, not a folder"),
            format!("remove it with `rm {shown}`, or set XDG_RUNTIME_DIR to a folder you own"),
        ));
    }
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
