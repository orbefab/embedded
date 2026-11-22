//! Each folder in `tests/bad/` is laid over the `nano-button` fixture, and its `scene.jsonc`
//! must give exactly the findings its `expect` file lists, one `<code> <file>:<line>:<col>`
//! per line. ` | <message>` and a further ` | <hint>` are substrings the finding must contain.
//! Every finding has a hint. An `edit` file changes one project file: its path, the text, and
//! the new text. A `base` file names another fixture to lay the case over, and a `blobs` file
//! copies blobs no fixture commits, as `common::overlaid` says. `orbe check` prints the same
//! findings and exits 1, in the CLI's own tests.

mod common;

use std::fs;
use std::path::Path;

use orbe_scene::Files;

const CODES: &[&str] = &[
    "scene.syntax",
    "scene.format",
    "part_format",
    "scene.name",
    "scene.duplicate",
    "scene.type",
    "scene.role",
    "scene.unknown_ref",
    "scene.unknown_pin",
    "scene.param",
    "scene.world",
    "scene.layout",
    "scene.wires",
    "not_simulated",
    "scene.ground",
    "scene.cycle",
    "scene.name_too_long",
    "part_missing",
    "firmware_missing",
];

fn run(case: &Path) -> (Vec<(String, String, String)>, String) {
    let project = common::overlaid(case);
    let diagnostics = orbe_scene::read(&Files::disk(), &project.path().join("scene.jsonc"))
        .err()
        .unwrap_or_default();
    let found = diagnostics
        .iter()
        .map(|d| {
            (
                format!("{} {}:{}:{}", d.code, d.file, d.line, d.column),
                d.message.clone(),
                d.hint.clone(),
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
    for case in common::cases("bad") {
        let expect = fs::read_to_string(case.join("expect")).unwrap();
        let expected: Vec<(&str, Option<&str>, Option<&str>)> = expect
            .lines()
            .map(|l| {
                let mut parts = l.splitn(3, " | ");
                (parts.next().unwrap_or(""), parts.next(), parts.next())
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
                .all(|((at, message, hint), (want, text, want_hint))| {
                    at == want
                        && text.is_none_or(|t| message.contains(t))
                        && want_hint.is_none_or(|t| hint.contains(t))
                        && !hint.is_empty()
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
                .map(|(at, _, _)| at.split(' ').next().unwrap().to_string()),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for code in CODES {
        assert!(covered.iter().any(|c| c == code), "no fixture gives {code}");
    }
}
