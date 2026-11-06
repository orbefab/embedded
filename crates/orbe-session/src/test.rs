//! `orbe test`: each test file is a transcript of live commands, run in a fresh session and
//! stopped at its first failed line.

use std::path::{Component, Path, PathBuf};

use crate::command::Command;
use crate::live::Live;
use crate::reply::{Code, Error, Reply};

#[derive(Clone, Debug)]
pub struct Line {
    pub number: usize,
    pub text: String,
    pub reply: Reply,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub file: PathBuf,
    /// Every line that ran. The last one failed when the test failed.
    pub lines: Vec<Line>,
    /// Why the file as a whole did not run, such as a file with no commands.
    pub refused: Option<Reply>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.refused.is_none() && self.failure().is_none()
    }

    pub fn failure(&self) -> Option<&Line> {
        self.lines.last().filter(|line| !line.reply.ok())
    }

    /// `orbe check`'s exit codes: 0 when it passed, 1 for a failed check or any other refusal
    /// at run time, 2 for a malformed line or file, 3 for an io error.
    pub fn exit(&self) -> u8 {
        let reply = self
            .refused
            .as_ref()
            .or_else(|| self.failure().map(|line| &line.reply));
        match reply.and_then(|r| r.error.as_ref()).map(|e| &e.code) {
            None => 0,
            Some(Code::Usage) => 2,
            Some(Code::IoError) => 3,
            Some(_) => 1,
        }
    }
}

/// Runs a test file's text. Paths in it start from `root`, the project folder above `tests/`.
pub fn run(file: &Path, text: &str, root: &Path) -> Report {
    let mut live = Live::new(file.display().to_string());
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let reply = match Command::parse(line, root) {
            Ok(None) => continue,
            Ok(Some(command)) => live.run(&command),
            Err(reason) => live.refuse(Error::new(Code::Usage, reason, "")),
        };
        let ok = reply.ok();
        lines.push(Line {
            number: index + 1,
            text: line.trim().to_string(),
            reply,
        });
        if !ok {
            break;
        }
    }
    let refused = lines.is_empty().then(|| {
        live.refuse(Error::new(
            Code::Usage,
            format!("{} has no commands", file.display()),
            "write live commands with `expect` lines, such as `open` and `expect @led1 off`",
        ))
    });
    Report {
        file: file.to_path_buf(),
        lines,
        refused,
    }
}

/// A test file and its project folder, as paths from the working folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Located {
    pub file: PathBuf,
    pub root: PathBuf,
}

/// Finds a test file's project, the nearest folder above it that holds `scene.jsonc` or
/// `parts.lock`, as `orbe add` finds one. The file must sit under that folder's `tests/`.
/// `..` is read from the path's text. `cwd` is the working folder, as `cwd` names it. A
/// refusal is its message and hint.
pub fn locate(file: &Path, cwd: &Path) -> Result<Located, (String, String)> {
    let path = lexical(&cwd.join(file));
    let Some(project) = path.ancestors().skip(1).find(|dir| is_project(dir)) else {
        return Err((
            format!("{} is in no orbe project", file.display()),
            "no folder above it holds scene.jsonc or parts.lock; put it in the project's tests/ folder, beside scene.jsonc".into(),
        ));
    };
    let root = shown(project, cwd);
    if !path.starts_with(project.join("tests")) {
        let named = if root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &root
        };
        return Err((
            format!("{} is not in its project's tests/ folder", file.display()),
            format!(
                "its project is {}, the nearest folder above it with scene.jsonc or parts.lock; put the file in {}/",
                named.display(),
                root.join("tests").display()
            ),
        ));
    }
    Ok(Located {
        file: shown(&path, cwd),
        root,
    })
}

/// The working folder as the shell names it: `$PWD` when that is the same folder as the
/// process's, so a folder reached through a symlink keeps its name, else the process's.
pub fn cwd() -> std::io::Result<PathBuf> {
    let process = std::env::current_dir()?;
    let shell = std::env::var_os("PWD")
        .map(PathBuf::from)
        .filter(|pwd| pwd.is_absolute() && same(pwd, &process));
    Ok(shell.map_or(process, |pwd| lexical(&pwd)))
}

fn is_project(dir: &Path) -> bool {
    dir.join("scene.jsonc").is_file() || dir.join("parts.lock").is_file()
}

fn same(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// An absolute path with each `..` taken off the name before it.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// `path` from `cwd`: below it as is, else with `..` when that reaches the same file through
/// the process's folder, else whole.
fn shown(path: &Path, cwd: &Path) -> PathBuf {
    if let Ok(below) = path.strip_prefix(cwd) {
        return below.to_path_buf();
    }
    let Some(common) = cwd.ancestors().find(|dir| path.starts_with(dir)) else {
        return path.to_path_buf();
    };
    let mut up: PathBuf = cwd
        .strip_prefix(common)
        .into_iter()
        .flat_map(Path::components)
        .map(|_| Component::ParentDir)
        .collect();
    up.push(path.strip_prefix(common).unwrap_or(path));
    if same(&up, path) {
        up
    } else {
        path.to_path_buf()
    }
}
