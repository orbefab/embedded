//! Each folder in `blocks/` is a Block: a `block.jsonc` that names it, and a scene that reads
//! with the catalog alone and finds its firmware and tests inside the folder.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use jsonc_parser::{JsonValue, ParseOptions, parse_to_value};
use orbe_scene::{BLOCK, Files};
use serde_json::{Value, json};

fn to_json(value: JsonValue) -> Value {
    match value {
        JsonValue::String(text) => Value::String(text.into_owned()),
        JsonValue::Number(text) => serde_json::from_str(text).unwrap(),
        JsonValue::Boolean(flag) => Value::Bool(flag),
        JsonValue::Null => Value::Null,
        JsonValue::Array(items) => Value::Array(items.into_iter().map(to_json).collect()),
        JsonValue::Object(props) => Value::Object(
            props
                .into_iter()
                .map(|(key, value)| (key.into_owned(), to_json(value)))
                .collect(),
        ),
    }
}

fn read_jsonc(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    let value = parse_to_value(&text, &ParseOptions::default())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .unwrap();
    to_json(value)
}

fn non_empty_text(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
}

/// Each Block is a folder. A file beside them, such as `README.md`, is not one.
fn block_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir(common::repo().join("blocks"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// Identifies a firmware `LICENSE-*` file's SPDX licence from its text.
fn license_from_text(text: &str) -> Option<&'static str> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.starts_with("The newlib subdirectory is a collection of software from several sources.")
    {
        return Some("LicenseRef-newlib");
    }
    if flat.starts_with("GNU LESSER GENERAL PUBLIC LICENSE Version 2.1, February 1999") {
        return Some("LGPL-2.1-or-later");
    }
    if flat.starts_with("GNU LESSER GENERAL PUBLIC LICENSE Version 3, 29 June 2007") {
        return Some("LGPL-3.0-or-later");
    }
    if flat.starts_with("GNU GENERAL PUBLIC LICENSE Version 2, June 1991") {
        return Some("GPL-2.0-or-later");
    }
    if flat.starts_with("GNU GENERAL PUBLIC LICENSE Version 3, 29 June 2007") {
        return Some("GPL-3.0-or-later");
    }
    if flat.starts_with("Apache License Version 2.0, January 2004") {
        return Some("Apache-2.0");
    }
    if flat.contains(
        "Permission is hereby granted, free of charge, to any person obtaining a copy of this \
         software and associated documentation files (the \"Software\"), to deal in the Software \
         without restriction",
    ) && flat.contains(
        "The above copyright notice and this permission notice shall be included in all copies \
         or substantial portions of the Software.",
    ) && flat.contains("THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND")
    {
        return Some("MIT");
    }
    if flat.contains(
        "Redistribution and use in source and binary forms, with or without modification, are \
         permitted provided that the following conditions are met:",
    ) && flat.contains(
        "Redistributions of source code must retain the above copyright notice, this list of \
         conditions and the following disclaimer.",
    ) && flat.contains(
        "Redistributions in binary form must reproduce the above copyright notice, this list of \
         conditions and the following disclaimer in the documentation and/or other materials \
         provided with the distribution.",
    ) && flat.contains("THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS")
    {
        let third_clause = flat.contains("Neither the name of")
            && flat.contains(
                "may be used to endorse or promote products derived from this software without \
                 specific prior written permission.",
            );
        return Some(if third_clause {
            "BSD-3-Clause"
        } else {
            "BSD-2-Clause"
        });
    }
    if flat.contains(
        "Permission to use, copy, modify, and/or distribute this software for any purpose with \
         or without fee is hereby granted",
    ) {
        return Some("ISC");
    }
    None
}

/// Every `LICENSE-*` file name mentioned in `notice`.
fn notice_license_files(notice: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes = notice.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if notice[i..].starts_with("LICENSE-") {
            let mut end = i + "LICENSE-".len();
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b'_'))
            {
                end += 1;
            }
            if end > i + "LICENSE-".len() {
                found.insert(notice[i..end].to_string());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    found
}

/// The firmware licences named in `block.jsonc` after the `(MIT OR Apache-2.0)` prefix for
/// orbe's own files.
fn block_firmware_licenses<'a>(expr: &'a str, id: &str) -> BTreeSet<&'a str> {
    let rest = expr
        .strip_prefix("(MIT OR Apache-2.0) AND ")
        .unwrap_or_else(|| {
            panic!("{id}: license `{expr}` does not start with `(MIT OR Apache-2.0) AND `")
        });
    let mut terms = BTreeSet::new();
    for term in rest.split(" AND ") {
        assert!(
            !term.is_empty() && !term.contains(char::is_whitespace),
            "{id}: unexpected license term `{term}` in `{expr}`"
        );
        assert!(
            terms.insert(term),
            "{id}: duplicate license term `{term}` in `{expr}`"
        );
    }
    assert!(
        !terms.is_empty(),
        "{id}: license `{expr}` has no firmware licence"
    );
    terms
}

