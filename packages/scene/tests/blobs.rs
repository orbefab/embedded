//! A part in format 2 reads a file param `"as": "blob"` as bytes. The fixture `esp32s3-hello`
//! is the engine's `packages/engine/tests/circuits/esp32s3-hello` as a scene: a test chip in
//! format 2 whose scene names the app and the partition table, and whose ROM is the default of
//! its `rom` param. No fixture commits the ROM, so each test copies it from the folder
//! `packages/engine/scripts/blobs.sh` fills. The same fixture shows that the reader takes the
//! esp32s3's required pins from the engine: its GPIOs may go unwired, and `VCC` and `GND` may not.

mod common;

use std::fs;
use std::path::Path;

use orbe_engine::blob::Sha256;
use orbe_scene::{Code, Diagnostic, Files, Netlist};
use serde_json::Value;
use tempfile::TempDir;

const ROM_SHA256: &str = "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd";

/// A copy of the fixture with the ROM at `chip/rom.elf`.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    common::copy_dir(&common::fixture("esp32s3-hello"), dir.path());
    let rom = common::blob_cache().join(ROM_SHA256);
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
fn a_part_in_format_4_needs_a_newer_orbe() {
    let project = project();
    edit(
        project.path(),
        "chip/part.jsonc",
        "\"format\": 2",
        "\"format\": 4",
    );
    let found = errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, Code::PartFormat);
    assert_eq!(found[0].file, "chip/part.jsonc");
    assert_eq!(
        found[0].message,
        "format 4 is newer than this binary reads; this project needs a newer orbe"
    );
    assert_eq!(found[0].hint, "this binary reads format 1, 2, 3");
}

/// A project a newer orbe wrote holds `parts/<id>/` for an id this binary does not carry.
/// Its one finding is `part_format` at the part's `format`, saying the project needs a newer
/// orbe. The part's `"as": "image"` is not a value this binary knows, and it is never read,
/// so it cannot turn into a parse error.
#[test]
fn a_locked_part_this_orbe_does_not_carry_in_a_newer_format_needs_a_newer_orbe() {
    let part_format = orbe_scene::PART_FORMATS.iter().max().unwrap() + 1;
    let scene_format = orbe_scene::SCENE_FORMATS.iter().max().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("parts/newer-chip");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("part.jsonc"),
        format!(
            "{{\n  \"format\": {part_format},\n  \"part\": \"newer-chip\",\n  \"version\": \"0.1.0\",\n  \"pins\": {{ \"a\": [1] }},\n  \"params\": {{ \"app\": {{ \"file\": \"bin\", \"as\": \"image\", \"required\": true }} }}\n}}\n"
        ),
    )
    .unwrap();
    fs::write(
        dir.path().join("scene.jsonc"),
        format!(
            "{{\n  \"format\": {scene_format},\n  \"parts\": {{ \"u1\": {{ \"type\": \"newer-chip\", \"app\": \"app.bin\" }} }}\n}}\n"
        ),
    )
    .unwrap();
    let found = errors(dir.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, Code::PartFormat);
    assert_eq!(found[0].file, "parts/newer-chip/part.jsonc");
    assert_eq!(found[0].line, 2);
    assert_eq!(
        found[0].message,
        format!(
            "format {part_format} is newer than this binary reads; this project needs a newer orbe"
        )
    );
    let reads: Vec<String> = orbe_scene::PART_FORMATS
        .iter()
        .map(u32::to_string)
        .collect();
    assert_eq!(
        found[0].hint,
        format!("this binary reads format {}", reads.join(", "))
    );
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
/// file the scene placed on the flash, the app and the partition table, and blames the image
/// built from both, not either file. `tests/bad/flash_image` shows the same finding.
#[test]
fn an_app_that_breaks_rule_image_is_an_error_at_the_scene_lines_of_both_files() {
    let project = project();
    let app = project.path().join("firmware/hello_world.bin");
    let mut bytes = fs::read(&app).unwrap();
    bytes[100] ^= 0xff;
    fs::write(&app, bytes).unwrap();
    let found = errors(project.path());
    let lines: Vec<usize> = found.iter().map(|d| d.line).collect();
    let line = |text| line_of(project.path(), "scene.jsonc", text);
    assert_eq!(
        lines,
        vec![line("\"partition_table\":"), line("\"app\":")],
        "{found:#?}"
    );
    for finding in &found {
        assert_eq!(finding.code, Code::Param);
        assert_eq!(finding.file, "scene.jsonc");
        assert_eq!(
            finding.message,
            "the firmware image built from `u1.partition_table` (firmware/partition-table.bin) and `u1.app` (firmware/hello_world.bin) failed to read; #u1 firmware has no app the bootloader would start: a partition table, an app partition, blank otadata, and an ESP32-S3 app image whose checksum and hash hold",
        );
        assert_eq!(
            finding.hint,
            "the engine does not say which of these files breaks the image; check each, or build them again together"
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

/// The supply pin `pin` leaves the part and the model's `pins`, so the engine pin sits on a part
/// pin of its own name, which the part lacks. The fixture leaves every GPIO the same way, and
/// the reader takes those: the engine says the first two pins, `VCC` and `GND`, are required.
/// `listed` is the part's pins after the rename, in its order.
fn without_supply_pin(pin: &str, renamed: &str, mapped: (&str, &str), listed: &str) {
    let project = project();
    let chip = "chip/part.jsonc";
    edit(
        project.path(),
        chip,
        &format!("\"{pin}\": ["),
        &format!("\"{renamed}\": ["),
    );
    edit(project.path(), chip, mapped.0, mapped.1);
    edit(
        project.path(),
        "scene.jsonc",
        &format!("\"u1.{pin}\""),
        &format!("\"u1.{renamed}\""),
    );
    let found = errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, Code::UnknownPin, "{}", found[0]);
    assert_eq!(found[0].file, chip);
    assert_eq!(
        found[0].line,
        line_of(project.path(), chip, "\"kind\": \"esp32s3\"")
    );
    assert_eq!(
        found[0].message,
        format!("engine pin `{pin}` sits on `{pin}`, which is not a pin of this part")
    );
    assert_eq!(
        found[0].hint,
        format!("map it in the model's `pins`; the part's pins: {listed}")
    );
}

#[test]
fn an_esp32s3_part_that_maps_no_vcc_is_refused() {
    without_supply_pin(
        "VCC",
        "3V3",
        (
            "\"VCC\": \"VCC\",\n      \"GND\": \"GND\"",
            "\"GND\": \"GND\"",
        ),
        "3V3, GND",
    );
}

#[test]
fn an_esp32s3_part_that_maps_no_gnd_is_refused() {
    without_supply_pin(
        "GND",
        "0V",
        (
            "\"VCC\": \"VCC\",\n      \"GND\": \"GND\"",
            "\"VCC\": \"VCC\"",
        ),
        "VCC, 0V",
    );
}
