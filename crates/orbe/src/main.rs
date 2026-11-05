//! `orbe`, the CLI. This first version has one command, `orbe check`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use orbe_scene::Diagnostic;
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
}

const FAILED: u8 = 1;
const IO: u8 = 3;

const USAGE: u8 = 2;

fn main() -> ExitCode {
    match Cli::try_parse() {
        Ok(cli) => match cli.command {
            Command::Check { scene, json } => check(&scene, json),
        },
        Err(err) => usage(err),
    }
}

/// A usage error with `--json` is one envelope on stdout. Help and version stay clap's.
fn usage(err: clap::Error) -> ExitCode {
    let json = std::env::args().any(|arg| arg == "--json");
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

fn print_json(value: &Value) {
    println!("{}", serde_json::to_string(value).expect("a JSON value"));
}