/// The firmware licences `firmware/NOTICE` names for the built image under `build/`.
fn notice_build_licenses(notice: &str, id: &str) -> BTreeSet<String> {
    let (_, tail) = notice
        .split_once("These files and their licences:")
        .unwrap_or_else(|| panic!("{id}: firmware/NOTICE has no licence summary"));
    let flat = tail.split_whitespace().collect::<Vec<_>>().join(" ");
    let (_, from_build) = flat
        .split_once("`build/")
        .unwrap_or_else(|| panic!("{id}: firmware/NOTICE names no build/ image in its summary"));
    let (_, after_under) = from_build
        .split_once(" under ")
        .unwrap_or_else(|| panic!("{id}: firmware/NOTICE gives no licence for its build/ image"));
    let end = after_under
        .match_indices('.')
        .find(|(at, _)| {
            after_under[*at + 1..].is_empty() || after_under[*at + 1..].starts_with(' ')
        })
        .map(|(at, _)| at)
        .unwrap_or(after_under.len());
    let terms: BTreeSet<String> = after_under[..end]
        .split(", ")
        .map(|part| part.strip_prefix("and ").unwrap_or(part))
        .flat_map(|part| part.split(" and "))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    assert!(
        !terms.is_empty(),
        "{id}: firmware/NOTICE names no licence for its build/ image"
    );
    terms
}

/// Base toolchain notices a board's firmware carries beside `LICENSE-runtime`, verified against
/// the repo's pinned copies. `block.jsonc` does not repeat these toolchain notices in its
/// expression; extra linked libraries must appear there.
fn board_toolchain_notices(repo: &Path, board: &str) -> Vec<(&'static str, PathBuf)> {
    if board == "esp32s3-devkitc-1" {
        let chip = repo.join("parts/esp32s3-qfn56");
        vec![
            ("LICENSE-apache", repo.join("LICENSE-APACHE")),
            ("LICENSE-freertos", chip.join("LICENSE-freertos")),
            ("LICENSE-newlib", chip.join("LICENSE-newlib")),
            ("LICENSE-tlsf", chip.join("LICENSE-tlsf")),
            ("LICENSE-ubsan", chip.join("LICENSE-ubsan")),
            ("LICENSE-xtensa", chip.join("LICENSE-xtensa")),
        ]
    } else {
        vec![(
            "LICENSE-libc",
            repo.join("packages/engine/tests/firmware/sketch/LICENSE-libc"),
        )]
    }
}

