//! Blob params in a registry build. A blob param with a default is probed at that default. A
//! required one with no default stays unset, and the build holds back only the bodies it
//! reaches. No file stands in for a value only a scene gives.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{MANIFEST, put, record, repo};
use orbe_registry::{Error, build};

/// An ELF the scene crate's tests run as an `esp32s3` program. It is a test program, not a
/// stand-in: the part below names it as its own default.
fn test_elf() -> PathBuf {
    repo().join("packages/scene/tests/fixtures/catalog-defaults/catalog/blob-chip/image.elf")
}

/// The mask ROM the catalog chip commits as the default of its `rom` param.
fn rom_elf() -> PathBuf {
    repo().join("parts/esp32s3-qfn56/esp32s3_rev0_rom.elf")
}

/// The fields of an `esp32s3` model besides its firmware.
const ESP32S3_FIELDS: &str = r#""output": {
        "high_drop": { "value": 0.0, "unit": "V", "source": "user" },
        "high_resistance": { "value": 16.5, "unit": "ohm", "source": "user" },
        "low_drop": { "value": 0.0, "unit": "V", "source": "user" },
        "low_resistance": { "value": 17.68, "unit": "ohm", "source": "user" }
      },
      "pull_up_resistance": { "value": 45000.0, "unit": "ohm", "source": "user" },
      "leakage_resistance": { "value": 66000000.0, "unit": "ohm", "source": "user" },
      "input_threshold": {
        "low": { "value": 0.25, "unit": "1", "source": "user" },
        "high": { "value": 0.75, "unit": "1", "source": "user" }
      }"#;

/// A chip whose program is a blob param with a default, the file beside it.
fn elf_chip() -> String {
    format!(
        r#"{{
  "format": 2,
  "part": "elf-chip",
  "version": "0.1.0",
  "title": "ELF chip",
  "license": "MIT",
  "pins": {{ "VCC": [1], "GND": [2] }},
  "params": {{
    "program": {{ "file": "elf", "as": "blob", "default": "program.elf", "sets": "/firmware/elf" }}
  }},
  "model": {{
    "kind": "esp32s3",
    "firmware": {{}},
    {ESP32S3_FIELDS}
  }}
}}
"#
    )
}

/// A chip that boots from its flash, beside a resistor body. `app` and `partitions` are
/// required with no default and write the chip body. `rom` defaults to the file beside it.
/// `resistance` is the resistor body's field as the model writes it.
fn flash_chip(resistance: &str) -> String {
    format!(
        r#"{{
  "format": 2,
  "part": "flash-chip",
  "version": "0.1.0",
  "title": "Flash chip",
  "license": "MIT",
  "pins": {{ "VCC": [1], "GND": [2], "OUT": [3] }},
  "params": {{
    "app": {{ "file": "bin", "as": "blob", "required": true, "sets": "/mcu/firmware/flash/1" }},
    "partitions": {{ "file": "bin", "as": "blob", "required": true, "sets": "/mcu/firmware/flash/0" }},
    "rom": {{ "file": "elf", "as": "blob", "default": "rom.elf", "sets": "/mcu/rom" }}
  }},
  "model": {{
    "mcu": {{
      "kind": "esp32s3",
      "firmware": {{ "flash": [{{ "offset": 32768 }}, {{ "offset": 65536 }}] }},
      {ESP32S3_FIELDS}
    }},
    "load": {{ "kind": "resistor", "resistance": {resistance}, "ratings": {{ "power": null }}, "pins": {{ "a": "OUT", "b": "GND" }} }}
  }}
}}
"#
    )
}

const LOAD: &str = r#"{ "value": 220, "unit": "ohm", "source": "user" }"#;

/// A module that passes `app` and `partitions` on to the flash chip, as the ESP32-S3 module
/// does, and has a resistor body of its own.
fn flash_module(resistance: &str) -> String {
    format!(
        r#"{{
  "format": 2,
  "part": "flash-module",
  "version": "0.1.0",
  "title": "Flash module",
  "license": "MIT",
  "pins": {{ "VCC": "u1.VCC", "GND": "u1.GND", "LED": [3] }},
  "params": {{ "app": {{}}, "partitions": {{}} }},
  "parts": {{
    "u1": {{ "type": "flash-chip", "app": {{ "param": "app" }}, "partitions": {{ "param": "partitions" }} }}
  }},
  "model": {{ "kind": "resistor", "resistance": {resistance}, "ratings": {{ "power": null }}, "pins": {{ "a": "LED", "b": "GND" }} }}
}}
"#
    )
}

fn source(parts: &[(&str, String)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "registry.jsonc", MANIFEST);
    for (id, text) in parts {
        put(dir.path(), &format!("parts/{id}/part.jsonc"), text);
    }
    dir
}

fn copy(from: &Path, root: &Path, rel: &str) {
    fs::copy(from, root.join(rel)).unwrap();
}

/// The findings `build` refuses `src` with, as `file:line:column code message` lines.
fn refused(src: &Path) -> Vec<String> {
    match build(src, None) {
        Err(Error::Check(found)) => found
            .iter()
            .map(|d| {
                format!(
                    "{}:{}:{} {} {}",
                    d.file, d.line, d.column, d.code, d.message
                )
            })
            .collect(),
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("{} built", src.display()),
    }
}

