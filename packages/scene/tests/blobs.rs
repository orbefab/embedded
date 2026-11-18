//! A part in format 2 reads a file param `"as": "blob"` as bytes. The fixture `esp32s3-hello`
//! is the engine's `packages/engine/tests/circuits/esp32s3-hello` as a scene: a test chip in
//! format 2 whose scene names the app and the partition table, and whose ROM is the default of
//! its `rom` param. No fixture commits the ROM, so each test copies it from the folder
//! `packages/engine/scripts/blobs.sh` fills.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use orbe_engine::blob::Sha256;
use orbe_scene::{Code, Diagnostic, Files, Netlist};
use serde_json::Value;
use tempfile::TempDir;

const ROM_SHA256: &str = "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd";

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

/// A copy of the fixture with the ROM at `chip/rom.elf`.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    common::copy_dir(&common::fixture("esp32s3-hello"), dir.path());
    let rom = blob_cache().join(ROM_SHA256);
    fs::copy(&rom, dir.path().join("chip/rom.elf")).unwrap_or_else(|_| {
        panic!(
            "the ROM blob is not at {}; run packages/engine/scripts/blobs.sh",
            rom.display()
        )
    });
    dir
}

fn read(project: &Path) -> Result<Netlist, Vec<Diagnostic>> {
    orbe_scene::read(&Files::disk(), &project.join("scene.jsonc"))
}

fn errors(project: &Path) -> Vec<Diagnostic> {
    read(project).err().expect("the scene does not check")
}

/// Replaces the one `from` in a project file with `to`.
fn edit(project: &Path, file: &str, from: &str, to: &str) {
    let path = project.join(file);
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches(from).count(), 1, "{file}: {from}");
    fs::write(path, text.replacen(from, to, 1)).unwrap();
}

fn part<'v>(netlist: &'v Value, name: &str) -> &'v Value {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == name)
        .unwrap_or_else(|| panic!("no part {name}"))
}

/// The line of the first `text` in a project file.
fn line_of(project: &Path, file: &str, text: &str) -> usize {
    let source = fs::read_to_string(project.join(file)).unwrap();
    source
        .lines()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("{file} has no {text}"))
        + 1
}

