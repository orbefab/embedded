//! Helpers shared by the skill tests and the command tests.

#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use super::harness::{Run, Runtime, wait_until};

pub fn skill(name: &str) -> String {
    let runtime = Runtime::new();
    runtime
        .orbe(Path::new("."), &["skills", "get", name])
        .ok()
        .stdout
}

/// Each fenced block with its info string, such as `jsonc scene.jsonc`.
pub fn blocks(text: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let mut open: Option<(String, String)> = None;
    for line in text.lines() {
        match (line.strip_prefix("```"), open.take()) {
            (Some(info), None) => open = Some((info.to_string(), String::new())),
            (Some(_), Some(block)) => blocks.push(block),
            (None, Some((info, mut body))) => {
                body.push_str(line);
                body.push('\n');
                open = Some((info, body));
            }
            (None, None) => {}
        }
    }
    blocks
}

/// Whether `actual` is `shown`, where a shown line `...` stands for any lines.
pub fn matches(shown: &[&str], actual: &[&str]) -> bool {
    match shown.split_first() {
        None => actual.is_empty(),
        Some((&"...", rest)) => (0..=actual.len()).any(|skip| matches(rest, &actual[skip..])),
        Some((line, rest)) => actual
            .split_first()
            .is_some_and(|(first, more)| first == line && matches(rest, more)),
    }
}

/// Runs a shell line with this build's `orbe` first on `PATH`.
pub fn shell(runtime: &Runtime, dir: &Path, line: &str) -> Run {
    let bin = Path::new(env!("CARGO_BIN_EXE_orbe")).parent().unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let output = Command::new("sh")
        .args(["-c", line])
        .current_dir(dir)
        .env("PATH", path)
        .env("XDG_RUNTIME_DIR", runtime.dir.path())
        .env_remove("ORBE_SESSION")
        .env_remove("ORBE_TEST_IDLE_TIMEOUT")
        .env_remove("ORBE_TEST_STOP")
        .env("ORBE_REGISTRY", super::harness::registry_template())
        .output()
        .unwrap();
    Run::new(output)
}

/// One JSON envelope on stdout, and nothing on stderr.
pub fn one_envelope(run: &Run, what: &str) -> Value {
    assert_eq!(
        run.stdout.lines().count(),
        1,
        "{what} prints one line, got {}",
        run.stdout
    );
    assert!(run.stderr.is_empty(), "{what}: {}", run.stderr);
    serde_json::from_str(run.stdout.trim())
        .unwrap_or_else(|error| panic!("{what}: {error}: {}", run.stdout))
}

/// Applies a `diff` block: each `-` line of the file becomes the `+` line after it.
pub fn apply_diff(dir: &Path, file: &str, body: &str) {
    let path = dir.join(file);
    let mut text = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    for pair in lines.chunks(2) {
        let old = pair[0].strip_prefix('-');
        let new = pair.get(1).and_then(|l| l.strip_prefix('+'));
        let (Some(old), Some(new)) = (old, new) else {
            panic!("a diff block pairs each `-` line with a `+` line: {pair:?}");
        };
        assert_eq!(text.matches(old).count(), 1, "{file} has one `{old}`");
        text = text.replace(old, new);
    }
    fs::write(path, text).unwrap();
}

pub fn write_file(dir: &Path, file: &str, body: &str) {
    fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
    fs::write(dir.join(file), body).unwrap();
}

/// Runs each `$ ` line of a `console` block and compares what it prints with the lines shown.
pub fn run_shown(runtime: &Runtime, dir: &Path, transcript: &str) {
    let mut steps: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in transcript.lines() {
        match line.strip_prefix("$ ") {
            Some(command) => steps.push((command, Vec::new())),
            None => steps.last_mut().expect("a command first").1.push(line),
        }
    }
    for (command, shown) in &steps {
        assert!(command.starts_with("orbe "), "{command}");
        let run = shell(runtime, dir, command);
        let printed = format!("{}{}", run.stdout, run.stderr);
        let actual: Vec<&str> = printed.lines().collect();
        assert!(
            matches(shown, &actual),
            "`{command}` (exit {}) printed\n{printed}\nbut core shows\n{}",
            run.code,
            shown.join("\n")
        );
    }
}

/// Copies catalog parts into the project at `dir`, nested the way `orbe add` writes them.
pub fn add_parts(dir: &Path, ids: &[&str]) {
    let catalog = super::common::repo().join("parts");
    for id in ids {
        let dest = dir.join("parts").join(id);
        super::common::copy_dir(&catalog.join(id), &dest);
        super::common::nest_uses(&catalog, &dest, &mut Vec::new());
    }
}

/// The core skill's worked example, with its parts added.
pub fn worked_example() -> tempfile::TempDir {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    add_parts(
        dir,
        &[
            "led-5mm",
            "nano-board",
            "resistor-axial",
            "tactile-switch-6mm",
        ],
    );
    let (_, scene) = blocks(&skill("core"))
        .into_iter()
        .find(|(info, _)| info == "jsonc scene.jsonc")
        .unwrap();
    fs::write(dir.join("scene.jsonc"), scene).unwrap();
    fs::create_dir_all(dir.join("build")).unwrap();
    fs::copy(super::common::firmware(), dir.join("build/button.ino.hex")).unwrap();
    fs::create_dir(dir.join("tests")).unwrap();
    project
}

/// Every block of a skill, in order, in `dir`: each file block is written and each `console`
/// block runs as shown. Returns the `console` blocks.
pub fn run_blocks(text: &str, dir: &Path) -> Vec<String> {
    let runtime = Runtime::new();
    let mut sessions = Vec::new();
    for (info, body) in blocks(text) {
        match info.split_once(' ') {
            Some(("diff", file)) => apply_diff(dir, file, &body),
            Some((_, file)) => write_file(dir, file, &body),
            None if info == "console" => {
                run_shown(&runtime, dir, &body);
                sessions.push(body);
            }
            None => {}
        }
    }
    wait_until("each example closes its session", || {
        runtime.daemons().is_empty()
    });
    sessions
}
