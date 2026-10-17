mod doctor;

use std::path::Path;

use serde::Serialize;
use sim_core::diagnostic::{Diagnostic, Severity};
use sim_core::netlist;
use sim_core::run_folder::{self, Results, RunFolder};
use sim_core::timestamp::Timestamp;

pub use doctor::{Check, DoctorReport, doctor};

pub struct Completed {
    pub results: Results,
    pub folder: RunFolder,
}

/// netlist.json bytes to a run folder. Never fails. A rejected netlist is a
/// folder whose diagnostics say why.
pub fn run(input: &[u8], created_at: Timestamp) -> Completed {
    let (netlist, results) = match netlist::parse(input) {
        Ok(n) => {
            let r = sim_circuit::simulate(&n);
            (Some(n), r)
        }
        Err(d) => (None, Results::rejected(d)),
    };
    let folder = run_folder::render(input, netlist.as_ref(), &results, created_at);
    Completed { results, folder }
}

/// Every `sim` invocation ends in exactly one of these. The JSON `status`
/// field and the process exit code both come from this value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Exit 0. The run folder is complete. It may hold warnings.
    Ok,
    /// Exit 1. The run folder is complete and holds at least one error
    /// diagnostic. Doctor: a check failed.
    Failed,
    /// Exit 2. Bad command line. Nothing written.
    UsageError,
    /// Exit 3. Could not read the netlist or write the run folder.
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

/// `sim run` stdout body.
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
