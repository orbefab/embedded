//! Every `orbe add` command a check or an add prints, run from another directory.
//!
//! The project path is `my proj`, under a directory that is not the project, so a command
//! has to carry `-C`. The matrix:
//!
//! - 64: four copy states on each member of a pair (a board and its child, or two named
//!   parts), plain and `--overwrite`.
//! - 96: a board left as the catalog, older, or newer, and its two children in four states
//!   each, plain and `--overwrite`.
//! - 48: one missing part beside two neighbors in four states each. The missing board's
//!   neighbors are the two children it uses.
//! - 15: `orbe add` of a Block, of that Block with a version, and with `--overwrite`, into
//!   four mixed projects and an empty folder.
//! - 4: `--relock` beside a lock line that does not parse.
//! - 2: a catalog copy at format 0, a sealed part and a board. Each check prints an
//!   `orbe add`. A part opened by a `./` path prints none, so it is not here.
//!
//! 64 + 96 + 48 + 15 + 4 + 2 = 229. A printed command names an id, unless it is `--relock`.
//! Running it must not put a part at an older version, and must not print that same
//! command again. A check that prints a command already run is a loop. The project then
//! checks, or it reports only `part_newer` for a lock-matching copy this binary does not carry.
//!
//! A part file that cannot be read prints `chmod u+r` instead of an `orbe add`. That command
//! is run with `sh` and `bash`, from the parent of a project named `it's $HOME` and from
//! inside it. `orbe add` and `orbe add --relock` on an unreadable `part.jsonc`, `asset.json`,
//! `model.py` or `parts.lock` print the read error and the same chmod.
//! A directory `parts/`, `parts/<id>/`, or a subdirectory that cannot be listed prints
//! `chmod u+rx` the same way, and writes nothing. Mode 000 is still readable for root, so
//! those cases are skipped then.

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

#[derive(Clone, Copy)]
enum BlockMode {
    Plain,
    Version,
    Overwrite,
}

type Ver = (u32, u32, u32);

/// The version this binary carries for every catalog part.
const CARRIED: Ver = (0, 1, 0);

const STATES: [State; 4] = [State::Older, State::Newer, State::Edited, State::Other];

const COMBINATIONS: usize = 229;

/// The chmod test changes the process directory. The matrix reads that directory for
/// hint paths, so the two tests do not run at the same time.
fn one_directory() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[test]
fn printed_add_hints_leave_a_check_that_is_clean_or_only_newer() {
    let _directory = one_directory();
    let split = shell_split("orbe add -C '/tmp/my proj' led-5mm --overwrite").unwrap();
    assert_eq!(
        split,
        vec![
            "orbe",
            "add",
            "-C",
            "/tmp/my proj",
            "led-5mm",
            "--overwrite"
        ]
    );
    let quoted = shell_split("orbe add -C '/tmp/it'\\''s' led-5mm").unwrap();
    assert_eq!(quoted, vec!["orbe", "add", "-C", "/tmp/it's", "led-5mm"]);

    let mut tried = 0;
    let mut failures = Vec::new();
    let mut failed = BTreeSet::new();
    let mut fail = |name: String, problems: Vec<String>| {
        tried += 1;
        if !problems.is_empty() {
            failed.insert(name.clone());
        }
        for problem in problems {
            failures.push(format!("{name}: {problem}"));
        }
    };

    for pair in [Pair::BoardChild, Pair::Named] {
        for left in STATES {
            for right in STATES {
                for overwrite in [false, true] {
                    let name = present_label(pair, left, right, overwrite);
                    fail(name, present_case(pair, left, right, overwrite));
                }
            }
        }
    }
    for board in [None, Some(State::Older), Some(State::Newer)] {
        for chip in STATES {
            for led in STATES {
                for overwrite in [false, true] {
                    let name = format!(
                        "two-child board={} chip={} led-0603={} {}",
                        hold_name(board),
                        state_name(chip),
                        state_name(led),
                        mode_name(overwrite)
                    );
                    fail(name, two_child_case(board, chip, led, overwrite));
                }
            }
        }
    }
    for (missing, left_id, right_id) in [
        ("led-5mm", "resistor-axial", "nano-board"),
        ("nano-board", "atmega328p-tqfp32", "led-0603"),
        ("atmega328p-tqfp32", "nano-board", "led-0603"),
    ] {
        for left in STATES {
            for right in STATES {
                let name = format!(
                    "missing {missing} {left_id}={} {right_id}={}",
                    state_name(left),
                    state_name(right)
                );
                fail(name, missing_case(missing, left_id, left, right_id, right));
            }
        }
    }
    for mix in 0..4 {
        for mode in [BlockMode::Plain, BlockMode::Version, BlockMode::Overwrite] {
            let name = format!("block {} {}", mix_name(mix), block_mode_name(mode));
            fail(name, block_case(mix, mode));
        }
    }
    for mode in [BlockMode::Plain, BlockMode::Version, BlockMode::Overwrite] {
        let name = format!("block empty {}", block_mode_name(mode));
        fail(name, block_empty(mode));
    }
    for case in 0..4 {
        let name = format!("relock {}", relock_name(case));
        fail(name, relock_case(case));
    }
    for id in ["led-5mm", "nano-board"] {
        let name = format!("format-0 {id}");
        fail(name, format_case(id));
    }

    assert_eq!(tried, COMBINATIONS, "the matrix above");
    assert!(
        failures.is_empty(),
        "{} of {tried} combinations failed ({} problems):\n{}",
        failed.len(),
        failures.len(),
        failures.join("\n")
    );
}