/// True when any `.ino` sketch under `firmware` contains `needle`.
fn sketch_contains(firmware: &Path, needle: &str) -> bool {
    for entry in fs::read_dir(firmware).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            for inner in fs::read_dir(&path).unwrap() {
                let file = inner.unwrap().path();
                if file.extension().and_then(|ext| ext.to_str()) == Some("ino")
                    && fs::read_to_string(&file).unwrap().contains(needle)
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Identifies every `firmware/LICENSE-*` file from its text, verifies required board toolchain
/// pins and bidirectional `NOTICE` coverage, and returns the set of licences that must appear
/// in `block.jsonc`.
fn firmware_file_licenses(dir: &Path, board: &str, notice: &str, id: &str) -> BTreeSet<String> {
    let repo = common::repo();
    let firmware = dir.join("firmware");
    let runtime_pin = repo.join("packages/engine/tests/firmware/sketch/LICENSE-runtime");
    let toolchain = board_toolchain_notices(&repo, board);

    let mut disk_files = BTreeSet::new();
    for entry in fs::read_dir(&firmware).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if name.starts_with("LICENSE-") {
            disk_files.insert(name);
        }
    }

    assert!(
        disk_files.contains("LICENSE-runtime"),
        "{id}: firmware/LICENSE-runtime is missing"
    );
    assert_eq!(
        fs::read(firmware.join("LICENSE-runtime")).unwrap(),
        fs::read(&runtime_pin).unwrap(),
        "{id}: firmware/LICENSE-runtime does not match pinned runtime licence"
    );

    let mut omitted_from_expr: BTreeSet<&str> = BTreeSet::new();
    for (name, pin) in &toolchain {
        assert!(
            disk_files.contains(*name),
            "{id}: required toolchain notice firmware/{name} is missing"
        );
        assert_eq!(
            fs::read(firmware.join(name)).unwrap(),
            fs::read(pin).unwrap(),
            "{id}: firmware/{name} does not match {}",
            pin.display()
        );
        omitted_from_expr.insert(name);
    }

    // `Adafruit_GFX.h` includes `Adafruit_BusIO` headers at compile time while `sketches.sh`
    // links no BusIO code, so `LICENSE-busio` is carried without adding `MIT` to `block.jsonc`.
    if disk_files.contains("LICENSE-busio")
        && disk_files.contains("LICENSE-gfx")
        && sketch_contains(&firmware, "#include <Adafruit_GFX.h>")
    {
        let busio_pin =
            repo.join("packages/engine/tests/firmware/esp32s3/arduino/stock/oled/LICENSE-busio");
        assert_eq!(
            fs::read(firmware.join("LICENSE-busio")).unwrap(),
            fs::read(&busio_pin).unwrap(),
            "{id}: firmware/LICENSE-busio does not match {}",
            busio_pin.display()
        );
        omitted_from_expr.insert("LICENSE-busio");
    }

    let mentioned = notice_license_files(notice);
    assert_eq!(
        disk_files, mentioned,
        "{id}: firmware/LICENSE-* files on disk do not match firmware/NOTICE"
    );

    let (_, tail) = notice
        .split_once("These files and their licences:")
        .unwrap_or_else(|| panic!("{id}: firmware/NOTICE has no licence summary"));
    let flat_summary = tail.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut expr_licenses = BTreeSet::new();
    for name in &disk_files {
        let text = fs::read_to_string(firmware.join(name)).unwrap();
        let detected = license_from_text(&text).unwrap_or_else(|| {
            panic!("{id}: cannot identify licence from text of firmware/{name}")
        });

        let prefix = format!("`firmware/{name}` is under ");
        if let Some((_, after)) = flat_summary.split_once(&prefix) {
            let stated = after
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches('.');
            if stated != "the" {
                assert_eq!(
                    stated, detected,
                    "{id}: firmware/NOTICE says firmware/{name} is under {stated}, text is {detected}"
                );
            }
        }

        if !omitted_from_expr.contains(name.as_str()) {
            expr_licenses.insert(detected.to_string());
        }
    }
    expr_licenses
}

#[test]
fn each_block_names_itself_and_reads_with_the_catalog_alone() {
    let dirs = block_dirs();
    assert!(dirs.len() >= 3, "{dirs:?}");
    for dir in dirs {
        let id = dir.file_name().unwrap().to_str().unwrap();
        let block = read_jsonc(&dir.join(BLOCK));
        let keys: Vec<&str> = block
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "block", "board", "format", "license", "summary", "tags", "title", "version"
            ],
            "{id}"
        );
        assert_eq!(block["format"], json!(1), "{id}");
        let version = block["version"].as_str().unwrap();
        assert!(
            orbe_scene::Version::parse(version).is_some(),
            "{id}: version"
        );
        assert_eq!(
            block["block"],
            json!(id),
            "{id}: `block` is the folder's name"
        );
        for key in ["title", "summary", "license"] {
            assert!(non_empty_text(&block[key]), "{id}: {key}");
        }
        assert!(
            dir.join("firmware/NOTICE").is_file(),
            "{id}: firmware/NOTICE"
        );
        let notice = fs::read_to_string(dir.join("firmware/NOTICE")).unwrap();
        let board = block["board"].as_str().unwrap();
        let block_license = block["license"].as_str().unwrap();
        assert!(spdx_parses(block_license), "{id}");
        assert!(!spdx_parses("MIT OR"), "{id}");
        assert!(!spdx_parses("(MIT OR Apache-2.0"), "{id}");
        let from_files = firmware_file_licenses(&dir, board, &notice, id);
        let from_block: BTreeSet<String> = block_firmware_licenses(block_license, id)
            .into_iter()
            .map(str::to_string)
            .collect();
        let from_notice = notice_build_licenses(&notice, id);
        assert_eq!(
            from_block, from_files,
            "{id}: block.jsonc license does not match firmware/LICENSE-* files"
        );
        assert_eq!(
            from_notice, from_files,
            "{id}: firmware/NOTICE build summary does not match firmware/LICENSE-* files"
        );
        let tags = block["tags"].as_array().unwrap();
        assert!(
            !tags.is_empty() && tags.iter().all(non_empty_text),
            "{id}: tags"
        );

        assert!(!dir.join("parts").exists(), "{id} has parts");
        assert!(dir.join("README.md").is_file(), "{id}: README.md");
        let tests = fs::read_dir(dir.join("tests")).unwrap().count();
        assert!(tests > 0, "{id}: tests/");

        let scene = dir.join("scene.jsonc");
        let netlist = orbe_scene::read(&Files::disk(), &scene)
            .unwrap_or_else(|found| panic!("{id}: {found:#?}"));
        let boards: Vec<&str> = netlist
            .refs
            .top
            .iter()
            .filter(|name| netlist.refs.parts[*name].ty == board)
            .map(String::as_str)
            .collect();
        assert_eq!(boards.len(), 1, "{id}: the scene holds one `{board}`");

        let written = read_jsonc(&scene);
        for (name, part) in written["parts"].as_object().unwrap() {
            if let Some(firmware) = part.get("firmware") {
                let firmware = firmware.as_str().unwrap();
                assert!(
                    firmware.starts_with("build/")
                        && firmware.ends_with(".ino.hex")
                        && dir.join(firmware).is_file(),
                    "{id}: `{name}` runs `{firmware}`, the file arduino-cli writes under build/"
                );
                assert!(
                    notice.contains(firmware),
                    "{id}: firmware/NOTICE does not name `{firmware}`"
                );
            }
            for key in ["app", "partitions"] {
                if let Some(image) = part.get(key).and_then(|value| value.as_str()) {
                    assert!(
                        image.starts_with("build/") && dir.join(image).is_file(),
                        "{id}: `{name}` {key} `{image}` is not a file under build/"
                    );
                    assert!(
                        notice.contains(image),
                        "{id}: firmware/NOTICE does not name `{image}`"
                    );
                }
            }
        }
    }
}