/// The line and column of the first `needle` in `text`, 1-based.
fn place(text: &str, needle: &str) -> (usize, usize) {
    let at = text.find(needle).unwrap_or_else(|| panic!("no {needle}"));
    let line = text[..at].matches('\n').count() + 1;
    let column = at - text[..at].rfind('\n').map_or(0, |i| i + 1) + 1;
    (line, column)
}

/// The engine reads the bytes the default names. A good program builds, a file that is not an
/// ELF is the engine's finding on that default, and a missing file is `firmware_missing` there.
#[test]
fn a_blob_param_with_a_default_is_probed_at_that_default() {
    let part = elf_chip();
    let src = source(&[("elf-chip", part.clone())]);
    copy(&test_elf(), src.path(), "parts/elf-chip/program.elf");
    let registry = build(src.path(), None).unwrap();
    let files = record(&registry.output, "elf-chip@0.1.0")["files"].to_string();
    assert!(files.contains("program.elf"), "{files}");

    let (line, column) = place(&part, r#""program.elf""#);
    let at = format!("parts/elf-chip/part.jsonc:{line}:{column} ");

    fs::write(src.path().join("parts/elf-chip/program.elf"), "not an ELF").unwrap();
    let found = refused(src.path());
    assert_eq!(
        found,
        [format!(
            "{at}scene.param `r1.program` is program.elf; #r1 firmware is not a 32-bit little-endian Xtensa ELF executable"
        )],
    );

    fs::remove_file(src.path().join("parts/elf-chip/program.elf")).unwrap();
    let found = refused(src.path());
    assert_eq!(
        found,
        [format!(
            "{at}firmware_missing `r1.program`: no file at `program.elf`"
        )],
    );
}

/// `app` and `partitions` stay unset, so the chip body they write waits for a scene. The
/// resistor body beside it, and the module's own resistor, have their values and are checked.
/// The `rom` default is still read.
#[test]
fn a_required_blob_param_holds_back_only_the_bodies_it_reaches() {
    let src = source(&[
        ("flash-chip", flash_chip(LOAD)),
        ("flash-module", flash_module(LOAD)),
    ]);
    copy(&rom_elf(), src.path(), "parts/flash-chip/rom.elf");
    let registry = build(src.path(), None).unwrap();
    for address in ["flash-chip@0.1.0", "flash-module@0.1.0"] {
        record(&registry.output, address);
    }

    // The chip's resistor body is checked while the chip body waits.
    let chip = flash_chip(r#""220""#);
    fs::write(src.path().join("parts/flash-chip/part.jsonc"), &chip).unwrap();
    let found = refused(src.path());
    let (line, column) = place(&chip, r#""220""#);
    let syntax = |site: &str| {
        format!(
            "parts/flash-chip/part.jsonc:{line}:{column} netlist.syntax `{site}`: the engine does not read `resistance`: invalid type: string \"220\", expected a value object or a waveform object"
        )
    };
    // `r1` is the chip placed alone, and `r1.u1` the chip inside the module.
    assert_eq!(found, [syntax("r1.u1"), syntax("r1")]);
    fs::write(
        src.path().join("parts/flash-chip/part.jsonc"),
        flash_chip(LOAD),
    )
    .unwrap();

    // The module's resistor is checked while the chip it passes `app` to waits.
    let module = flash_module(r#""220""#);
    fs::write(src.path().join("parts/flash-module/part.jsonc"), &module).unwrap();
    let found = refused(src.path());
    let (line, column) = place(&module, r#""220""#);
    assert_eq!(
        found,
        [format!(
            "parts/flash-module/part.jsonc:{line}:{column} netlist.syntax `r1`: the engine does not read `resistance`: invalid type: string \"220\", expected a value object or a waveform object"
        )],
    );
    fs::write(
        src.path().join("parts/flash-module/part.jsonc"),
        flash_module(LOAD),
    )
    .unwrap();

    // The `rom` default is read though the body it writes waits, for the chip and the module.
    fs::remove_file(src.path().join("parts/flash-chip/rom.elf")).unwrap();
    let (line, column) = place(&flash_chip(LOAD), r#""rom.elf""#);
    let missing = |site: &str| {
        format!(
            "parts/flash-chip/part.jsonc:{line}:{column} firmware_missing `{site}`: no file at `rom.elf`"
        )
    };
    assert_eq!(
        refused(src.path()),
        [missing("r1.rom"), missing("r1.u1.rom")]
    );
}

/// The catalog's ESP32-S3 chip, module and board build. Their `app` and `partitions` wait for a
/// scene, so no record ships a file for them. The chip's `rom` is the committed ROM, stored with
/// the chip.
#[test]
fn the_catalog_esp32s3_parts_build_with_the_rom_and_without_an_app() {
    let registry = build(&repo(), None).unwrap();
    let chip = record(&registry.output, "esp32s3-qfn56@0.1.0");
    let rom = chip["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "esp32s3_rev0_rom.elf")
        .unwrap_or_else(|| panic!("no ROM in {chip:#}"));
    assert_eq!(rom["bytes"], 949_552, "{rom}");
    for id in [
        "esp32s3-qfn56",
        "esp32s3-wroom-1-n16r8",
        "esp32s3-devkitc-1",
    ] {
        let item = record(&registry.output, &format!("{id}@0.1.0"));
        for file in item["files"].as_array().unwrap() {
            let path = file["path"].as_str().unwrap();
            assert!(!path.ends_with(".bin"), "{id} ships {path}");
        }
    }
}
