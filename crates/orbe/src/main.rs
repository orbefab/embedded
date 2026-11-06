//! `orbe`, the CLI. `orbe check` type-checks a scene. `orbe add` copies catalog parts.
//! `orbe test` runs each test file's live commands in a fresh in-process session. The live
//! commands, such as `orbe open` and `orbe step`, drive a session that a background process
//! holds between commands.

#[cfg(unix)]
mod live;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use orbe_scene::{Diagnostic, Reply, Request};
use orbe_session::test::{self, Report};
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
    /// Runs each test file, a transcript of live commands, in a fresh session, and stops a file
    /// at its first failed line.
    Test {
        /// Test files, or project folders whose tests/*.test run. The current folder by default.
        paths: Vec<PathBuf>,
        /// Print every command with its reply.
        #[arg(short, long)]
        verbose: bool,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
    /// The session's background process, which `orbe open` starts.
    #[command(hide = true)]
    Daemon {
        #[arg(long)]
        session: String,
    },
}

const FAILED: u8 = 1;
const IO: u8 = 3;

const USAGE: u8 = 2;

fn main() -> ExitCode {
    let name = std::env::args_os().nth(1);
    if let Some(spec) = name
        .and_then(|n| n.into_string().ok())
        .and_then(|n| orbe_session::spec(&n))
    {
        let args: Result<Vec<String>, _> = std::env::args_os()
            .skip(2)
            .map(|a| a.into_string())
            .collect();
        let Ok(args) = args else {
            eprintln!("error[usage]: `orbe {}` takes UTF-8 arguments", spec.name);
            return ExitCode::from(USAGE);
        };
        return live_command(spec, &args);
    }
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
            Command::Test {
                paths,
                verbose,
                json,
            } => run_tests(&paths, verbose, json),
            Command::Daemon { session } => daemon(&session),
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

#[cfg(unix)]
fn live_command(spec: &orbe_session::Spec, args: &[String]) -> ExitCode {
    live::main(spec, args)
}

#[cfg(unix)]
fn daemon(session: &str) -> ExitCode {
    live::daemon(session)
}

#[cfg(not(unix))]
fn live_command(spec: &orbe_session::Spec, args: &[String]) -> ExitCode {
    let message = format!("`orbe {}` needs Unix for now", spec.name);
    let hint = format!(
        "write `{}` in a tests/<name>.test file and run `orbe test`",
        spec.usage
    );
    if args.iter().any(|arg| arg == "--json") {
        print_json(&json!({
            "ok": false,
            "error": { "code": "usage", "message": message, "hint": hint }
        }));
    } else {
        eprintln!("error[usage]: {message}\n  hint: {hint}");
    }
    ExitCode::from(USAGE)
}

#[cfg(not(unix))]
fn daemon(_session: &str) -> ExitCode {
    eprintln!("error[usage]: the session process needs Unix for now");
    ExitCode::from(USAGE)
}

/// Every test file the paths name, with its project folder: a file itself, or a folder's
/// tests/*.test, sorted.
fn test_files(paths: &[PathBuf]) -> Result<Vec<(PathBuf, PathBuf)>, (u8, String, String)> {
    let paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    };
    let cwd = test::cwd().map_err(|error| {
        (
            IO,
            format!("cannot read the working folder: {error}"),
            "run `orbe test` from a folder that exists".to_string(),
        )
    })?;
    let missing = |path: &Path| {
        (
            IO,
            format!("no test file or folder at {}", path.display()),
            "name a .test file or a project folder".to_string(),
        )
    };
    let locate = |named: &Path| {
        let real = named.canonicalize().map_err(|_| missing(named))?;
        test::locate(&real, named, &cwd)
            .map(|found| (found.file, found.root))
            .map_err(|(message, hint)| (USAGE, message, hint))
    };
    let mut files = Vec::new();
    for path in &paths {
        let real = path.canonicalize().map_err(|_| missing(path))?;
        if real.is_file() {
            files.push(locate(path)?);
            continue;
        }
        if !real.is_dir() {
            return Err(missing(path));
        }
        let tests = real.join("tests");
        let shown = test::shown(&tests, &cwd);
        let unreadable = |error: std::io::Error| {
            (
                IO,
                format!("cannot read {}: {error}", shown.display()),
                "make the folder readable".to_string(),
            )
        };
        let entries = match std::fs::read_dir(&tests) {
            Ok(entries) => entries.collect::<Result<Vec<_>, _>>().map_err(unreadable)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(unreadable(error)),
        };
        let mut found: Vec<PathBuf> = entries
            .into_iter()
            .map(|entry| entry.path())
            .filter(|p| p.extension().is_some_and(|e| e == "test"))
            .map(|p| test::shown(&p, &cwd))
            .collect();
        if found.is_empty() {
            return Err((
                USAGE,
                format!("no tests in {}", shown.display()),
                format!(
                    "its real path is {}; write a tests/<name>.test file of live commands with `expect` lines",
                    tests.display()
                ),
            ));
        }
        found.sort();
        for file in found {
            files.push(locate(&file)?);
        }
    }
    Ok(files)
}