/// `sketches.sh` rebuilds a Block's image to the path arduino-cli writes, from the sketch in
/// `firmware/`, and the licences it carries are the ones that build needs.
#[test]
fn each_block_ships_the_image_arduino_cli_writes() {
    let sketches = common::repo().join("packages/engine/tests/firmware/sketch");
    for dir in block_dirs() {
        let firmware = dir.join("firmware");
        let build = dir.join("build");
        let mut images = 0;
        for file in fs::read_dir(&build).unwrap() {
            let name = file.unwrap().file_name().into_string().unwrap();
            let sketch = name
                .strip_suffix(".ino.hex")
                .or_else(|| name.strip_suffix(".ino.bin"));
            let Some(sketch) = sketch else {
                continue;
            };
            let source = firmware.join(format!("{sketch}/{sketch}.ino"));
            assert!(source.is_file(), "{}", source.display());
            if name.ends_with(".ino.bin") {
                assert!(
                    build.join(format!("{sketch}.ino.partitions.bin")).is_file(),
                    "{}: the app has no partition table",
                    build.display()
                );
            }
            images += 1;
        }
        assert!(images > 0, "{}", build.display());
        let notice = fs::read_to_string(firmware.join("NOTICE")).unwrap();
        for file in fs::read_dir(&firmware).unwrap() {
            let name = file.unwrap().file_name().into_string().unwrap();
            if name.starts_with("LICENSE-") {
                let pinned = sketches.join(&name);
                if pinned.is_file() {
                    assert_eq!(
                        fs::read(firmware.join(&name)).unwrap(),
                        fs::read(&pinned).unwrap(),
                        "{}: {name}",
                        firmware.display()
                    );
                } else {
                    assert!(
                        notice.contains(&name),
                        "{}: {name} is not a runtime licence, and NOTICE does not name it",
                        firmware.display()
                    );
                }
            }
        }
    }
}

