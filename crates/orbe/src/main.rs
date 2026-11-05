//! `orbe`, the CLI. `orbe check` type-checks a scene. `orbe add` copies catalog parts.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use orbe_scene::{Diagnostic, Reply, Request};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "orbe", version, about = "A hardware simulator for agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Type-checks a scene and every part file it uses, and prints each finding.
    Check {
        /// The scene file, such as scene.jsonc.
        scene: PathBuf,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Copies catalog parts into `parts/` and writes `parts.lock`.
    Add {
        /// Part ids. A version is `id@0.1.0`.
        ids: Vec<String>,
        /// The project folder, the one with scene.jsonc. The current folder by default.
        #[arg(short = 'C', value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Replace a copy that differs within the same major version.
        #[arg(long)]
        overwrite: bool,
        /// Show what would change, and write nothing.
        #[arg(long)]
        diff: bool,
        /// Rebuild `parts.lock` from the part folders.
        #[arg(long)]
        relock: bool,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
}

const FAILED: u8 = 1;
const IO: u8 = 3;

const USAGE: u8 = 2;

fn main() -> ExitCode {
    match Cli::try_parse() {
        Ok(cli) => match cli.command {
            Command::Check { scene, json } => check(&scene, json),
            Command::Add {
                ids,
                dir,
                overwrite,
                diff,
                relock,
                json,
            } => add(
                dir.as_deref(),
                Request {
                    ids,
                    overwrite,
                    diff,
                    relock,
                },
                json,
            ),
        },
        Err(err) => usage(err),
    }
}

/// A usage error with `--json` is one envelope on stdout. Help and version stay clap's.
fn usage(err: clap::Error) -> ExitCode {
    let json = std::env::args().any(|arg| arg == "--json" || arg.starts_with("--json="));
    if json && err.exit_code() == i32::from(USAGE) {
        print_json(&json!({
            "ok": false,
            "error": { "code": "usage", "message": err.to_string().trim().to_string() }
        }));
        return ExitCode::from(USAGE);
    }
    err.exit();
}

fn check(scene: &Path, json: bool) -> ExitCode {
    if !scene.is_file() {
        let message = format!("no scene file at {}", scene.display());
        if json {
            print_json(
                &json!({ "ok": false, "error": { "code": "io_error", "message": message } }),
            );
        } else {
            eprintln!("error[io_error]: {message}");
        }
        return ExitCode::from(IO);
    }
    let dir = scene.parent().unwrap_or(Path::new(""));
    match orbe_scene::read(scene) {
        Ok(netlist) => {
            if json {
                print_json(&json!({
                    "ok": true,
                    "data": {
                        "scene": scene.display().to_string(),
                        "parts": netlist.parts,
                        "nets": netlist.nets.len(),
                        "ground": netlist.ground,
                        "modified": netlist.modified,
                    },
                }));
            } else {
                println!(
                    "{}: ok, {} engine parts, {} nets, ground {}",
                    scene.display(),
                    netlist.parts,
                    netlist.nets.len(),
                    netlist.ground
                );
                for id in &netlist.modified {
                    println!(
                        "note: `{id}` differs from its parts.lock line; it has children and no model, so the edit is allowed"
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(diagnostics) => {
            let placed: Vec<Diagnostic> = diagnostics
                .into_iter()
                .map(|d| Diagnostic {
                    file: dir.join(&d.file).display().to_string(),
                    ..d
                })
                .collect();
            let count = format!(
                "{} error{} in {}",
                placed.len(),
                if placed.len() == 1 { "" } else { "s" },
                scene.display()
            );
            if json {
                let found: Vec<Value> = placed
                    .iter()
                    .map(|d| json!({ "code": d.code.as_str(), "file": d.file, "line": d.line, "column": d.column, "message": d.message, "hint": d.hint }))
                    .collect();
                print_json(
                    &json!({ "ok": false, "error": { "code": "scene_invalid", "message": count }, "data": { "diagnostics": found } }),
                );
            } else {
                for diagnostic in &placed {
                    println!("{diagnostic}");
                }
                println!("{count}");
            }
            ExitCode::from(FAILED)
        }
    }
}

fn add(dir: Option<&Path>, request: Request, json: bool) -> ExitCode {
    match orbe_scene::add_parts(dir, &request) {
        Ok(reply) => print_reply(reply, json),
        Err(error) => io_exit(&error.to_string(), json),
    }
}

fn print_reply(reply: Reply, json: bool) -> ExitCode {
    if json {
        print_json(&reply.json());
    } else if reply.code() == USAGE {
        eprintln!("{}", reply.text());
    } else {
        println!("{}", reply.text());
    }
    ExitCode::from(reply.code())
}

fn io_exit(message: &str, json: bool) -> ExitCode {
    if json {
        print_json(&json!({
            "ok": false,
            "error": { "code": "io_error", "message": message }
        }));
    } else {
        eprintln!("error[io_error]: {message}");
    }
    ExitCode::from(IO)
}

fn print_json(value: &Value) {
    println!("{}", serde_json::to_string(value).expect("a JSON value"));
}
