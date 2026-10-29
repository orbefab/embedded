use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde::Serialize;
use sim_cli::{Envelope, Failure, RunReport, Status};
use sim_core::timestamp::Timestamp;

#[derive(Parser)]
#[command(name = "sim", version, about = "Simulate a netlist into a run folder")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Simulate netlist.json and write the run folder to --out.
    Run {
        netlist: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// A folder of blobs, each file named by its SHA-256 in hex, searched after the
        /// `blobs` folder beside the netlist, in the order given. Repeatable.
        #[arg(long = "blobs", value_name = "DIR")]
        blobs: Vec<PathBuf>,
    },
    /// Write what each display part's glass showed at the end of a run folder, one PNG a part.
    View {
        run: PathBuf,
        /// The folder for `display-<part>.png`.
        #[arg(long)]
        out: PathBuf,
    },
    /// Report engine version, formats, and self-checks.
    Doctor,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // --help and --version print text and exit 0. Every other parse
        // failure is a usage_error document.
        Err(e) if !e.use_stderr() => {
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            let body = Failure {
                message: e.to_string(),
                path: None,
            };
            return emit(Status::UsageError, &body);
        }
    };
    match cli.command {
        Command::Run {
            netlist,
            out,
            blobs,
        } => run(&netlist, &out, &blobs),
        Command::View { run, out } => match sim_cli::view(&run, &out) {
            Ok(report) => emit(Status::Ok, &report),
            Err(failure) => emit(Status::IoError, &failure),
        },
        Command::Doctor => {
            let report = sim_cli::doctor();
            emit(report.status(), &report)
        }
    }
}

fn run(netlist: &Path, out: &Path, dirs: &[PathBuf]) -> ExitCode {
    let input = match fs::read(netlist) {
        Ok(bytes) => bytes,
        Err(e) => return emit(Status::IoError, &io_failure(netlist, e)),
    };
    let blobs = match sim_cli::blobs(&input, netlist, dirs) {
        Ok(blobs) => blobs,
        Err((path, e)) => return emit(Status::IoError, &io_failure(&path, e)),
    };
    let done = sim_cli::run_with(&input, &blobs, Timestamp::now());
    if let Err(e) = done.folder.write(out) {
        return emit(Status::IoError, &io_failure(out, e));
    }
    let report = RunReport::new(out, &done.results);
    emit(report.status(), &report)
}

fn io_failure(path: &Path, e: std::io::Error) -> Failure {
    Failure {
        message: e.to_string(),
        path: Some(path.display().to_string()),
    }
}

fn emit<T: Serialize>(status: Status, body: &T) -> ExitCode {
    let doc = Envelope { status, body };
    let text = serde_json::to_string_pretty(&doc).expect("report types serialize");
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{text}");
    ExitCode::from(status.exit_code())
}