/// Each Block notice names the runtime archive `sketches.sh` fetches, by the same URL and SHA-256.
#[test]
fn each_block_notice_names_the_runtime_sketches_sh_pins_and_where_the_site_serves_it() {
    let script =
        fs::read_to_string(common::repo().join("packages/engine/scripts/sketches.sh")).unwrap();
    let pin = |key: &str| {
        script
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("{key} is not pinned in sketches.sh"))
            .to_string()
    };
    let url = pin("runtime_url");
    let sha = pin("runtime_sha256");
    assert!(url.starts_with("https://"), "{url}");
    assert_eq!(sha.len(), 64);
    // Released NOTICE files keep this URL. The site redirects it to /orbe/sources/.
    let hosted = format!(
        "https://orbefab.com/embedded/sources/{}",
        url.rsplit('/').next().unwrap()
    );
    for dir in block_dirs() {
        let notice = fs::read_to_string(dir.join("firmware/NOTICE")).unwrap();
        let board = read_jsonc(&dir.join("block.jsonc"))["board"]
            .as_str()
            .unwrap()
            .to_string();
        if board == "esp32s3-devkitc-1" {
            assert!(
                notice.contains("esp32:esp32 3.3.12") && notice.contains("esp32:esp32:esp32s3"),
                "{} notice does not name the ESP32S3 Dev Module core and fqbn",
                dir.display()
            );
            continue;
        }
        assert!(
            notice.contains(&hosted),
            "{} notice does not name {hosted}, where scripts/site.sh serves the source",
            dir.display()
        );
        assert!(
            notice.contains(&url),
            "{} notice does not name {url}",
            dir.display()
        );
        assert!(
            notice.contains(&sha),
            "{} notice does not name {sha}",
            dir.display()
        );
    }
}

