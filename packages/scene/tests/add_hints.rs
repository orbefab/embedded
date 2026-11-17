//! Every `orbe add` command a check or an add prints, followed as printed.
//!
//! The matrix is each of four copy states on each member of a pair, for a board and the
//! child it uses and for two named parts, with a plain add and with `--overwrite`.
//! 4 × 4 × 2 × 2 = 64. A printed command has to name an id. Running it must not put a part
//! at an older version, and must not print that same command again. The project then
//! checks, or it reports only `part_newer` for a lock-matching copy this binary does not carry.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use orbe_scene::{Code, Diagnostic, Files, Request, add_parts, read};

#[derive(Clone, Copy)]
enum State {
    Older,
    Newer,
    Edited,
    Other,
}

#[derive(Clone, Copy)]
enum Pair {
    BoardChild,
    Named,
}

type Ver = (u32, u32, u32);

/// The version this binary carries for every catalog part.
const CARRIED: Ver = (0, 1, 0);

const STATES: [State; 4] = [State::Older, State::Newer, State::Edited, State::Other];

#[test]
fn printed_add_hints_leave_a_check_that_is_clean_or_only_newer() {
    let mut tried = 0;
    let mut failures = Vec::new();
    for pair in [Pair::BoardChild, Pair::Named] {
        for left in STATES {
            for right in STATES {
                for overwrite in [false, true] {
                    tried += 1;
                    let name = label(pair, left, right, overwrite);
                    for problem in exercise(pair, left, right, overwrite) {
                        failures.push(format!("{name}: {problem}"));
                    }
                }
            }
        }
    }
    assert_eq!(tried, 64, "4 states × 4 states × 2 pairs × 2 modes");
    assert!(
        failures.is_empty(),
        "{} of {tried} combinations failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn exercise(pair: Pair, left: State, right: State, overwrite: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let project = common::project("nano-button");
    let root = project.path();
    let (first, second) = pair_ids(pair);
    apply(root, first, left);
    apply(root, second, right);
    let before = versions(root);
    let ids = match pair {
        Pair::BoardChild => vec!["nano-board".to_string()],
        Pair::Named => vec!["led-5mm".to_string(), "resistor-axial".to_string()],
    };
    let first_text = match run_add(Some(root), ids, overwrite) {
        Ok(text) => text,
        Err(error) => return vec![format!("the add failed: {error}")],
    };
    for down in downgrades(&before, root) {
        problems.push(format!("the add downgraded {down}"));
    }
    for (id, state) in [(first, left), (second, right)] {
        if matches!(state, State::Newer) {
            let needle = format!("left `{id}` at");
            if !first_text.contains(&needle) {
                problems.push(format!("first reply omitted `{needle}`"));
            }
        }
    }
    follow(&hint_commands(&first_text), &before, root, &mut problems);
    let (_, from_check) = scene_hints(root);
    follow(&from_check, &before, root, &mut problems);
    for down in downgrades(&before, root) {
        problems.push(format!("end state downgraded {down}"));
    }
    let (diagnostics, _) = scene_hints(root);
    let after = versions(root);
    let mut left_newer = BTreeSet::new();
    for (id, state) in [(first, left), (second, right)] {
        if !matches!(state, State::Newer) {
            continue;
        }
        let stayed = before
            .get(id)
            .zip(after.get(id))
            .is_some_and(|(was, now)| now > &CARRIED && now >= was);
        if stayed {
            left_newer.insert(id.to_string());
        }
    }
    let mut reported = BTreeSet::new();
    for diagnostic in &diagnostics {
        if diagnostic.code != Code::PartNewer {
            problems.push(format!(
                "check reported {} `{}`",
                diagnostic.code, diagnostic.message
            ));
            continue;
        }
        match newer_id(&diagnostic.message) {
            Some(id) => {
                reported.insert(id);
            }
            None => problems.push(format!(
                "part_newer message has no id `{}`",
                diagnostic.message
            )),
        }
    }
    if reported != left_newer {
        problems.push(format!(
            "part_newer ids {reported:?}, copies left newer {left_newer:?}"
        ));
    }
    problems
}

/// Runs each printed command. A command with no id is a failure and is not run. A command
/// that shows up again, including in its own reply, is a loop.
fn follow(
    commands: &[String],
    before: &BTreeMap<String, Ver>,
    project: &Path,
    problems: &mut Vec<String>,
) {
    let mut pending: Vec<String> = commands.to_vec();
    let mut ran = BTreeSet::new();
    let mut steps = 0;
    while let Some(command) = pending.first().cloned() {
        pending.remove(0);
        if !ran.insert(command.clone()) {
            problems.push(format!("hint repeated `{command}`"));
            continue;
        }
        steps += 1;
        if steps > 8 {
            problems.push("more than 8 hint commands".to_string());
            break;
        }
        let invocation = match parse_add(&command) {
            Ok(invocation) => invocation,
            Err(error) => {
                problems.push(error);
                continue;
            }
        };
        if invocation.ids.is_empty() {
            problems.push(format!("command with no ids `{command}`"));
            continue;
        }
        let dir = invocation.dir.unwrap_or_else(|| project.to_path_buf());
        let text = match run_add(Some(&dir), invocation.ids, invocation.overwrite) {
            Ok(text) => text,
            Err(error) => {
                problems.push(format!("`{command}` failed: {error}"));
                continue;
            }
        };
        for down in downgrades(before, project) {
            problems.push(format!("`{command}` downgraded {down}"));
        }
        for next in hint_commands(&text) {
            if next == command {
                problems.push(format!("`{command}` prints itself again"));
                continue;
            }
            pending.push(next);
        }
    }
}

fn run_add(dir: Option<&Path>, ids: Vec<String>, overwrite: bool) -> Result<String, String> {
    let reply = add_parts(
        dir,
        &Request {
            ids,
            overwrite,
            diff: false,
            relock: false,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(reply.text())
}

struct Invocation {
    dir: Option<PathBuf>,
    ids: Vec<String>,
    overwrite: bool,
}

fn parse_add(command: &str) -> Result<Invocation, String> {
    if command.contains(['\'', '"']) {
        return Err(format!("quoted command `{command}`"));
    }
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.len() < 2 || words[0] != "orbe" || words[1] != "add" {
        return Err(format!("not an add command `{command}`"));
    }
    let mut index = 2;
    let mut dir = None;
    if words.get(index) == Some(&"-C") {
        let path = words
            .get(index + 1)
            .ok_or_else(|| format!("-C without a folder in `{command}`"))?;
        dir = Some(PathBuf::from(path));
        index += 2;
    }
    let mut ids = Vec::new();
    let mut overwrite = false;
    while index < words.len() {
        match words[index] {
            "--overwrite" => overwrite = true,
            "--diff" | "--relock" => {
                return Err(format!("hint is not a writing add `{command}`"));
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown flag in `{command}`"));
            }
            id => ids.push(id.to_string()),
        }
        index += 1;
    }
    Ok(Invocation {
        dir,
        ids,
        overwrite,
    })
}

/// Commands inside `hint:` lines. The usage example sits in the message, not on a hint line.
fn hint_commands(text: &str) -> Vec<String> {
    let mut commands = Vec::new();
    for line in text.lines() {
        if !line.contains("hint:") {
            continue;
        }
        let mut rest = line;
        while let Some(at) = rest.find("`orbe add") {
            rest = &rest[at + 1..];
            let end = rest.find('`').unwrap_or(rest.len());
            let command = rest[..end].to_string();
            rest = &rest[end..];
            if !commands.contains(&command) {
                commands.push(command);
            }
        }
    }
    commands
}

fn scene_hints(project: &Path) -> (Vec<Diagnostic>, Vec<String>) {
    match read(&Files::disk(), &project.join("scene.jsonc")) {
        Ok(_) => (Vec::new(), Vec::new()),
        Err(diagnostics) => {
            let text = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let hints = hint_commands(&text);
            (diagnostics, hints)
        }
    }
}

fn newer_id(message: &str) -> Option<String> {
    let rest = message.split("holds `").nth(1)?;
    Some(rest.split('@').next()?.to_string())
}

fn downgrades(before: &BTreeMap<String, Ver>, project: &Path) -> Vec<String> {
    let after = versions(project);
    let mut found = Vec::new();
    for (id, was) in before {
        match after.get(id) {
            None => found.push(format!("`{id}` is gone")),
            Some(now) if now < was => {
                found.push(format!("`{id}` {} -> {}", show_ver(*was), show_ver(*now)));
            }
            Some(_) => {}
        }
    }
    found
}

fn versions(project: &Path) -> BTreeMap<String, Ver> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(project.join("parts")).unwrap() {
        let path = entry.unwrap().path();
        let id = path.file_name().unwrap().to_string_lossy().into_owned();
        if id.starts_with('.') || !path.join("part.jsonc").is_file() {
            continue;
        }
        let text = fs::read_to_string(path.join("part.jsonc")).unwrap();
        let version = text
            .split("\"version\": \"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or_else(|| panic!("no version in {id}"));
        out.insert(id, parse_ver(version));
    }
    out
}

fn parse_ver(text: &str) -> Ver {
    let mut parts = text.split('.');
    let major = parts.next().unwrap().parse().unwrap();
    let minor = parts.next().unwrap().parse().unwrap();
    let patch = parts.next().unwrap().parse().unwrap();
    assert!(parts.next().is_none(), "{text}");
    (major, minor, patch)
}

fn show_ver(ver: Ver) -> String {
    format!("{}.{}.{}", ver.0, ver.1, ver.2)
}

fn apply(project: &Path, id: &str, state: State) {
    match state {
        State::Older => hold(project, id, "0.0.9"),
        State::Newer => hold(project, id, "0.2.0"),
        State::Edited => edit_title(project, id),
        State::Other => set_registry(project, id, "@acme"),
    }
}

fn hold(project: &Path, id: &str, version: &str) {
    let file = project.join("parts").join(id).join("part.jsonc");
    let text = fs::read_to_string(&file).unwrap();
    let edited = text.replacen(
        "\"version\": \"0.1.0\"",
        &format!("\"version\": \"{version}\""),
        1,
    );
    assert_ne!(text, edited, "{id}");
    fs::write(&file, edited).unwrap();
    rewrite_lock_line(project, id, &common::lock_line(project, id));
}

/// A title change at `0.1.0` whose lock line still names the catalog hash.
fn edit_title(project: &Path, id: &str) {
    let path = project.join("parts").join(id).join("part.jsonc");
    let text = fs::read_to_string(&path).unwrap();
    let marker = "\"title\": \"";
    let start = text.find(marker).unwrap_or_else(|| panic!("{id} title")) + marker.len();
    let edited = format!("{}edited {}", &text[..start], &text[start..]);
    assert_ne!(text, edited, "{id}");
    fs::write(path, edited).unwrap();
}

fn set_registry(project: &Path, id: &str, registry: &str) {
    let lock = project.join("parts.lock");
    let text = fs::read_to_string(&lock).unwrap();
    let mut found = false;
    let mut out = String::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.first() == Some(&id) {
            assert_eq!(fields.len(), 4, "{line}");
            out.push_str(&format!(
                "{} {} {registry} {}\n",
                fields[0], fields[1], fields[3]
            ));
            found = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    assert!(found, "{id} has no lock line");
    fs::write(lock, out).unwrap();
}

fn rewrite_lock_line(project: &Path, id: &str, new_line: &str) {
    let lock = project.join("parts.lock");
    let text = fs::read_to_string(&lock).unwrap();
    let mut found = false;
    let mut out = String::new();
    for line in text.lines() {
        if line.split_whitespace().next() == Some(id) {
            out.push_str(new_line);
            found = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    assert!(found, "{id} has no lock line");
    fs::write(lock, out).unwrap();
}

fn pair_ids(pair: Pair) -> (&'static str, &'static str) {
    match pair {
        Pair::BoardChild => ("nano-board", "atmega328p-tqfp32"),
        Pair::Named => ("led-5mm", "resistor-axial"),
    }
}

fn label(pair: Pair, left: State, right: State, overwrite: bool) -> String {
    let (first, second) = pair_ids(pair);
    let kind = match pair {
        Pair::BoardChild => "board+child",
        Pair::Named => "two-named",
    };
    let mode = if overwrite { "--overwrite" } else { "plain" };
    format!(
        "{kind} {first}={} {second}={} {mode}",
        state_name(left),
        state_name(right)
    )
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Older => "older",
        State::Newer => "newer",
        State::Edited => "edited",
        State::Other => "other-registry",
    }
}
