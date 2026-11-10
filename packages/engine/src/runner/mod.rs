//! What `orbe-run` reads and prints: the blobs a run reads, the report of a run, and the images
//! of a run's displays.

mod png;
mod view;

pub use view::{ViewReport, view};

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::blob::{Blobs, Sha256};
use crate::diagnostic::{Diagnostic, Severity};
use crate::run_folder::Results;
use crate::timestamp::Timestamp;
use serde::Serialize;

use crate::run_with;

/// The blobs a run offers the netlist `json` read from `netlist`. For each blob it names, the
/// first file under that hash, in the `blobs` folder beside the netlist when there is one and
/// then in each of `dirs`, whose bytes hash to it. When no file does, the first file found, so
/// the run reports its hash. A file is read only when the netlist names its hash, and a file that
/// cannot be read fails the lookup only when no later folder has the blob.
pub fn blobs(json: &[u8], netlist: &Path, dirs: &[PathBuf]) -> Result<Blobs, (PathBuf, io::Error)> {
    let beside = netlist
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("blobs");
    let beside = beside.is_dir().then_some(beside);
    for dir in dirs {
        fs::read_dir(dir).map_err(|e| (dir.clone(), e))?;
    }
    let mut blobs = Blobs::from_folders();
    'names: for name in crate::netlist::blob_names(json) {
        let mut other = None;
        let mut unreadable = None;
        for dir in beside.iter().chain(dirs) {
            let path = dir.join(name.to_string());
            match fs::read(&path) {
                Ok(bytes) if Sha256::of(&bytes) == name => {
                    blobs.insert(name, bytes.into());
                    continue 'names;
                }
                Ok(bytes) => {
                    other.get_or_insert(bytes);
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    unreadable.get_or_insert((path, e));
                }
            }
        }
        if let Some(failure) = unreadable {
            return Err(failure);
        }
        if let Some(bytes) = other {
            blobs.insert(name, bytes.into());
        }
    }
    Ok(blobs)
}

/// Every run ends in exactly one of these. The JSON `status` field and the process exit code
/// both come from this value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Exit 0. The run folder is complete. It may hold warnings.
    Ok,
    /// Exit 1. The run folder is complete and holds at least one error diagnostic.
    Failed,
    /// Exit 2. Bad command line. Nothing written.
    UsageError,
    /// Exit 3. Could not read the netlist or write the run folder. View: could not read the run
    /// folder or write an image.
    IoError,
}

impl Status {
    pub fn exit_code(self) -> u8 {
        match self {
            Status::Ok => 0,
            Status::Failed => 1,
            Status::UsageError => 2,
            Status::IoError => 3,
        }
    }
}

/// stdout body of a run.
#[derive(Serialize)]
pub struct RunReport<'a> {
    pub out: String,
    pub errors: usize,
    pub warnings: usize,
    /// Same list, same order, as diagnostics.json.
    pub diagnostics: &'a [Diagnostic],
}

impl<'a> RunReport<'a> {
    pub fn new(out: &Path, results: &'a Results) -> RunReport<'a> {
        RunReport {
            out: out.display().to_string(),
            errors: results.count(Severity::Error),
            warnings: results.count(Severity::Warning),
            diagnostics: results.diagnostics(),
        }
    }

    pub fn status(&self) -> Status {
        if self.errors > 0 {
            Status::Failed
        } else {
            Status::Ok
        }
    }
}

/// stdout body for `usage_error` and `io_error`.
#[derive(Serialize)]
pub struct Failure {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The stdout document: `{"status": .., ...body}`.
#[derive(Serialize)]
pub struct Envelope<'a, T: Serialize> {
    pub status: Status,
    #[serde(flatten)]
    pub body: &'a T,
}

/// Prints one JSON document and returns the process exit code for `status`.
pub fn emit<T: Serialize>(status: Status, body: &T) -> ExitCode {
    let doc = Envelope { status, body };
    let text = serde_json::to_string_pretty(&doc).expect("report types serialize");
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{text}");
    ExitCode::from(status.exit_code())
}

/// Reads `netlist`, runs it with the blobs it names, and writes the run folder to `out`. The
/// results are what `orbe-run run` reports. An `Err` is its `io_error`.
pub fn run_file(netlist: &Path, out: &Path, dirs: &[PathBuf]) -> Result<Results, Failure> {
    let input = fs::read(netlist).map_err(|e| io_failure(netlist, e))?;
    let blobs = blobs(&input, netlist, dirs).map_err(|(path, e)| io_failure(&path, e))?;
    let done = run_with(&input, &blobs, Timestamp::now());
    done.folder.write(out).map_err(|e| io_failure(out, e))?;
    Ok(done.results)
}

fn io_failure(path: &Path, e: io::Error) -> Failure {
    Failure {
        message: e.to_string(),
        path: Some(path.display().to_string()),
    }
}
