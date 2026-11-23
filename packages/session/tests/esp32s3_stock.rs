//! Stock Arduino-ESP32 sketches, built with the plain ESP32S3 Dev Module fqbn, on the
//! DevKitC-1. Each fixture's app and partition table are the images named in
//! `packages/engine/tests/firmware/esp32s3/arduino/stock/NOTICE`.

use std::fs;
use std::path::{Path, PathBuf};

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

#[path = "../../scene/tests/common/mod.rs"]
mod layout;

/// The fixture plus every catalog part, with each part's own parts nested inside its folder.
fn project(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = repo();
    copy_dir(
        &root.join("packages/scene/tests/fixtures").join(name),
        dir.path(),
    );
    let catalog = root.join("parts");
    let mut ids: Vec<String> = fs::read_dir(&catalog)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| catalog.join(id).join("part.jsonc").is_file())
        .collect();
    ids.sort();
    for id in &ids {
        let dest = dir.path().join("parts").join(id);
        copy_dir(&catalog.join(id), &dest);
        layout::nest_uses(&catalog, &dest, &mut Vec::new());
    }
    dir
}

fn run(live: &mut Live, project: &Path, line: &str) -> Data {
    let command = Command::parse(line, project).unwrap().unwrap();
    let reply = live.run(&command);
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply.data
}

fn play(name: &str, lines: &[&str]) {
    let project = project(name);
    let mut live = Live::new("t", Files::disk());
    let Data::Opened { diagnostics, .. } = run(&mut live, project.path(), "open") else {
        panic!("open opened nothing");
    };
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for line in lines {
        run(&mut live, project.path(), line);
    }
}

#[test]
fn a_stock_blink_lights_the_led_and_prints_ready() {
    play(
        "esp32s3-stock-blink",
        &[
            "step 300ms",
            "expect @led1 lit",
            r#"expect serial contains "ready""#,
            "step 500ms",
            "expect @led1 off",
        ],
    );
}

#[test]
fn a_stock_sketch_prints_ready_and_ticks() {
    play(
        "esp32s3-stock-serial",
        &[
            "step 300ms",
            r#"expect serial contains "ready""#,
            r#"expect serial contains "tick""#,
        ],
    );
}

#[test]
fn a_stock_sketch_reads_the_divider_in_millivolts() {
    play(
        "esp32s3-stock-adc",
        &["step 400ms", r#"expect serial contains "raw 1779 mv 1650""#],
    );
}

#[test]
fn a_stock_sketch_reads_a_button() {
    play(
        "esp32s3-stock-button",
        &[
            "step 300ms",
            r#"expect serial contains "open""#,
            "press @btn1",
            "step 100ms",
            r#"expect serial contains "down""#,
        ],
    );
}

#[test]
fn a_stock_sketch_latches_the_board_rgb_led() {
    play(
        "esp32s3-stock-rgb",
        &[
            "step 300ms",
            "expect @board.led rgb(0,40,0)",
            r#"expect serial contains "ready""#,
        ],
    );
}
