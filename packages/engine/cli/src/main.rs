use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use orbe_engine_cli::{Failure, Status, emit, run_file};

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
        } => run_file(&netlist, &out, &blobs),
        Command::View { run, out } => match orbe_engine_cli::view(&run, &out) {
            Ok(report) => emit(Status::Ok, &report),
            Err(failure) => emit(Status::IoError, &failure),
        },
    }
}