/// SPDX license expression, enough to accept a Block's `license` and reject a broken one.
/// `AND` binds tighter than `OR`.
fn spdx_parses(text: &str) -> bool {
    struct Scan<'a> {
        text: &'a str,
    }
    impl Scan<'_> {
        fn skip(&mut self) {
            self.text = self.text.trim_start();
        }
        fn word(&mut self) -> Option<String> {
            self.skip();
            let end = self
                .text
                .find(|c: char| !(c.is_ascii_alphanumeric() || ".-+".contains(c)))
                .unwrap_or(self.text.len());
            if end == 0 {
                return None;
            }
            let word = self.text[..end].to_string();
            self.text = &self.text[end..];
            Some(word)
        }
        fn op(&mut self, op: &str) -> bool {
            self.skip();
            let rest = self.text.get(op.len()..).unwrap_or("");
            let boundary = rest.is_empty()
                || rest.starts_with(|c: char| !(c.is_ascii_alphanumeric() || ".-+".contains(c)));
            if self.text.starts_with(op) && boundary {
                self.text = &self.text[op.len()..];
                true
            } else {
                false
            }
        }
        fn ident(&mut self) -> bool {
            matches!(
                self.word(),
                Some(word) if !matches!(word.as_str(), "AND" | "OR" | "WITH")
            )
        }
        fn primary(&mut self) -> bool {
            self.skip();
            if self.text.starts_with('(') {
                self.text = &self.text[1..];
                return self.or() && self.op(")");
            }
            self.ident()
        }
        fn with(&mut self) -> bool {
            if !self.primary() {
                return false;
            }
            if self.op("WITH") { self.ident() } else { true }
        }
        fn and(&mut self) -> bool {
            if !self.with() {
                return false;
            }
            while self.op("AND") {
                if !self.with() {
                    return false;
                }
            }
            true
        }
        fn or(&mut self) -> bool {
            if !self.and() {
                return false;
            }
            while self.op("OR") {
                if !self.and() {
                    return false;
                }
            }
            true
        }
    }
    let mut scan = Scan { text };
    scan.or() && {
        scan.skip();
        scan.text.is_empty()
    }
}

fn block_with(scene: &str, extra: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(BLOCK), "{}\n").unwrap();
    fs::write(dir.path().join("scene.jsonc"), scene).unwrap();
    for name in extra {
        fs::write(dir.path().join(name), "").unwrap();
    }
    dir
}

fn findings(dir: &Path) -> Vec<String> {
    match orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc")) {
        Ok(_) => Vec::new(),
        Err(found) => found
            .iter()
            .map(|d| format!("{}: {} | {}", d.code.as_str(), d.message, d.hint))
            .collect(),
    }
}

#[test]
fn a_block_names_an_id_the_catalog_lacks() {
    let dir = block_with(
        r#"{ "format": 1, "parts": { "led1": { "type": "led-5mn" } } }"#,
        &[],
    );
    assert_eq!(
        findings(dir.path()),
        [
            "part_missing: part `led-5mn` is not in the catalog this Block reads | the catalog has no part named `led-5mn`"
        ]
    );
}

/// A lock or copies beside a Block would not be read, so the reader says so.
#[test]
fn a_block_that_holds_a_lock_is_refused() {
    let dir = block_with(
        r#"{ "format": 1, "parts": { "led1": { "type": "led-5mm" } } }"#,
        &["parts"],
    );
    assert_eq!(
        findings(dir.path())[0],
        "scene.role: this folder holds block.jsonc, so it is a Block, which reads the parts/ beside blocks/, and it also holds `parts` | remove `parts`, or remove block.jsonc to make the folder a project that reads its own copies"
    );
}

/// A Block ships no `REUSE.toml`. Each file's record takes the Block's licence, and
/// `firmware/NOTICE` states which file carries which licence.
#[test]
fn each_block_file_takes_the_block_licence() {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                into.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    let parts = orbe_scene::catalog_cards(&common::repo().join("parts"));
    for dir in block_dirs() {
        let id = dir.file_name().unwrap().to_string_lossy().into_owned();
        let mut files = BTreeMap::new();
        walk(&dir, &dir, &mut files);
        let card = orbe_scene::block_card(&id, &files, &parts).unwrap();
        assert!(
            !files.contains_key("REUSE.toml"),
            "{id} still has REUSE.toml"
        );
        let notice = std::str::from_utf8(files.get("firmware/NOTICE").unwrap()).unwrap();
        for fact in ["MIT OR Apache-2.0", "LGPL-2.1-or-later", "BSD-3-Clause"] {
            assert!(notice.contains(fact), "{id}: NOTICE does not state {fact}");
        }
        for file in &card.files {
            assert_eq!(file.license, card.license, "{id}/{}", file.path);
        }
    }
}
