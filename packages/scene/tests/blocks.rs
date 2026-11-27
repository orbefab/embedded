//! Each folder in `blocks/` is a Block: a `block.jsonc` that names it, and a scene that reads
//! with the catalog alone and finds its firmware and tests inside the folder.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

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
        let expr = block["license"].as_str().unwrap();
        assert!(
            spdx_parses(expr),
            "{id}: `{expr}` is not a valid SPDX expression"
        );
        assert!(!spdx_parses("MIT OR"), "{id}");
        assert!(!spdx_parses("(MIT OR Apache-2.0"), "{id}");
        assert!(!spdx_parses("GPL-3.0-or-later WITH"), "{id}");
        assert!(
            !spdx_parses("(MIT OR Apache-2.0) WITH GCC-exception-3.1"),
            "{id}"
        );
        let terms = spdx_and_terms(expr);
        assert_eq!(
            terms.first().copied(),
            Some(SKETCH_LICENSE),
            "{id}: license `{expr}` must start with the sketch's licence `{SKETCH_LICENSE}`"
        );
        let mut actual = BTreeSet::new();
        for &term in &terms[1..] {
            assert!(
                actual.insert(term),
                "{id}: license `{expr}` repeats `{term}`"
            );
        }
        let mut expected = BTreeSet::new();
        for entry in fs::read_dir(dir.join("firmware")).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if name.starts_with("LICENSE-") {
                let spdx = license_file_spdx(&name).unwrap_or_else(|| {
                    panic!("{id}: firmware/{name} has no SPDX mapping in license_file_spdx")
                });
                expected.insert(spdx);
            }
        }
        let missing: Vec<&str> = expected.difference(&actual).copied().collect();
        let extra: Vec<&str> = actual.difference(&expected).copied().collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "{id}: block.jsonc license `{expr}` disagrees with firmware/LICENSE-* files (missing: {missing:?}, extra: {extra:?})"
        );
        let notice = fs::read_to_string(dir.join("firmware/NOTICE")).unwrap();
        for term in &actual {
            if term.starts_with("LicenseRef-") {
                assert!(
                    notice.contains(term),
                    "{id}: firmware/NOTICE does not map `{term}` to its license file"
                );
            }
        }
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
        let board = block["board"].as_str().unwrap();
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
        assert!(
            dir.join("firmware/NOTICE").is_file(),
            "{id}: firmware/NOTICE"
        );
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

const SKETCH_LICENSE: &str = "(MIT OR Apache-2.0)";

/// One source of truth from each `firmware/LICENSE-*` file name to its SPDX identifier.
fn license_file_spdx(name: &str) -> Option<&'static str> {
    match name {
        "LICENSE-runtime" | "LICENSE-servo" | "LICENSE-md-max72xx" | "LICENSE-md-parola" => {
            Some("LGPL-2.1-or-later")
        }
        "LICENSE-neopixel" => Some("LGPL-3.0-or-later"),
        "LICENSE-keypad" => Some("GPL-3.0-only"),
        "LICENSE-libc" | "LICENSE-tlsf" | "LICENSE-ssd1306" => Some("BSD-3-Clause"),
        "LICENSE-ubsan" | "LICENSE-gfx" => Some("BSD-2-Clause"),
        "LICENSE-apache" | "LICENSE-sensor" => Some("Apache-2.0"),
        "LICENSE-freertos"
        | "LICENSE-bmp280"
        | "LICENSE-busio"
        | "LICENSE-ledcontrol"
        | "LICENSE-liquidcrystal-i2c"
        | "LICENSE-xtensa" => Some("MIT"),
        "LICENSE-newlib" => Some("LicenseRef-newlib"),
        "LICENSE-gcc-exception" => Some("GPL-3.0-or-later WITH GCC-exception-3.1"),
        _ => None,
    }
}

/// Top-level `AND` terms of an SPDX expression, keeping parenthesized sub-expressions intact.
fn spdx_and_terms(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut terms = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b' ' if depth == 0 && text[i..].starts_with(" AND ") => {
                terms.push(text[start..i].trim());
                i += " AND ".len();
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    terms.push(text[start..].trim());
    terms
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
            if !self.ident() {
                return false;
            }
            if self.op("WITH") { self.ident() } else { true }
        }
        fn and(&mut self) -> bool {
            if !self.primary() {
                return false;
            }
            while self.op("AND") {
                if !self.primary() {
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
