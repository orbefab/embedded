//! Blob params in a registry build. A blob param with a default is read at that default. A
//! required one is read at its example, and the file it names is the copy in the part's folder.

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

/// The app image the catalog chip commits as the example of its `app` param.
fn hello_app() -> PathBuf {
    repo().join("packages/scene/tests/fixtures/esp32s3-hello/firmware/hello_world.bin")
}

/// The partition table the catalog chip commits as the example of its `partitions` param.
fn hello_table() -> PathBuf {
    repo().join("packages/scene/tests/fixtures/esp32s3-hello/firmware/partition-table.bin")
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
    "app": {{ "file": "bin", "as": "blob", "required": true, "example": "app.bin", "sets": "/mcu/firmware/flash/1" }},
    "partitions": {{ "file": "bin", "as": "blob", "required": true, "example": "partitions.bin", "sets": "/mcu/firmware/flash/0" }},
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
    match build(src) {
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
    let registry = build(src.path()).unwrap();
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

/// `app` and `partitions` are read at their examples, so the chip body is checked. The
/// resistor body beside it, and the module's own resistor, are checked too. The `rom` default
/// is still read.
#[test]
fn a_required_blob_param_is_read_at_its_example() {
    let src = source(&[
        ("flash-chip", flash_chip(LOAD)),
        ("flash-module", flash_module(LOAD)),
    ]);
    copy(&rom_elf(), src.path(), "parts/flash-chip/rom.elf");
    copy(&hello_app(), src.path(), "parts/flash-chip/app.bin");
    copy(
        &hello_table(),
        src.path(),
        "parts/flash-chip/partitions.bin",
    );
    let registry = build(src.path()).unwrap();
    for address in ["flash-chip@0.1.0", "flash-module@0.1.0"] {
        record(&registry.output, address);
    }

    // The chip's resistor body is checked, and so is the chip body at its examples.
    let chip = flash_chip(r#""220""#);
    fs::write(src.path().join("parts/flash-chip/part.jsonc"), &chip).unwrap();
    let found = refused(src.path());
    let (line, column) = place(&chip, r#""220""#);
    let syntax = |file: &str, site: &str| {
        format!(
            "{file}:{line}:{column} netlist.syntax `{site}`: the engine does not read `resistance`: invalid type: string \"220\", expected a value object or a waveform object"
        )
    };
    // `r1` is the chip placed alone. `r1.u1` is that chip nested inside the module.
    assert_eq!(
        found,
        [
            syntax("parts/flash-chip/part.jsonc", "r1"),
            syntax("parts/flash-module/parts/flash-chip/part.jsonc", "r1.u1"),
        ]
    );
    fs::write(
        src.path().join("parts/flash-chip/part.jsonc"),
        flash_chip(LOAD),
    )
    .unwrap();

    // The module's resistor is checked. The chip it passes `app` to is checked at the example.
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

    // The `rom` default is read, for the chip and the module.
    fs::remove_file(src.path().join("parts/flash-chip/rom.elf")).unwrap();
    let (line, column) = place(&flash_chip(LOAD), r#""rom.elf""#);
    let missing = |file: &str, site: &str| {
        format!("{file}:{line}:{column} firmware_missing `{site}`: no file at `rom.elf`")
    };
    assert_eq!(
        refused(src.path()),
        [
            missing("parts/flash-chip/part.jsonc", "r1.rom"),
            missing(
                "parts/flash-module/parts/flash-chip/part.jsonc",
                "r1.u1.rom"
            ),
        ]
    );
}

/// The catalog's ESP32-S3 chip, module and board build. The chip ships its `rom` and the `app`
/// and `partitions` examples. The module and the board pass those params on, so they ship no
/// image of their own.
#[test]
fn the_catalog_esp32s3_parts_build_with_the_rom_and_the_app_examples() {
    let registry = build(&repo()).unwrap();
    let chip = record(&registry.output, "esp32s3-qfn56@0.2.0");
    let paths: Vec<&str> = chip["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    let rom = chip["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "esp32s3_rev0_rom.elf")
        .unwrap_or_else(|| panic!("no ROM in {chip:#}"));
    assert_eq!(rom["bytes"], 949_552, "{rom}");
    assert!(paths.contains(&"hello_world.bin"), "{paths:?}");
    assert!(paths.contains(&"partition-table.bin"), "{paths:?}");
    for (id, version) in [
        ("esp32s3-wroom-1-n16r8", "0.2.0"),
        ("esp32s3-devkitc-1", "0.2.1"),
    ] {
        let item = record(&registry.output, &format!("{id}@{version}"));
        for file in item["files"].as_array().unwrap() {
            let path = file["path"].as_str().unwrap();
            assert!(!path.ends_with(".bin"), "{id} ships {path}");
        }
    }
}
