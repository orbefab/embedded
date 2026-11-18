//! A session of the DevKitC-1 catalog board with the esp32s3-hello app. It opens, plays,
//! and the console starts with that example's first line.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use orbe_scene::{REGISTRY, folder_hash};
use orbe_session::{Command, Data, Files, Live};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn masked(line: &str) -> String {
    let Some((level, rest)) = line.split_once(" (") else {
        return line.to_string();
    };
    match rest.split_once(") ") {
        Some((time, text)) if time.bytes().all(|b| b.is_ascii_digit()) => {
            format!("{level} (T) {text}")
        }
        _ => line.to_string(),
    }
}

fn run(live: &mut Live, project: &Path, line: &str) -> Data {
    let command = Command::parse(line, project).unwrap().unwrap();
    let reply = live.run(&command);
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply.data
}

/// The fixture plus every catalog part and a lock line for each.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = repo();
    copy_dir(
        &root.join("packages/scene/tests/fixtures/esp32s3-devkitc"),
        dir.path(),
    );
    let catalog = root.join("parts");
    let mut ids: Vec<String> = fs::read_dir(&catalog)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| catalog.join(id).join("part.jsonc").is_file())
        .collect();
    ids.sort();
    let mut lock = String::new();
    for id in &ids {
        let folder = catalog.join(id);
        copy_dir(&folder, &dir.path().join("parts").join(id));
        let text = fs::read_to_string(folder.join("part.jsonc")).unwrap();
        let version = text
            .split("\"version\": \"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        lock.push_str(&format!(
            "{id} {version} {REGISTRY} {}\n",
            folder_hash(&folder).unwrap()
        ));
    }
    fs::write(dir.path().join("parts.lock"), lock).unwrap();
    dir
}

#[test]
fn the_devkit_plays_and_its_console_starts_with_the_hello_line() {
    let project = project();
    let console: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            repo().join("packages/engine/tests/circuits/esp32s3-hello/console.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let first = console["transcript"]
        .as_str()
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();

    let mut live = Live::new("t", Files::disk());
    let Data::Opened { diagnostics, .. } = run(&mut live, project.path(), "open") else {
        panic!("open opened nothing");
    };
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    run(&mut live, project.path(), "play");
    let mut serial = String::new();
    for _ in 0..2_000 {
        assert!(live.tick(Duration::from_millis(1)).is_some(), "play ended");
        let Data::Snapshot(snapshot) = run(&mut live, project.path(), "snapshot") else {
            panic!("snapshot gave no snapshot");
        };
        serial = snapshot.serial;
        if serial.lines().count() > 1 {
            break;
        }
    }
    let got = serial.lines().next().unwrap_or_default();
    assert_eq!(masked(got), first, "the console so far: {serial:?}");
}