fn run_tests(paths: &[PathBuf], verbose: bool, json: bool) -> ExitCode {
    let files = match test_files(paths) {
        Ok(files) => files,
        Err((code, message, hint)) => {
            let name = if code == IO { "io_error" } else { "usage" };
            if json {
                print_json(&json!({
                    "ok": false,
                    "error": { "code": name, "message": message, "hint": hint }
                }));
            } else {
                eprintln!("error[{name}]: {message}\n  hint: {hint}");
            }
            return ExitCode::from(code);
        }
    };
    let mut reports = Vec::new();
    for (file, root) in &files {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) => return io_exit(&format!("{}: {error}", file.display()), json),
        };
        let report = test::run(file, &text, root);
        if !json {
            print_report(&report, verbose);
        }
        reports.push(report);
    }
    let exit = reports.iter().map(Report::exit).max().unwrap_or(0);
    let failed = reports.iter().filter(|r| !r.passed()).count();
    let passed = reports.len() - failed;
    if json {
        print_json(&json!({
            "ok": failed == 0,
            "data": {
                "passed": passed,
                "failed": failed,
                "tests": reports.iter().map(|r| report_json(r, verbose)).collect::<Vec<_>>(),
            },
        }));
    } else {
        println!("{passed} passed, {failed} failed");
    }
    ExitCode::from(exit)
}

fn print_report(report: &Report, verbose: bool) {
    let file = report.file.display();
    if verbose {
        for line in &report.lines {
            println!("{file}:{}> {}", line.number, line.text);
            for text in line.reply.text().lines() {
                println!("  {text}");
            }
        }
    }
    if let Some(refused) = &report.refused {
        println!("FAIL {file}");
        for text in refused.text().lines() {
            println!("  {text}");
        }
        return;
    }
    let Some(failure) = report.failure() else {
        println!("PASS {file}");
        return;
    };
    println!("FAIL {file}:{}: {}", failure.number, failure.text);
    if !verbose {
        for text in failure.reply.text().lines() {
            println!("  {text}");
        }
    }
}

fn report_json(report: &Report, verbose: bool) -> Value {
    let mut entry = json!({ "file": report.file.display().to_string(), "ok": report.passed() });
    if let Some(refused) = &report.refused {
        entry["error"] = refused.json()["error"].clone();
    }
    if let Some(failure) = report.failure() {
        let reply = &failure.reply;
        let envelope = reply.json();
        entry["line"] = json!(failure.number);
        entry["command"] = json!(failure.text);
        entry["error"] = envelope["error"].clone();
        if let Some(check) = reply.check() {
            entry["expected"] = json!(check.expected);
            entry["actual"] = json!(check.actual);
        }
        if let Some(data) = envelope.get("data") {
            entry["data"] = data.clone();
        }
    }
    if verbose {
        entry["replies"] = report
            .lines
            .iter()
            .map(|l| json!({ "line": l.number, "command": l.text, "reply": l.reply.json() }))
            .collect();
    }
    entry
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
