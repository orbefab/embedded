//! `orbe test`: each test file is a transcript of live commands, run in a fresh session and
//! stopped at its first failed line.

use std::path::{Path, PathBuf};

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
}

impl Report {
    pub fn passed(&self) -> bool {
        self.lines.last().is_none_or(|line| line.reply.ok())
    }

    pub fn failure(&self) -> Option<&Line> {
        self.lines.last().filter(|line| !line.reply.ok())
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
    Report {
        file: file.to_path_buf(),
        lines,
    }
}

/// The project folder a test file belongs to: the folder above the one that holds it.
pub fn root(file: &Path) -> PathBuf {
    file.parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}
