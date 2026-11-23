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

    /// The project folder above `cwd` (the nearest one with scene.jsonc or parts/), else
    /// the git worktree root, else `cwd`: its folder name and a short hash of its path.
    pub fn default_for(cwd: &Path) -> Name {
        let above = |marker: fn(&Path) -> bool| cwd.ancestors().find(|dir| marker(dir));
        let folder = above(|dir| dir.join("scene.jsonc").is_file() || dir.join("parts").is_dir())
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

/// The folder of this user's sessions: `$XDG_RUNTIME_DIR/orbe`, else `/tmp/orbe-<uid>`.
/// `/tmp` keeps the socket path under the system's limit on macOS, where the default
/// temporary folder is already long. The folder is the user's own, mode 0700.
#[derive(Clone)]
pub struct Place {
    dir: PathBuf,
}

impl Place {
    /// Finds the folder, creating what is missing. A refusal is its message and hint.
    pub fn find() -> Result<Place, (String, String)> {
        let dir = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
            Some(runtime) if runtime.is_absolute() => runtime.join("orbe"),
            _ => PathBuf::from(format!("/tmp/orbe-{}", rustix::process::getuid().as_raw())),
        };
        private(&dir)?;
        Ok(Place { dir })
    }

    /// `<runtime>/orbe/<name>/`, the session's pid, socket, log and crash files.
    pub fn state(&self, name: &Name) -> PathBuf {
        self.dir.join(name.as_str())
    }

    /// Creates the session's folder, after refusing a socket path the system cannot bind.
    /// A refusal is its message and hint.
    pub fn ensure(&self, name: &Name) -> Result<PathBuf, (String, String)> {
        let _ = self.socket(name)?;
        let state = self.state(name);
        private(&state)?;
        Ok(state)
    }

    /// The session's socket path, refused when it is longer than the system can bind.
    pub fn socket(&self, name: &Name) -> Result<PathBuf, (String, String)> {
        let socket = self.state(name).join("sock");
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

    pub fn pid(&self, name: &Name) -> PathBuf {
        self.state(name).join("pid")
    }

    /// What the session's process writes to stderr.
    pub fn log(&self, name: &Name) -> PathBuf {
        self.state(name).join("log")
    }

    /// The orbe version that started the session, one line, written when that process binds.
    pub fn version(&self, name: &Name) -> PathBuf {
        self.state(name).join("version")
    }

    /// Held for as long as the session's process runs, so a second start leaves it alone.
    pub fn lock(&self, name: &Name) -> PathBuf {
        self.state(name).join("lock")
    }

    /// `<pid>.crashed`, the output of a session that ended without closing.
    pub fn crashed(&self, name: &Name, pid: u32) -> PathBuf {
        self.state(name).join(format!("{pid}.crashed"))
    }

    /// Every session that still has a socket or a pid file, sorted.
    pub fn names(&self) -> io::Result<Vec<Name>> {
        let mut names: Vec<Name> = fs::read_dir(&self.dir)?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file = entry.file_name();
                let name = Name::parse(file.to_str()?).ok()?;
                let state = entry.path();
                (state.join("sock").exists() || state.join("pid").exists()).then_some(name)
            })
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(names)
    }

    /// The pid in the pid file, when it is one number.
    pub fn read_pid(&self, name: &Name) -> Option<u32> {
        let text = fs::read_to_string(self.pid(name)).ok()?;
        text.trim().parse().ok()
    }
}

/// `kill(pid, 0)`: the process exists and is not a zombie. A zombie has ended; its parent
/// has not reaped it yet. A client that sees the socket close must still keep the crash log.
pub(super) fn pid_alive(pid: u32) -> bool {
    let Some(raw) = rustix::process::Pid::from_raw(pid as i32) else {
        return false;
    };
    rustix::process::test_kill_process(raw).is_ok() && !zombie(pid)
}

fn zombie(pid: u32) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit(") ")
        .next()
        .is_some_and(|rest| rest.starts_with('Z'))
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