fn present_case(pair: Pair, left: State, right: State, overwrite: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let (_scratch, outside, project) = placed();
    let (first, second) = pair_ids(pair);
    apply(&project, first, left);
    apply(&project, second, right);
    let before = versions(&project);
    let ids = match pair {
        Pair::BoardChild => vec!["nano-board".to_string()],
        Pair::Named => vec!["led-5mm".to_string(), "resistor-axial".to_string()],
    };
    let newer = newer_ids([(first, left), (second, right)]);
    let text = match run_request(&project, ids, overwrite, false) {
        Ok(text) => text,
        Err(error) => return vec![format!("the add failed: {error}")],
    };
    for down in downgrades(&before, &project) {
        problems.push(format!("the add downgraded {down}"));
    }
    expect_left(&text, &[(first, left), (second, right)], &mut problems);
    drive(
        &outside,
        &project,
        Some(text),
        &before,
        &newer,
        false,
        &mut problems,
    );
    problems
}

fn two_child_case(board: Option<State>, chip: State, led: State, overwrite: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let (_scratch, outside, project) = placed();
    if let Some(state) = board {
        apply(&project, "nano-board", state);
    }
    apply(&project, "atmega328p-tqfp32", chip);
    apply(&project, "led-0603", led);
    let before = versions(&project);
    let mut newer = BTreeSet::new();
    if matches!(board, Some(State::Newer)) {
        newer.insert("nano-board".to_string());
    }
    if matches!(chip, State::Newer) {
        newer.insert("atmega328p-tqfp32".to_string());
    }
    if matches!(led, State::Newer) {
        newer.insert("led-0603".to_string());
    }
    let text = match run_request(&project, vec!["nano-board".to_string()], overwrite, false) {
        Ok(text) => text,
        Err(error) => return vec![format!("the add failed: {error}")],
    };
    for down in downgrades(&before, &project) {
        problems.push(format!("the add downgraded {down}"));
    }
    let mut states = vec![("atmega328p-tqfp32", chip), ("led-0603", led)];
    if let Some(state) = board {
        states.push(("nano-board", state));
    }
    expect_left(&text, &states, &mut problems);
    drive(
        &outside,
        &project,
        Some(text),
        &before,
        &newer,
        false,
        &mut problems,
    );
    problems
}

fn missing_case(
    missing: &str,
    left_id: &str,
    left: State,
    right_id: &str,
    right: State,
) -> Vec<String> {
    let mut problems = Vec::new();
    let (_scratch, outside, project) = placed();
    apply(&project, left_id, left);
    apply(&project, right_id, right);
    fs::remove_dir_all(project.join("parts").join(missing)).unwrap();
    let before = versions(&project);
    let newer = newer_ids([(left_id, left), (right_id, right)]);
    drive(
        &outside,
        &project,
        None,
        &before,
        &newer,
        false,
        &mut problems,
    );
    problems
}

fn block_case(mix: usize, mode: BlockMode) -> Vec<String> {
    let mut problems = Vec::new();
    let (_scratch, outside, project) = placed();
    let newer = apply_mix(&project, mix);
    let before = versions(&project);
    let text = match block_request(&project, mode) {
        Ok(text) => text,
        Err(error) => return vec![format!("the add failed: {error}")],
    };
    for down in downgrades(&before, &project) {
        problems.push(format!("the add downgraded {down}"));
    }
    drive(
        &outside,
        &project,
        Some(text),
        &before,
        &newer,
        false,
        &mut problems,
    );
    problems
}

fn block_empty(mode: BlockMode) -> Vec<String> {
    let mut problems = Vec::new();
    let (_scratch, outside, project) = empty_project();
    let before = versions(&project);
    let text = match block_request(&project, mode) {
        Ok(text) => text,
        Err(error) => return vec![format!("the add failed: {error}")],
    };
    let installs = !matches!(mode, BlockMode::Overwrite);
    drive(
        &outside,
        &project,
        Some(text),
        &before,
        &BTreeSet::new(),
        installs,
        &mut problems,
    );
    problems
}