#[test]
fn the_reader_writes_the_engine_examples_rom_and_flash() {
    let project = project();
    let netlist = read(project.path()).unwrap_or_else(|found| panic!("{found:#?}"));
    let read: Value = serde_json::from_str(&netlist.json).unwrap();
    let example: Value = serde_json::from_str(
        &fs::read_to_string(
            common::repo().join("packages/engine/tests/circuits/esp32s3-hello/netlist.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let (u1, expected) = (part(&read, "u1"), part(&example, "u1"));
    assert_eq!(u1["rom"], expected["rom"]);
    assert_eq!(u1["firmware"], expected["firmware"]);
}

#[test]
fn the_reader_keeps_each_blobs_bytes_under_its_hash() {
    let project = project();
    let netlist = read(project.path()).unwrap_or_else(|found| panic!("{found:#?}"));
    for (file, length) in [
        ("chip/rom.elf", 949_552),
        ("firmware/partition-table.bin", 3_072),
        ("firmware/hello_world.bin", 151_664),
    ] {
        let bytes = fs::read(project.path().join(file)).unwrap();
        assert_eq!(bytes.len(), length, "{file}");
        let kept = netlist
            .blobs
            .get(&Sha256::of(&bytes))
            .unwrap_or_else(|| panic!("{file} is not kept"));
        assert_eq!(&kept[..], &bytes[..], "{file}");
    }
}

#[test]
fn a_blob_param_in_part_format_1_needs_format_2() {
    let project = project();
    edit(
        project.path(),
        "chip/part.jsonc",
        "\"format\": 2",
        "\"format\": 1",
    );
    let found = errors(project.path());
    assert_eq!(found.len(), 3, "{found:#?}");
    for (finding, param) in found.iter().zip(["rom", "partition_table", "app"]) {
        assert_eq!(finding.code, Code::Param, "{finding}");
        assert_eq!(finding.file, "chip/part.jsonc", "{finding}");
        assert_eq!(
            finding.line,
            line_of(
                project.path(),
                "chip/part.jsonc",
                &format!("\"{param}\": {{")
            ),
            "{finding}"
        );
        assert_eq!(
            finding.message,
            format!(
                "file param `{param}` takes \"as\": \"blob\" only in part format 2; this file is format 1"
            )
        );
        assert_eq!(finding.hint, "set \"format\": 2 in this part file");
    }
}

#[test]
fn a_part_in_format_3_needs_a_newer_orbe() {
    let project = project();
    edit(
        project.path(),
        "chip/part.jsonc",
        "\"format\": 2",
        "\"format\": 3",
    );
    let found = errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, Code::PartFormat);
    assert_eq!(found[0].file, "chip/part.jsonc");
    assert_eq!(
        found[0].message,
        "format 3 is newer than this binary reads; this project needs a newer orbe"
    );
    assert_eq!(found[0].hint, "this binary reads format 1, 2");
}

#[test]
fn a_scene_in_format_2_needs_a_newer_orbe() {
    let project = project();
    edit(
        project.path(),
        "scene.jsonc",
        "\"format\": 1",
        "\"format\": 2",
    );
    let found = errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, Code::Format);
    assert_eq!(
        found[0].message,
        "format 2 is newer than this binary reads; this project needs a newer orbe"
    );
    assert_eq!(found[0].hint, "this binary reads format 1");
}

/// The engine says only that the flash holds no app it would start, so the finding goes to each
/// file the scene placed on the flash: the app and the partition table.
#[test]
fn an_app_that_breaks_rule_image_is_an_error_at_the_scene_line_of_app() {
    let project = project();
    let app = project.path().join("firmware/hello_world.bin");
    let mut bytes = fs::read(&app).unwrap();
    bytes[100] ^= 0xff;
    fs::write(&app, bytes).unwrap();
    let found = errors(project.path());
    let lines: Vec<(usize, &str)> = found
        .iter()
        .map(|d| (d.line, d.message.split(';').next().unwrap()))
        .collect();
    let line = |text| line_of(project.path(), "scene.jsonc", text);
    assert_eq!(
        lines,
        vec![
            (
                line("\"partition_table\":"),
                "`u1.partition_table` is firmware/partition-table.bin"
            ),
            (line("\"app\":"), "`u1.app` is firmware/hello_world.bin"),
        ],
        "{found:#?}"
    );
    for finding in &found {
        assert_eq!(finding.code, Code::Param);
        assert_eq!(finding.file, "scene.jsonc");
        assert!(
            finding
                .message
                .ends_with("#u1 firmware has no app the bootloader would start: a partition table, an app partition, blank otadata, and an ESP32-S3 app image whose checksum and hash hold"),
            "{finding}"
        );
    }
}

#[test]
fn a_rom_that_breaks_rule_rom_is_an_error_at_the_rom_param() {
    let project = project();
    fs::copy(
        project.path().join("firmware/hello_world.bin"),
        project.path().join("chip/rom.elf"),
    )
    .unwrap();
    let found = errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, Code::Param);
    assert_eq!(found[0].file, "chip/part.jsonc");
    assert_eq!(
        found[0].line,
        line_of(project.path(), "chip/part.jsonc", "\"rom\": {")
    );
    assert!(
        found[0]
            .message
            .starts_with("`u1.rom` is rom.elf; #u1 firmware needs `rom`"),
        "{}",
        found[0]
    );
}

#[test]
fn a_missing_app_is_firmware_missing() {
    let project = project();
    fs::remove_file(project.path().join("firmware/hello_world.bin")).unwrap();
    let found = errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, Code::FirmwareMissing);
    assert_eq!(found[0].file, "scene.jsonc");
    assert_eq!(
        found[0].line,
        line_of(project.path(), "scene.jsonc", "\"app\":")
    );
    assert_eq!(
        found[0].message,
        "`u1.app`: no file at `firmware/hello_world.bin`"
    );
}
