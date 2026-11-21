//! A session opens a scene whose part reads its firmware as blobs: the scene reader's fixture
//! `esp32s3-hello`, the engine's `packages/engine/tests/circuits/esp32s3-hello` as a scene. It
//! plays, and its console starts as the board's did. No fixture commits the ROM, so the test
//! copies it from the folder `packages/engine/scripts/blobs.sh` fills.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use orbe_session::{Command, Data, Files, Live};

const ROM_SHA256: &str = "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The folder `packages/engine/scripts/blobs.sh` fills: `$ORBE_BLOBS`, else
/// `$XDG_CACHE_HOME/orbe/blobs`, else `~/.cache/orbe/blobs`.
fn blob_cache() -> PathBuf {
    let set = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    set("ORBE_BLOBS").map(PathBuf::from).unwrap_or_else(|| {
        set("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".cache")
            })
            .join("orbe/blobs")
    })
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

/// A log line with its time masked, as `packages/engine/scripts/esp32s3_hello.py` masks it:
/// `I (123) ` reads `I (T) `.
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

#[test]
fn the_hello_app_plays_and_its_console_starts_as_the_boards() {
    let project = tempfile::tempdir().unwrap();
    copy_dir(
        &repo().join("packages/scene/tests/fixtures/esp32s3-hello"),
        project.path(),
    );
    let rom = blob_cache().join(ROM_SHA256);
    fs::copy(&rom, project.path().join("chip/rom.elf")).unwrap_or_else(|_| {
        panic!(
            "the ROM blob is not at {}; run packages/engine/scripts/blobs.sh",
            rom.display()
        )
    });
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
    // A millisecond a tick, so the first line is still among the lines a snapshot shows when
    // the next line has started.
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