/// A catalog copy at format 0. Check prints one `orbe add`, and running it restores the copy.
fn format_case(id: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let (_scratch, outside, project) = placed();
    set_format(&project, id);
    let (diagnostics, commands) = scene_hints(&project);
    if !diagnostics.iter().any(|d| d.code == Code::PartFormat) {
        problems.push(format!("check reported no part_format for {id}"));
    }
    if commands.is_empty() {
        problems.push(format!("part_format for {id} printed no orbe add"));
    }
    let before = versions(&project);
    drive(
        &outside,
        &project,
        None,
        &before,
        &BTreeSet::new(),
        false,
        &mut problems,
    );
    problems
}

fn set_format(project: &Path, id: &str) {
    let file = project.join("parts").join(id).join("part.jsonc");
    let text = fs::read_to_string(&file).unwrap();
    let edited = text.replacen("\"format\": 1", "\"format\": 0", 1);
    assert_ne!(text, edited, "{id}");
    fs::write(&file, edited).unwrap();
}

fn relock_case(case: usize) -> Vec<String> {
    let mut problems = Vec::new();
    let (_scratch, outside, project) = placed();
    let newer = match case {
        0 => {
            apply(&project, "led-5mm", State::Newer);
            BTreeSet::from(["led-5mm".to_string()])
        }
        1 => BTreeSet::new(),
        2 => {
            apply(&project, "led-5mm", State::Newer);
            apply(&project, "resistor-axial", State::Newer);
            BTreeSet::from(["led-5mm".to_string(), "resistor-axial".to_string()])
        }
        _ => {
            apply(&project, "nano-board", State::Edited);
            apply(&project, "led-5mm", State::Newer);
            BTreeSet::from(["led-5mm".to_string()])
        }
    };
    spoil_lock(&project);
    let before = versions(&project);
    drive(
        &outside,
        &project,
        None,
        &before,
        &newer,
        false,
        &mut problems,
    );
    problems
}

/// The four mixes a Block is added into. Each leaves the named parts newer.
fn apply_mix(project: &Path, mix: usize) -> BTreeSet<String> {
    match mix {
        0 => {
            apply(project, "led-5mm", State::Newer);
            apply(project, "resistor-axial", State::Older);
            apply(project, "atmega328p-tqfp32", State::Newer);
            apply(project, "tactile-switch-6mm", State::Edited);
            BTreeSet::from(["led-5mm".to_string(), "atmega328p-tqfp32".to_string()])
        }
        1 => {
            apply(project, "led-5mm", State::Older);
            apply(project, "resistor-axial", State::Newer);
            apply(project, "atmega328p-tqfp32", State::Older);
            apply(project, "tactile-switch-6mm", State::Other);
            BTreeSet::from(["resistor-axial".to_string()])
        }
        2 => {
            apply(project, "atmega328p-tqfp32", State::Newer);
            apply(project, "led-0603", State::Older);
            apply(project, "led-5mm", State::Newer);
            apply(project, "resistor-axial", State::Edited);
            BTreeSet::from(["atmega328p-tqfp32".to_string(), "led-5mm".to_string()])
        }
        _ => {
            apply(project, "led-5mm", State::Newer);
            apply(project, "resistor-axial", State::Newer);
            apply(project, "atmega328p-tqfp32", State::Newer);
            apply(project, "tactile-switch-6mm", State::Newer);
            BTreeSet::from([
                "led-5mm".to_string(),
                "resistor-axial".to_string(),
                "atmega328p-tqfp32".to_string(),
                "tactile-switch-6mm".to_string(),
            ])
        }
    }
}

fn block_request(project: &Path, mode: BlockMode) -> Result<String, String> {
    let overwrite = matches!(mode, BlockMode::Overwrite);
    let id = match mode {
        BlockMode::Version => "nano-button-led@0.2.0",
        _ => "nano-button-led",
    };
    run_request(project, vec![id.to_string()], overwrite, false)
}

