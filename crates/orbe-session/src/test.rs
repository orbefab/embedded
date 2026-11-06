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

/// The project folder a test file belongs to: the folder above the nearest folder named
/// `tests` that holds it, as a path from where `file` starts. `None` when no folder above the
/// file is named `tests`.
pub fn root(file: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(file).ok()?;
    let mut folders = Vec::new();
    for component in absolute.parent()?.components() {
        match component {
            Component::ParentDir => {
                folders.pop();
            }
            Component::CurDir => {}
            other => folders.push(other.as_os_str().to_owned()),
        }
    }
    let up = folders.iter().rev().position(|name| name == "tests")? + 1;
    let mut root = file.parent().unwrap_or(Path::new("")).to_path_buf();
    for _ in 0..up {
        match root.components().next_back() {
            Some(Component::Normal(_)) => {
                root.pop();
            }
            _ => root.push(".."),
        }
    }
    Some(root)
}
