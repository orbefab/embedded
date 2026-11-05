//! Each folder in `tests/bad/` is laid over the `nano-button` fixture, and its `scene.jsonc`
//! must give exactly the findings its `expect` file lists, one `<code> <file>:<line>:<col>`
//! per line, optionally followed by ` | <text>` that the message must contain. An `edit` file
//! changes one project file: its path, the text, and the new text.

mod common;

use std::fs;
use std::path::Path;

const CODES: &[&str] = &[
    "scene.syntax",
    "scene.format",
    "scene.name",
    "scene.duplicate",
    "scene.type",
    "scene.role",
    "scene.unknown_ref",
    "scene.unknown_pin",
    "scene.param",
    "scene.layout",
    "scene.wires",
    "not_simulated",
    "scene.ground",
    "scene.cycle",
    "scene.name_too_long",
    "part_missing",
    "firmware_missing",
    "part_modified",
];

fn overlay(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap();
        if name == "expect" || name == "edit" {
            continue;
        }
        if path.is_dir() {
            common::copy_dir(&path, &to.join(name));
        } else {
            fs::copy(&path, to.join(name)).unwrap();
        }
    }
}

fn run(case: &Path) -> (Vec<(String, String)>, String) {
    let project = common::project("nano-button");
    overlay(case, project.path());
    if let Ok(edit) = fs::read_to_string(case.join("edit")) {
        let lines: Vec<&str> = edit.lines().collect();
        let target = project.path().join(lines[0]);
        let text = fs::read_to_string(&target).unwrap();
        assert!(
            text.contains(lines[1]),
            "{}: `{}` is not in {}",
            case.display(),
            lines[1],
            lines[0]
        );
        fs::write(&target, text.replacen(lines[1], lines[2], 1)).unwrap();
    }
    let diagnostics = orbe_scene::read(&project.path().join("scene.jsonc"))
        .err()
        .unwrap_or_default();
    let found = diagnostics
        .iter()
        .map(|d| {
            (
                format!("{} {}:{}:{}", d.code, d.file, d.line, d.column),
                d.message.clone(),
            )
        })
        .collect();
    let shown = diagnostics.iter().map(|d| format!("{d}\n")).collect();
    (found, shown)
}

#[test]
fn every_bad_fixture_gives_its_findings() {
    let mut covered = Vec::new();
    let mut failures = Vec::new();
    let mut cases: Vec<_> = fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bad"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    for case in cases {
        let expect = fs::read_to_string(case.join("expect")).unwrap();
        let expected: Vec<(&str, Option<&str>)> = expect
            .lines()
            .map(|l| {
                l.split_once(" | ")
                    .map_or((l, None), |(at, text)| (at, Some(text)))
            })
            .collect();
        let (found, shown) = run(&case);
        println!(
            "== {}\n{shown}",
            case.file_name().unwrap().to_string_lossy()
        );
        let matches = found.len() == expected.len()
            && found
                .iter()
                .zip(&expected)
                .all(|((at, message), (want, text))| {
                    at == want && text.is_none_or(|t| message.contains(t))
                });
        if !matches {
            failures.push(format!(
                "{}: expected {expected:?}, found {found:?}",
                case.display()
            ));
        }
        covered.extend(
            found
                .iter()
                .map(|(at, _)| at.split(' ').next().unwrap().to_string()),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for code in CODES {
        assert!(covered.iter().any(|c| c == code), "no fixture gives {code}");
    }
}