fn run_request(
    project: &Path,
    ids: Vec<String>,
    overwrite: bool,
    relock: bool,
) -> Result<String, String> {
    let reply = add_parts(
        Some(project),
        &Request {
            ids,
            overwrite,
            diff: false,
            relock,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(reply.text())
}

/// Runs the trigger's printed commands, then every command `orbe check` prints, from
/// `outside`. A check command is not followed into a different command its own reply prints.
fn drive(
    outside: &Path,
    project: &Path,
    trigger: Option<String>,
    before: &BTreeMap<String, Ver>,
    newer: &BTreeSet<String>,
    require_install: bool,
    problems: &mut Vec<String>,
) {
    let mut ran = BTreeSet::new();
    let mut created = Vec::new();
    if let Some(text) = trigger {
        follow(
            &commands_in(&text),
            outside,
            project,
            before,
            &mut ran,
            &mut created,
            problems,
        );
    }
    for _ in 0..6 {
        let (_, commands) = scene_hints(project);
        let fresh: Vec<String> = commands
            .iter()
            .filter(|command| !ran.contains(*command))
            .cloned()
            .collect();
        if fresh.is_empty() {
            if commands.iter().any(|command| ran.contains(command)) {
                problems.push(format!("check repeated `{}`", commands.join("`, `")));
            }
            break;
        }
        for command in fresh {
            ran.insert(command.clone());
            let reply = execute(outside, project, &command, before, &mut created, problems);
            if reply
                .iter()
                .any(|text| commands_in(text).iter().any(|next| next == &command))
            {
                problems.push(format!("`{command}` prints itself again"));
            }
        }
    }
    if outside.join("scene.jsonc").is_file() {
        problems.push("a command wrote the invoking directory".to_string());
    }
    if require_install && !project.join("scene.jsonc").is_file() {
        problems.push("the block is not in the project".to_string());
    }
    for dir in &created {
        if dir == project {
            continue;
        }
        let scene = dir.join("scene.jsonc");
        if !scene.is_file() {
            problems.push(format!(
                "the folder from mkdir has no scene: {}",
                dir.display()
            ));
            continue;
        }
        if let Err(diagnostics) = read(&Files::disk(), &scene) {
            let codes = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            problems.push(format!(
                "the folder from mkdir did not check ({}): {}",
                codes,
                dir.display()
            ));
        }
    }
    if project.join("scene.jsonc").is_file() {
        settle(project, before, newer, problems);
    }
}

fn follow(
    commands: &[String],
    outside: &Path,
    project: &Path,
    before: &BTreeMap<String, Ver>,
    ran: &mut BTreeSet<String>,
    created: &mut Vec<PathBuf>,
    problems: &mut Vec<String>,
) {
    let mut pending: Vec<String> = commands.to_vec();
    let mut steps = 0;
    while let Some(command) = pending.first().cloned() {
        pending.remove(0);
        if !ran.insert(command.clone()) {
            problems.push(format!("hint repeated `{command}`"));
            continue;
        }
        steps += 1;
        if steps > 12 {
            problems.push("more than 12 hint commands".to_string());
            break;
        }
        let Some(text) = execute(outside, project, &command, before, created, problems) else {
            continue;
        };
        for next in commands_in(&text) {
            if next == command {
                problems.push(format!("`{command}` prints itself again"));
                continue;
            }
            pending.push(next);
        }
    }
}

/// Runs one printed command. `None` when it was refused before `orbe add` ran.
fn execute(
    outside: &Path,
    project: &Path,
    command: &str,
    before: &BTreeMap<String, Ver>,
    created: &mut Vec<PathBuf>,
    problems: &mut Vec<String>,
) -> Option<String> {
    let mut made = Vec::new();
    let mut reply = None;
    for piece in command.split(" && ") {
        let words = match shell_split(piece) {
            Ok(words) => words,
            Err(error) => {
                problems.push(error);
                return None;
            }
        };
        if words.first().map(String::as_str) == Some("mkdir") {
            match run_mkdir(outside, &words) {
                Ok(path) => {
                    made.push(path.clone());
                    created.push(path);
                }
                Err(error) => problems.push(format!("`{piece}`: {error}")),
            }
            continue;
        }
        if words.first().map(String::as_str) == Some("chmod") {
            for shell in ["sh", "bash"] {
                if let Err(error) = run_shell(shell, outside, piece) {
                    problems.push(format!("`{piece}`: {error}"));
                }
            }
            continue;
        }
        if words.len() < 2 || words[0] != "orbe" || words[1] != "add" {
            problems.push(format!("not an add command `{piece}`"));
            continue;
        }
        let invocation = parse_add(&words, outside);
        if !invocation.had_c {
            problems.push(format!("command has no -C `{piece}`"));
        } else if invocation.dir != project && !made.iter().any(|dir| dir == &invocation.dir) {
            problems.push(format!("-C is not the project `{piece}`"));
        }
        if invocation.ids.is_empty() && !invocation.relock {
            problems.push(format!("command with no ids `{piece}`"));
            continue;
        }
        if invocation.diff {
            problems.push(format!("hint is not a writing add `{piece}`"));
            continue;
        }
        match run_request(
            &invocation.dir,
            invocation.ids,
            invocation.overwrite,
            invocation.relock,
        ) {
            Ok(text) => {
                for down in downgrades(before, project) {
                    problems.push(format!("`{piece}` downgraded {down}"));
                }
                reply = Some(text);
            }
            Err(error) => problems.push(format!("`{piece}` failed: {error}")),
        }
    }
    reply
}

fn run_mkdir(outside: &Path, words: &[String]) -> Result<PathBuf, String> {
    if words.len() != 2 {
        return Err(format!("mkdir takes one folder, got {}", words.len() - 1));
    }
    let path = outside.join(&words[1]);
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    fs::create_dir(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

struct Invocation {
    dir: PathBuf,
    ids: Vec<String>,
    overwrite: bool,
    diff: bool,
    relock: bool,
    had_c: bool,
}

fn parse_add(words: &[String], outside: &Path) -> Invocation {
    let mut index = 2;
    let mut dir = None;
    if words.get(index).map(String::as_str) == Some("-C") {
        if let Some(path) = words.get(index + 1) {
            let path = PathBuf::from(path);
            dir = Some(if path.is_absolute() {
                path
            } else {
                outside.join(path)
            });
        }
        index += 2;
    }
    let mut ids = Vec::new();
    let mut overwrite = false;
    let mut diff = false;
    let mut relock = false;
    while index < words.len() {
        match words[index].as_str() {
            "--overwrite" => overwrite = true,
            "--diff" => diff = true,
            "--relock" => relock = true,
            other if other.starts_with('-') => {}
            id => ids.push(id.to_string()),
        }
        index += 1;
    }
    let had_c = dir.is_some();
    Invocation {
        dir: dir.unwrap_or_else(|| outside.to_path_buf()),
        ids,
        overwrite,
        diff,
        relock,
        had_c,
    }
}

/// POSIX words. Single quotes and `'\''` round-trip the way `shell_word` writes them.
fn shell_split(command: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = false;
    let chars: Vec<char> = command.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if quote {
            if ch == '\'' {
                quote = false;
            } else {
                word.push(ch);
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' => {
                quote = true;
                started = true;
                index += 1;
            }
            '\\' => {
                let Some(next) = chars.get(index + 1).copied() else {
                    return Err(format!("a trailing backslash in `{command}`"));
                };
                word.push(next);
                started = true;
                index += 2;
            }
            ch if ch.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
                index += 1;
            }
            ch => {
                word.push(ch);
                started = true;
                index += 1;
            }
        }
    }
    if quote {
        return Err(format!("an open quote in `{command}`"));
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

fn commands_in(text: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('`') {
        rest = &rest[at + 1..];
        let Some(end) = rest.find('`') else {
            break;
        };
        let span = rest[..end].to_string();
        rest = &rest[end + 1..];
        let command = span.contains("orbe add") || span.starts_with("chmod ");
        if command && !commands.contains(&span) {
            commands.push(span);
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
            let hints = commands_in(&text);
            (diagnostics, hints)
        }
    }
}

fn settle(
    project: &Path,
    before: &BTreeMap<String, Ver>,
    newer: &BTreeSet<String>,
    problems: &mut Vec<String>,
) {
    for down in downgrades(before, project) {
        problems.push(format!("end state downgraded {down}"));
    }
    let (diagnostics, _) = scene_hints(project);
    let after = versions(project);
    let mut left_newer = BTreeSet::new();
    for id in newer {
        let stayed = before
            .get(id)
            .zip(after.get(id))
            .is_some_and(|(was, now)| *now > CARRIED && now >= was);
        if stayed {
            left_newer.insert(id.clone());
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
}

fn expect_left(text: &str, states: &[(&str, State)], problems: &mut Vec<String>) {
    for (id, state) in states {
        if matches!(state, State::Newer) {
            let needle = format!("left `{id}` at");
            if !text.contains(&needle) {
                problems.push(format!("first reply omitted `{needle}`"));
            }
        }
    }
}

fn newer_id(message: &str) -> Option<String> {
    let rest = message.split("holds `").nth(1)?;
    Some(rest.split('@').next()?.to_string())
}

fn newer_ids<const N: usize>(states: [(&str, State); N]) -> BTreeSet<String> {
    states
        .into_iter()
        .filter(|(_, state)| matches!(state, State::Newer))
        .map(|(id, _)| id.to_string())
        .collect()
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
    let Ok(entries) = fs::read_dir(project.join("parts")) else {
        return out;
    };
    for entry in entries {
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

/// A line that does not parse, ahead of every real line, so a reader that stops there
/// never sees the newer copy's line.
fn spoil_lock(project: &Path) {
    let lock = project.join("parts.lock");
    let text = fs::read_to_string(&lock).unwrap();
    fs::write(lock, format!("this line does not parse\n{text}")).unwrap();
}

fn placed() -> (tempfile::TempDir, PathBuf, PathBuf) {
    placed_named("my proj")
}

fn placed_named(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let scratch = tempfile::tempdir().unwrap();
    let outside = scratch.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let project = outside.join(name);
    let built = common::project("nano-button");
    fs::rename(built.path(), &project).unwrap();
    (scratch, outside, project)
}

/// A project folder whose name holds a space, a quote and `$`. The printed chmod is run
/// from the parent and from inside the project. Each shell starts from mode 000.
#[test]
fn an_unreadable_part_file_chmod_works_where_orbe_ran() {
    if common::mode_000_reads() {
        return;
    }
    let _directory = one_directory();
    let (_scratch, outside, project) = placed_named("it's $HOME");
    let file = project.join("parts/led-5mm/part.jsonc");
    let restore = cwd_guard();
    std::env::set_current_dir(&outside).unwrap();
    hide(&file);
    let parent_chmod = chmod_from_check(Path::new("it's $HOME/scene.jsonc"));
    assert_eq!(
        parent_chmod,
        "chmod u+r 'it'\\''s $HOME/parts/led-5mm/part.jsonc'"
    );
    let relock = relock_text(Some(Path::new("it's $HOME")));
    assert!(
        relock.contains("cannot read parts/led-5mm/part.jsonc"),
        "{relock}"
    );
    assert!(relock.contains("Permission denied"), "{relock}");
    assert!(relock.contains(&format!("`{parent_chmod}`")), "{relock}");
    assert!(!relock.contains("has no version"), "{relock}");
    assert_eq!(mode_bits(&file), 0);
    for shell in ["sh", "bash"] {
        hide(&file);
        run_shell(shell, &outside, &parent_chmod).unwrap();
        assert!(fs::read(&file).is_ok(), "{shell} left the file unreadable");
    }
    read(&Files::disk(), Path::new("it's $HOME/scene.jsonc")).unwrap();

    std::env::set_current_dir(&project).unwrap();
    hide(&file);
    let inside_chmod = chmod_from_check(Path::new("scene.jsonc"));
    assert_eq!(inside_chmod, "chmod u+r parts/led-5mm/part.jsonc");
    let relock = relock_text(None);
    assert!(
        relock.contains("cannot read parts/led-5mm/part.jsonc"),
        "{relock}"
    );
    assert!(relock.contains(&format!("`{inside_chmod}`")), "{relock}");
    assert!(!relock.contains("has no version"), "{relock}");
    for shell in ["sh", "bash"] {
        hide(&file);
        run_shell(shell, &project, &inside_chmod).unwrap();
        assert!(fs::read(&file).is_ok(), "{shell} left the file unreadable");
    }
    read(&Files::disk(), Path::new("scene.jsonc")).unwrap();
    drop(restore);
}

/// Plain `orbe add` and `--relock` name an unreadable `part.jsonc`, `asset.json`, `model.py`
/// or `parts.lock`, with the chmod `orbe check` prints. A missing `part.jsonc` is a different
/// error, and this one does not say the folder has none.
#[test]
fn an_unreadable_part_asset_model_or_lock_names_the_read_and_a_chmod() {
    if common::mode_000_reads() {
        return;
    }
    let _directory = one_directory();
    let (_scratch, outside, project) = placed();
    let restore = cwd_guard();
    std::env::set_current_dir(&project).unwrap();
    for rel in ["part.jsonc", "asset.json", "model.py"] {
        let path = project.join("parts/led-5mm").join(rel);
        hide(&path);
        let text = refused(None, &["led-5mm"], false);
        assert!(
            text.contains(&format!(
                "cannot read parts/led-5mm/{rel}: Permission denied"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!("chmod u+r parts/led-5mm/{rel}")),
            "{text}"
        );
        assert!(!text.contains("no part.jsonc"), "{text}");
        assert!(!text.contains("differs in"), "{text}");
        let relock = refused(None, &[], true);
        assert!(
            relock.contains(&format!(
                "cannot read parts/led-5mm/{rel}: Permission denied"
            )),
            "{relock}"
        );
        assert!(
            relock.contains(&format!("chmod u+r parts/led-5mm/{rel}")),
            "{relock}"
        );
        assert!(!relock.contains("cannot cover"), "{relock}");
        show(&path);
    }

    let lock = project.join("parts.lock");
    hide(&lock);
    let text = refused(None, &["led-5mm"], false);
    assert!(
        text.contains("cannot read parts.lock: Permission denied"),
        "{text}"
    );
    assert!(text.contains("`chmod u+r parts.lock`"), "{text}");
    let relock = refused(None, &[], true);
    assert!(relock.contains("`chmod u+r parts.lock`"), "{relock}");
    show(&lock);

    std::env::set_current_dir(&outside).unwrap();
    let part = project.join("parts/led-5mm/part.jsonc");
    hide(&part);
    let text = refused(Some(Path::new("my proj")), &["led-5mm"], false);
    assert!(
        text.contains("chmod u+r 'my proj/parts/led-5mm/part.jsonc'"),
        "{text}"
    );
    assert!(!text.contains("no part.jsonc"), "{text}");
    show(&part);
    hide(&lock);
    let relock = refused(Some(Path::new("my proj")), &[], true);
    assert!(
        relock.contains("chmod u+r 'my proj/parts.lock'"),
        "{relock}"
    );
    show(&lock);
    drop(restore);
}

/// `parts/`, a part directory, or a subdirectory at mode 000. `--relock` used to replace
/// `parts.lock` with an empty file when `parts/` could not be listed. The refusal prints
/// `chmod u+rx`, `sh` of that hint makes the next command succeed, and the lock bytes
/// stay until then. `orbe check` prints the same chmod.
#[test]
fn an_unreadable_directory_refuses_and_the_chmod_keeps_the_lock() {
    if common::mode_000_reads() {
        return;
    }
    let _directory = one_directory();
    let (_scratch, outside, project) = placed_named("it's $HOME");
    // Declared after the temp folder, so write permission returns before that folder is removed.
    let _open = OpenTree(_scratch.path().to_path_buf());
    let restore = cwd_guard();
    let lock_path = project.join("parts.lock");
    let part_path = project.join("parts/led-5mm/part.jsonc");
    let lock = fs::read(&lock_path).unwrap();
    let part = fs::read(&part_path).unwrap();
    let notes = project.join("parts/led-5mm/notes");
    fs::create_dir(&notes).unwrap();

    std::env::set_current_dir(&project).unwrap();
    refuse_dir(&project.join("parts"), &lock, "chmod u+rx parts");
    show_dir(&project.join("parts"));
    assert_eq!(fs::read(&part_path).unwrap(), part);

    refuse_dir(
        &project.join("parts/led-5mm"),
        &lock,
        "chmod u+rx parts/led-5mm",
    );
    show_dir(&project.join("parts/led-5mm"));
    assert_eq!(fs::read(&part_path).unwrap(), part);
    assert_eq!(fs::read(&lock_path).unwrap(), lock);

    hide(&notes);
    let add = refused(None, &["led-5mm"], false);
    assert!(
        add.contains("cannot read parts/led-5mm/notes: Permission denied"),
        "{add}"
    );
    assert!(add.contains("`chmod u+rx parts/led-5mm/notes`"), "{add}");
    assert!(!add.contains("differs"), "{add}");
    assert_eq!(fs::read(&lock_path).unwrap(), lock);
    let relock = refused(None, &[], true);
    assert!(
        relock.contains("cannot read parts/led-5mm/notes: Permission denied"),
        "{relock}"
    );
    assert!(
        relock.contains("`chmod u+rx parts/led-5mm/notes`"),
        "{relock}"
    );
    assert!(!relock.contains("cannot cover"), "{relock}");
    assert!(!relock.contains("differs"), "{relock}");
    assert_eq!(fs::read(&lock_path).unwrap(), lock);
    assert_eq!(mode_bits(&notes), 0);
    let check = check_text(Path::new("scene.jsonc"));
    assert!(
        check.contains("`chmod u+rx parts/led-5mm/notes`"),
        "{check}"
    );
    run_shell("sh", &project, "chmod u+rx parts/led-5mm/notes").unwrap();
    let reply = add_parts(
        None,
        &Request {
            ids: Vec::new(),
            overwrite: false,
            diff: false,
            relock: true,
        },
    )
    .unwrap();
    assert_eq!(reply.code(), 0, "{}", reply.text());
    assert!(
        reply.text().contains("parts.lock unchanged"),
        "{}",
        reply.text()
    );
    assert_eq!(fs::read(&lock_path).unwrap(), lock);

    std::env::set_current_dir(&outside).unwrap();
    hide(&project.join("parts"));
    let relock = refused(Some(Path::new("it's $HOME")), &[], true);
    let chmod = "chmod u+rx 'it'\\''s $HOME/parts'";
    assert!(
        relock.contains("cannot read parts: Permission denied"),
        "{relock}"
    );
    assert!(relock.contains(&format!("`{chmod}`")), "{relock}");
    assert_eq!(fs::read(&lock_path).unwrap(), lock);
    assert!(!project.join("parts/.orbe-add").exists());
    let check = check_text(Path::new("it's $HOME/scene.jsonc"));
    assert!(check.contains(&format!("`{chmod}`")), "{check}");
    run_shell("sh", &outside, chmod).unwrap();
    assert!(
        !project.join("parts/.orbe-add").exists(),
        "the refusal wrote parts/.orbe-add"
    );
    let reply = add_parts(
        Some(Path::new("it's $HOME")),
        &Request {
            ids: vec!["led-5mm".into()],
            overwrite: false,
            diff: false,
            relock: false,
        },
    )
    .unwrap();
    assert_eq!(reply.code(), 0, "{}", reply.text());
    assert!(
        reply.text().contains("skipped parts/led-5mm"),
        "{}",
        reply.text()
    );
    assert_eq!(fs::read(&lock_path).unwrap(), lock);
    assert_eq!(fs::read(&part_path).unwrap(), part);
    drop(restore);
}

/// Hides `dir`, checks add, relock and check, and runs the printed chmod.
fn refuse_dir(dir: &Path, lock: &[u8], chmod: &str) {
    hide(dir);
    let add = refused(None, &["led-5mm"], false);
    assert!(add.contains("Permission denied"), "{add}");
    assert!(add.contains(&format!("`{chmod}`")), "{add}");
    assert!(!add.contains("no part.jsonc"), "{add}");
    assert!(!add.contains(".orbe-add"), "{add}");
    assert!(!add.contains("--overwrite"), "{add}");
    assert_eq!(mode_bits(dir), 0);
    let relock = refused(None, &[], true);
    assert!(relock.contains(&format!("`{chmod}`")), "{relock}");
    assert!(!relock.contains("wrote parts.lock"), "{relock}");
    let lock_path = std::env::current_dir().unwrap().join("parts.lock");
    assert_eq!(fs::read(&lock_path).unwrap(), lock);
    let check = check_text(Path::new("scene.jsonc"));
    assert!(check.contains(&format!("`{chmod}`")), "{check}");
    run_shell("sh", &std::env::current_dir().unwrap(), chmod).unwrap();
    assert!(fs::read_dir(dir).is_ok(), "{chmod} left {dir:?} unreadable");
    let cwd = std::env::current_dir().unwrap();
    assert!(
        !cwd.join("parts/.orbe-add").exists(),
        "the refusal wrote parts/.orbe-add"
    );
}

fn check_text(scene: &Path) -> String {
    let diagnostics = match read(&Files::disk(), scene) {
        Ok(_) => panic!("the directory is mode 000"),
        Err(diagnostics) => diagnostics,
    };
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn show_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn refused(dir: Option<&Path>, ids: &[&str], relock: bool) -> String {
    let reply = add_parts(
        dir,
        &Request {
            ids: ids.iter().map(|id| (*id).to_string()).collect(),
            overwrite: false,
            diff: false,
            relock,
        },
    )
    .unwrap();
    assert_eq!(reply.code(), 1, "{}", reply.text());
    reply.text()
}

fn show(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
}

fn chmod_from_check(scene: &Path) -> String {
    let diagnostics = match read(&Files::disk(), scene) {
        Ok(_) => panic!("the part file is mode 000"),
        Err(diagnostics) => diagnostics,
    };
    let text = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let commands = commands_in(&text);
    assert_eq!(commands.len(), 1, "{text}");
    commands[0].clone()
}

fn relock_text(dir: Option<&Path>) -> String {
    let reply = add_parts(
        dir,
        &Request {
            ids: Vec::new(),
            overwrite: false,
            diff: false,
            relock: true,
        },
    )
    .unwrap();
    assert_eq!(reply.code(), 1, "{}", reply.text());
    reply.text()
}

fn hide(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
}

fn mode_bits(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Puts every directory under a temp project back to mode 755.
/// A chmod hint leaves `parts/` at mode 0500, and `TempDir` then cannot delete it.
struct OpenTree(PathBuf);

impl Drop for OpenTree {
    fn drop(&mut self) {
        fn open(path: &Path) {
            use std::os::unix::fs::PermissionsExt;
            let Ok(meta) = fs::symlink_metadata(path) else {
                return;
            };
            if !meta.is_dir() {
                return;
            }
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                open(&entry.path());
            }
        }
        open(&self.0);
    }
}

struct CwdGuard(PathBuf);

fn cwd_guard() -> CwdGuard {
    CwdGuard(std::env::current_dir().unwrap())
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

fn run_shell(shell: &str, cwd: &Path, command: &str) -> Result<(), String> {
    let output = std::process::Command::new(shell)
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("{shell}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{shell} exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn empty_project() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let scratch = tempfile::tempdir().unwrap();
    let outside = scratch.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let project = outside.join("my proj");
    fs::create_dir(&project).unwrap();
    (scratch, outside, project)
}

fn pair_ids(pair: Pair) -> (&'static str, &'static str) {
    match pair {
        Pair::BoardChild => ("nano-board", "atmega328p-tqfp32"),
        Pair::Named => ("led-5mm", "resistor-axial"),
    }
}

fn present_label(pair: Pair, left: State, right: State, overwrite: bool) -> String {
    let (first, second) = pair_ids(pair);
    let kind = match pair {
        Pair::BoardChild => "board+child",
        Pair::Named => "two-named",
    };
    format!(
        "present {kind} {first}={} {second}={} {}",
        state_name(left),
        state_name(right),
        mode_name(overwrite)
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

fn hold_name(state: Option<State>) -> &'static str {
    match state {
        None => "catalog",
        Some(state) => state_name(state),
    }
}

fn mode_name(overwrite: bool) -> &'static str {
    if overwrite { "--overwrite" } else { "plain" }
}

fn block_mode_name(mode: BlockMode) -> &'static str {
    match mode {
        BlockMode::Plain => "plain",
        BlockMode::Version => "@version",
        BlockMode::Overwrite => "--overwrite",
    }
}

fn mix_name(mix: usize) -> &'static str {
    match mix {
        0 => "led-newer-resistor-older",
        1 => "led-older-resistor-newer",
        2 => "children-split",
        _ => "four-newer",
    }
}

fn relock_name(case: usize) -> &'static str {
    match case {
        0 => "newer-led",
        1 => "catalog",
        2 => "two-newer",
        _ => "edited-board-newer-led",
    }
}
