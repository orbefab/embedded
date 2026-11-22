//! `orbe test`: each test file is a transcript of live commands, run in a fresh session and
//! stopped at its first failed line.

use std::path::{Component, Path, PathBuf};

use orbe_scene::Files;

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
        self.refused
            .as_ref()
            .or_else(|| self.failure().map(|line| &line.reply))
            .map_or(0, Reply::exit)
    }
}

/// Runs a test file's text on a scene in `files`. Paths in it start from `root`, the project
/// folder above `tests/`.
pub fn run(files: Files, file: &Path, text: &str, root: &Path) -> Report {
    let mut live = Live::new(file.display().to_string(), files);
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let reply = match Command::parse(line, root) {
            Ok(None) => continue,
            Ok(Some(Command::Play | Command::Pause)) => live.refuse(Error::new(
                Code::Usage,
                "`play` and `pause` pace time to the wall clock, which a test does not run on",
                "advance time with `step`, such as `step 100ms`",
            )),
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

/// Finds a test file's project from `real`, its path with every symlink resolved: the nearest
/// folder above it that holds `scene.jsonc` or a `parts/` folder, as `orbe add` finds one. The file
/// must sit under that folder's `tests/`, resolved too, so a linked `tests/` is its target.
/// `named` is the path as given, and `cwd` is the working folder, resolved. A refusal is its
/// message and hint.
pub fn locate(real: &Path, named: &Path, cwd: &Path) -> Result<Located, (String, String)> {
    let really = format!("its real path is {}; ", real.display());
    let Some(project) = real.ancestors().skip(1).find(|dir| is_project(dir)) else {
        return Err((
            format!("{} is in no orbe project", named.display()),
            format!(
                "{really}no folder above it holds scene.jsonc or parts/; put it in the project's tests/ folder, beside scene.jsonc"
            ),
        ));
    };
    let root = shown(project, cwd);
    let tests = project.join("tests");
    if !real.starts_with(tests.canonicalize().unwrap_or(tests)) {
        let named_root = if root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &root
        };
        return Err((
            format!("{} is not in its project's tests/ folder", named.display()),
            format!(
                "{really}its project is {}, the nearest folder above it with scene.jsonc or parts/; put the file in {}/",
                named_root.display(),
                root.join("tests").display()
            ),
        ));
    }
    Ok(Located {
        file: shown(real, cwd),
        root,
    })
}

/// The working folder with every symlink resolved, which a relative path starts from.
pub fn cwd() -> std::io::Result<PathBuf> {
    std::env::current_dir()?.canonicalize()
}

fn is_project(dir: &Path) -> bool {
    dir.join("scene.jsonc").is_file() || dir.join("parts").is_dir()
}

/// `path` from `cwd`, both resolved: below it as is, else up with `..` to a folder they share
/// below the root, else whole.
pub fn shown(path: &Path, cwd: &Path) -> PathBuf {
    if let Ok(below) = path.strip_prefix(cwd) {
        return below.to_path_buf();
    }
    let Some(common) = cwd
        .ancestors()
        .find(|dir| dir.parent().is_some() && path.starts_with(dir))
    else {
        return path.to_path_buf();
    };
    let mut up: PathBuf = cwd
        .strip_prefix(common)
        .into_iter()
        .flat_map(Path::components)
        .map(|_| Component::ParentDir)
        .collect();
    up.push(path.strip_prefix(common).unwrap_or(path));
    up
}
