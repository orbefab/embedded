//! `orbe`, the CLI. `orbe check` type-checks a scene, and `orbe lint` checks it, then warns of
//! what is likely wrong or unclear. `orbe add` copies catalog parts, or installs a Block. `orbe part` prints one
//! catalog part, and `orbe search` finds parts. `orbe registry build` turns a folder of parts and
//! Blocks into a registry.
//! `orbe test` runs each test file's live commands in a fresh in-process session. The live
//! commands, such as `orbe open` and `orbe step`, drive a session that a background process
//! holds between commands. `orbe skills` prints the skills that tell an agent how to use it.

#[cfg(unix)]
mod live;
mod part;
mod skills;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Arg, ArgAction, CommandFactory, FromArgMatches, Parser, Subcommand};
use orbe_scene::{Diagnostic, Files};
#[cfg(unix)]
use orbe_scene::{Reply, Request};
use orbe_session::test::{self, Report};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "orbe",
    version,
    about = "A hardware simulator for agents",
    after_help = "An agent new to orbe runs `orbe skills get core` first."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Type-checks a scene and every part file it uses, and prints each finding.
    #[command(display_order = 1)]
    Check {
        /// The scene file, such as scene.jsonc.
        scene: PathBuf,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Runs `check`, then on a scene that checks prints each warning of what is likely wrong
    /// or unclear. Warnings never fail it.
    #[command(display_order = 2)]
    Lint {
        /// The scene file, such as scene.jsonc.
        scene: PathBuf,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Copies catalog parts into `parts/` and writes `parts.lock`, or installs a Block by
    /// copying its parts first, then writing its scene, firmware and tests.
    #[command(display_order = 3)]
    Add {
        /// Part ids, or one Block id. A part's version is `id@0.1.0`.
        ids: Vec<String>,
        /// The project folder, the one with scene.jsonc. The current folder by default.
        #[arg(short = 'C', value_name = "DIR", allow_hyphen_values = true)]
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
    /// Prints one catalog part.
    #[command(display_order = 4)]
    Part {
        /// Catalog id, such as `led-5mm`. A whole word of exactly one id, such as `1n4007`, names that part.
        id: String,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Finds catalog parts whose id, title, maker, number, summary or tags hold every word.
    #[command(display_order = 5)]
    Search {
        /// Words to match. Every word must appear.
        #[arg(required = true)]
        words: Vec<String>,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Runs each test file, a transcript of live commands, in a fresh session, and stops a file
    /// at its first failed line.
    #[command(display_order = 20)]
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
    /// Opens a viewer on a running session: a page the session's process serves on 127.0.0.1,
    /// in the default browser. Prints the page's URL.
    #[command(display_order = 19, after_help = SHOW_HELP)]
    Show {
        /// The session, instead of ORBE_SESSION or the project's default name.
        #[arg(long, value_name = "NAME")]
        session: Option<String>,
        /// Print the URL, and open no browser.
        #[arg(long)]
        no_browser: bool,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Builds registries of parts and Blocks.
    #[command(display_order = 22)]
    Registry {
        #[command(subcommand)]
        command: RegistryCommand,
    },
    /// Prints the skills that tell an agent how to use orbe.
    #[command(display_order = 21)]
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },
    /// The session's background process, which `orbe open` starts.
    #[command(hide = true)]
    Daemon {
        #[arg(long)]
        session: String,
        #[arg(long)]
        start_log: Option<u64>,
    },
}

#[derive(Subcommand)]
enum RegistryCommand {
    /// Writes the registry `<SRC>/registry.jsonc` names, of the parts in `<SRC>/parts/` and the
    /// Blocks in `<SRC>/blocks/`, into the empty folder `<OUT>`: an index, one record per item
    /// version, and each file once by its SHA-256. Prints the file count, and warns of what
    /// parts and Blocks lack and of hidden or large files they publish.
    Build {
        /// The folder that holds `registry.jsonc`, `parts/` and `blocks/`, such as this repo's root.
        src: PathBuf,
        /// The folder to write. It must be missing or empty.
        out: PathBuf,
        /// The folder of the build this one replaces. Every record it serves stays served: a
        /// version `<SRC>` still holds must build to the same bytes, and older versions are
        /// carried forward.
        #[arg(long, value_name = "DIR")]
        previous: Option<PathBuf>,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum SkillsCommand {
    /// Lists the skills with what each covers.
    List {
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Prints one skill, such as `core`.
    Get {
        name: String,
        /// Print one JSON envelope instead of text.
        #[arg(long)]
        json: bool,
    },
}

/// The live commands parse their own words, from the table in `orbe-session`. Clap only
/// describes them, so `orbe --help` and `orbe <command> --help` list them.
fn cli() -> clap::Command {
    let live = orbe_session::COMMANDS
        .iter()
        .zip(6..)
        .map(|(spec, order)| live_help(spec).display_order(order));
    Cli::command().subcommands(live)
}

const LIVE_HELP: &str = "Refs: @btn1, @nano.D13, @led1.i. Conditions: @led1 on, @nano.D13 high, @led1.i '>' 5mA, serial contains \"text\", no diagnostics. Durations: 100ms, 1.5ms, 20us. `orbe skills get core` explains each.";

const SHOW_HELP: &str = "The URL holds a token that only this session's process accepts, so keep it to yourself. Several viewers may watch one session. Closing the page leaves the session running, and `orbe close` ends the viewers too. A person can press buttons in the page, and `play` and `pause` there are the CLI's own. ORBE_VIEWER_DIR serves a built page's folder in place of the one built into orbe. An orbe built without the page says how to build it.";

const SWITCH_HELP: &str = "The input lands at the session's time t, and reads show it after t. On a switch with `bounce`, an input inside the bounce window of the one before lands after that window. The reply gives the time.";

fn live_help(spec: &orbe_session::Spec) -> clap::Command {
    let usage: Vec<String> = spec
        .usage
        .split(" | ")
        .map(|form| format!("orbe {form} [OPTIONS]"))
        .collect();
    let mut command = clap::Command::new(spec.name)
        .about(spec.about.trim_end_matches('.'))
        .override_usage(usage.join("\n       "))
        .arg(
            Arg::new("session")
                .long("session")
                .value_name("NAME")
                .help("The session, instead of ORBE_SESSION or the project's default name"),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Print one JSON envelope instead of text"),
        );
    if ["press", "release"].contains(&spec.name) {
        command = command.after_help(SWITCH_HELP);
    } else if ["<ref>", "<condition>", "<duration>"]
        .iter()
        .any(|word| spec.usage.contains(word))
    {
        command = command.after_help(LIVE_HELP);
    }
    if spec.name == "open" {
        command = command
            .arg(
                Arg::new("headed")
                    .long("headed")
                    .action(ArgAction::SetTrue)
                    .help("Then run `orbe show`, which opens a viewer on the session. When `show` fails, the session stays open"),
            )
            .arg(
                Arg::new("no-browser")
                    .long("no-browser")
                    .action(ArgAction::SetTrue)
                    .help("With --headed, print the viewer's URL and open no browser"),
            );
    }
    if spec.name == "close" {
        command = command.arg(
            Arg::new("all")
                .long("all")
                .action(ArgAction::SetTrue)
                .help("Close every session of this orbe"),
        );
    }
    command
}

const FAILED: u8 = 1;
const IO: u8 = 3;

const USAGE: u8 = 2;

fn main() -> ExitCode {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if let Some(exit) = flag_before_command(&args[1..]) {
        return exit;
    }
    if args.get(1).is_some_and(|a| a == "help") {
        let words = args.len();
        args.retain(|a| a != "--help" && a != "-h");
        if args.len() == 2 && words > 2 {
            args.push("help".into());
        }
    }
    if let Some(spec) = args
        .get(1)
        .and_then(|n| n.to_str())
        .and_then(orbe_session::spec)
    {
        let args: Result<Vec<String>, _> = std::env::args_os()
            .skip(2)
            .map(|a| a.into_string())
            .collect();
        let Ok(args) = args else {
            eprintln!("error[usage]: `orbe {}` takes UTF-8 arguments", spec.name);
            return ExitCode::from(USAGE);
        };
        if args.iter().any(|arg| arg == "--help" || arg == "-h") {
            let help = cli().try_get_matches_from(["orbe", spec.name, "--help"]);
            return usage(help.expect_err("--help stops the parse"));
        }
        return live_command(spec, &args);
    }
    let parsed = cli()
        .try_get_matches_from(args)
        .and_then(|matches| Cli::from_arg_matches(&matches));
    match parsed {
        Ok(cli) => match cli.command {
            Command::Check { scene, json } => check(&scene, json),
            Command::Lint { scene, json } => lint(&scene, json),
            Command::Add {
                ids,
                dir,
                overwrite,
                diff,
                relock,
                json,
            } => add(dir.as_deref(), ids, overwrite, diff, relock, json),
            Command::Part { id, json } => show_part(&id, json),
            Command::Search { words, json } => search_parts(&words, json),
            Command::Test {
                paths,
                verbose,
                json,
            } => run_tests(&paths, verbose, json),
            Command::Skills { command } => print_skill(command),
            Command::Registry {
                command:
                    RegistryCommand::Build {
                        src,
                        out,
                        previous,
                        json,
                    },
            } => build_registry(&src, &out, previous.as_deref(), json),
            Command::Show {
                session,
                no_browser,
                json,
            } => show(session.as_deref(), json, !no_browser),
            Command::Daemon { session, start_log } => daemon(&session, start_log),
        },
        Err(err) => usage(err),
    }
}

/// The flags that take the next word as their value, so that word is never the command name.
const VALUED: [&str; 4] = ["--session", "-C", "--timeout", "--until"];

/// A flag before the command name, as in `orbe --session x open`, is refused with a hint
/// that moves the words before the command name to after it, as typed, less a repeat clap
/// refuses. Only `--help` and `--version` go first.
fn flag_before_command(args: &[OsString]) -> Option<ExitCode> {
    let words: Vec<&str> = args.iter().map(|a| a.to_str()).collect::<Option<_>>()?;
    let first = words.first()?;
    if !first.starts_with('-') || ["-h", "--help", "-V", "--version"].contains(first) {
        return None;
    }
    let commands = cli();
    let mut at = 0;
    while commands.find_subcommand(words.get(at)?).is_none() {
        at += if VALUED.contains(&words[at]) { 2 } else { 1 };
    }
    let mut command = commands.find_subcommand(words[at])?;
    for word in &words[at + 1..] {
        match command.find_subcommand(word) {
            Some(sub) => command = sub,
            None => break,
        }
    }
    let live = orbe_session::spec(words[at]).is_some();
    let takes_once = |flag: &str| {
        !live
            && command.get_arguments().any(|arg| {
                names(arg, flag)
                    && matches!(
                        arg.get_action(),
                        ArgAction::Set | ArgAction::SetTrue | ArgAction::SetFalse
                    )
            })
    };
    let mut line: Vec<&str> = words[at..].to_vec();
    let mut before = words[..at].iter().copied();
    while let Some(flag) = before.next() {
        let unit: Vec<&str> = if VALUED.contains(&flag) {
            [flag].into_iter().chain(before.next()).collect()
        } else {
            vec![flag]
        };
        if !(takes_once(flag) && line.windows(unit.len()).any(|w| w == unit)) {
            line.extend(unit);
        }
    }
    let shown = |words: &[&str]| orbe_scene::shell_join(words);
    let message = format!("`{}` is before the command name", shown(&words[..at]));
    let hint = format!("put it after the command name, as `orbe {}`", shown(&line));
    if words.contains(&"--json") {
        print_json(&json!({
            "ok": false,
            "error": { "code": "usage", "message": message, "hint": hint }
        }));
    } else {
        eprintln!("error[usage]: {message}\n  hint: {hint}");
    }
    Some(ExitCode::from(USAGE))
}

/// Whether `flag`, such as `--json`, `--session=x` or `-Cdir`, is `arg`.
fn names(arg: &Arg, flag: &str) -> bool {
    match flag.strip_prefix("--") {
        Some(long) => arg.get_long() == long.split('=').next(),
        None => flag
            .chars()
            .nth(1)
            .is_some_and(|c| arg.get_short() == Some(c)),
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
fn daemon(session: &str, start_log: Option<u64>) -> ExitCode {
    live::daemon(session, start_log)
}

#[cfg(unix)]
fn show(session: Option<&str>, json: bool, browser: bool) -> ExitCode {
    live::show(session, json, browser)
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
fn daemon(_session: &str, _start_log: Option<u64>) -> ExitCode {
    eprintln!("error[usage]: the session process needs Unix for now");
    ExitCode::from(USAGE)
}

#[cfg(not(unix))]
fn show(_session: Option<&str>, _json: bool, _browser: bool) -> ExitCode {
    eprintln!("error[usage]: `orbe show` needs Unix for now");
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
        let report = test::run(Files::disk(), file, &text, root);
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

fn print_skill(command: SkillsCommand) -> ExitCode {
    match command {
        SkillsCommand::List { json: true } => {
            let skills: Vec<Value> = skills::SKILLS
                .iter()
                .map(|s| json!({ "name": s.name, "about": s.about }))
                .collect();
            print_json(&json!({ "ok": true, "data": { "skills": skills } }));
        }
        SkillsCommand::List { json: false } => {
            for skill in skills::SKILLS {
                println!("{:<8} {}", skill.name, skill.about);
            }
        }
        SkillsCommand::Get { name, json } => {
            let Some(skill) = skills::find(&name) else {
                let message = format!("no skill named `{name}`");
                let hint = format!("the skills are {}", skills::names());
                if json {
                    print_json(&json!({
                        "ok": false,
                        "error": { "code": "usage", "message": message, "hint": hint }
                    }));
                } else {
                    eprintln!("error[usage]: {message}\n  hint: {hint}");
                }
                return ExitCode::from(USAGE);
            };
            let text = match skill.text() {
                Ok(text) => text,
                Err(message) => {
                    if json {
                        print_json(&json!({
                            "ok": false,
                            "error": { "code": "skill", "message": message }
                        }));
                    } else {
                        eprintln!("error[skill]: {message}");
                    }
                    return ExitCode::from(FAILED);
                }
            };
            if json {
                print_json(&json!({ "ok": true, "data": { "name": skill.name, "text": text } }));
            } else {
                print!("{text}");
            }
        }
    }
    ExitCode::SUCCESS
}

fn check(scene: &Path, json: bool) -> ExitCode {
    if let Some(exit) = no_scene(scene, json) {
        return exit;
    }
    let netlist = match orbe_scene::read(&Files::disk(), scene) {
        Ok(netlist) => netlist,
        Err(errors) => return invalid(scene, errors, json),
    };
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

fn lint(scene: &Path, json: bool) -> ExitCode {
    if let Some(exit) = no_scene(scene, json) {
        return exit;
    }
    let warnings = match orbe_scene::lint(&Files::disk(), scene) {
        Ok(warnings) => placed(scene, warnings),
        Err(errors) => return invalid(scene, errors, json),
    };
    if json {
        print_json(&json!({
            "ok": true,
            "data": {
                "scene": scene.display().to_string(),
                "diagnostics": warnings.iter().map(diagnostic_json).collect::<Vec<_>>(),
            },
        }));
    } else if warnings.is_empty() {
        println!("{}: no warnings", scene.display());
    } else {
        for warning in &warnings {
            println!("{warning}");
        }
        println!(
            "{} in {}",
            counted(warnings.len(), "warning"),
            scene.display()
        );
    }
    ExitCode::SUCCESS
}

fn no_scene(scene: &Path, json: bool) -> Option<ExitCode> {
    if scene.is_file() {
        return None;
    }
    let message = format!("no scene file at {}", scene.display());
    if json {
        print_json(&json!({ "ok": false, "error": { "code": "io_error", "message": message } }));
    } else {
        eprintln!("error[io_error]: {message}");
    }
    Some(ExitCode::from(IO))
}

/// Each finding with its file from the current folder rather than the scene's.
fn placed(scene: &Path, found: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let dir = scene.parent().unwrap_or(Path::new(""));
    found
        .into_iter()
        .map(|d| Diagnostic {
            file: dir.join(&d.file).display().to_string(),
            ..d
        })
        .collect()
}

fn counted(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

fn diagnostic_json(d: &Diagnostic) -> Value {
    json!({ "code": d.code.as_str(), "file": d.file, "line": d.line, "column": d.column, "message": d.message, "hint": d.hint })
}

fn invalid(scene: &Path, errors: Vec<Diagnostic>, json: bool) -> ExitCode {
    let errors = placed(scene, errors);
    let count = format!("{} in {}", counted(errors.len(), "error"), scene.display());
    if json {
        let found: Vec<Value> = errors.iter().map(diagnostic_json).collect();
        print_json(
            &json!({ "ok": false, "error": { "code": "scene_invalid", "message": count }, "data": { "diagnostics": found } }),
        );
    } else {
        for error in &errors {
            println!("{error}");
        }
        println!("{count}");
    }
    ExitCode::from(FAILED)
}

fn show_part(id: &str, json: bool) -> ExitCode {
    if let Some(card) = orbe_scene::card(id) {
        return print_part(&card, json);
    }
    let matches = orbe_scene::ids_with_segment(id);
    if matches.len() == 1
        && let Some(card) = orbe_scene::card(&matches[0])
    {
        return print_part(&card, json);
    }
    let names = if matches.len() > 1 {
        matches
    } else {
        orbe_scene::near_names(id)
    };
    unknown_part(id, &names, json)
}

fn print_part(card: &orbe_scene::Card, json: bool) -> ExitCode {
    if json {
        print_json(&json!({ "ok": true, "data": card }));
    } else {
        print!("{}", part::text(card));
    }
    ExitCode::SUCCESS
}

fn unknown_part(id: &str, names: &[String], json: bool) -> ExitCode {
    let message = format!("no catalog part named `{id}`");
    let hint = if names.is_empty() {
        "the catalog has no part with that id".to_string()
    } else {
        let names = names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("did you mean {names}?")
    };
    if json {
        print_json(&json!({
            "ok": false,
            "error": { "code": "unknown_part", "message": message, "hint": hint }
        }));
    } else {
        eprintln!("error[unknown_part]: {message}\n  hint: {hint}");
    }
    ExitCode::from(FAILED)
}

fn search_parts(words: &[String], json: bool) -> ExitCode {
    let found = orbe_scene::search(&words.join(" "));
    if json {
        print_json(&json!({ "ok": true, "data": { "parts": found } }));
    } else {
        print!("{}", part::search_text(&found));
    }
    ExitCode::SUCCESS
}

fn build_registry(src: &Path, out: &Path, previous: Option<&Path>, json: bool) -> ExitCode {
    let built = orbe_registry::build(src, previous).and_then(|registry| {
        registry.write(out)?;
        Ok(registry)
    });
    let registry = match built {
        Ok(registry) => registry,
        Err(error) => {
            let (code, exit, hint) = match &error {
                orbe_registry::Error::Invalid(_) => (
                    "registry_invalid",
                    FAILED,
                    "fix the part or Block it names. docs/registry.md says what a registry holds",
                ),
                orbe_registry::Error::NotEmpty(_) => (
                    "registry_out_not_empty",
                    FAILED,
                    "name a new folder, or empty this one first",
                ),
                orbe_registry::Error::Io(_) => ("io_error", IO, ""),
            };
            let message = error.to_string();
            if json {
                print_json(&json!({
                    "ok": false,
                    "error": { "code": code, "message": message, "hint": hint }
                }));
            } else if hint.is_empty() {
                eprintln!("error[{code}]: {message}");
            } else {
                eprintln!("error[{code}]: {message}\n  hint: {hint}");
            }
            return ExitCode::from(exit);
        }
    };
    let report = &registry.report;
    let warnings = report.warnings();
    if json {
        print_json(&json!({
            "ok": true,
            "data": { "out": out, "report": report, "warnings": warnings }
        }));
    } else {
        println!("wrote {}: {report}", out.display());
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
    }
    ExitCode::SUCCESS
}

#[cfg(unix)]
fn add(
    dir: Option<&Path>,
    ids: Vec<String>,
    overwrite: bool,
    diff: bool,
    relock: bool,
    json: bool,
) -> ExitCode {
    let request = Request {
        ids,
        overwrite,
        diff,
        relock,
    };
    match orbe_scene::add_parts(dir, &request) {
        Ok(reply) => print_reply(reply, json),
        Err(error) => io_exit(&error.to_string(), json),
    }
}

#[cfg(not(unix))]
fn add(
    _dir: Option<&Path>,
    _ids: Vec<String>,
    _overwrite: bool,
    _diff: bool,
    _relock: bool,
    json: bool,
) -> ExitCode {
    let message = "`orbe add` needs Unix for now";
    if json {
        print_json(&json!({
            "ok": false,
            "error": { "code": "usage", "message": message }
        }));
    } else {
        eprintln!("error[usage]: {message}");
    }
    ExitCode::from(USAGE)
}

#[cfg(unix)]
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
