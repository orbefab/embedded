//! Each folder in `blocks/` is a Block: a `block.jsonc` that names it, and a scene that reads
//! with the catalog alone and finds its firmware and tests inside the folder.

mod common;

use std::collections::BTreeMap;
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
        assert_eq!(
            block["license"], "(MIT OR Apache-2.0) AND LGPL-2.1-or-later",
            "{id}: license is the SPDX expression for the sketch and the LGPL runtime"
        );
        assert!(spdx_parses(block["license"].as_str().unwrap()), "{id}");
        assert!(!spdx_parses("MIT OR"), "{id}");
        assert!(!spdx_parses("(MIT OR Apache-2.0"), "{id}");
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
                    firmware.starts_with("firmware/") && dir.join(firmware).is_file(),
                    "{id}: `{name}` runs `{firmware}`, which must be in the Block's firmware/"
                );
            }
        }
        assert!(
            dir.join("firmware/NOTICE").is_file(),
            "{id}: firmware/NOTICE"
        );
    }
}

/// `sketches.sh` rebuilds a Block's image from the sketch beside it, and the licences it carries
/// are the ones that build needs.
#[test]
fn each_block_firmware_is_built_from_the_sketch_beside_it() {
    let sketches = common::repo().join("packages/engine/tests/firmware/sketch");
    for dir in block_dirs() {
        let firmware = dir.join("firmware");
        let mut images = 0;
        for file in fs::read_dir(&firmware).unwrap() {
            let name = file.unwrap().file_name().into_string().unwrap();
            if let Some(sketch) = name.strip_suffix(".hex") {
                let source = firmware.join(format!("{sketch}/{sketch}.ino"));
                assert!(source.is_file(), "{}", source.display());
                images += 1;
            } else if name.starts_with("LICENSE-") {
                assert_eq!(
                    fs::read(firmware.join(&name)).unwrap(),
                    fs::read(sketches.join(&name)).unwrap(),
                    "{}: {name}",
                    firmware.display()
                );
            }
        }
        assert!(images > 0, "{}", firmware.display());
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
    let hosted = format!(
        "https://orbefab.com/embedded/sources/{}",
        url.rsplit('/').next().unwrap()
    );
    for dir in block_dirs() {
        let notice = fs::read_to_string(dir.join("firmware/NOTICE")).unwrap();
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
        r#"{ "format": 1, "parts": { "led1": { "type": "led-5mn", "color": "red" } } }"#,
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
        r#"{ "format": 1, "parts": { "led1": { "type": "led-5mm", "color": "red" } } }"#,
        &["parts"],
    );
    assert_eq!(
        findings(dir.path())[0],
        "scene.role: this folder holds block.jsonc, so it is a Block, which reads the parts/ beside blocks/, and it also holds `parts` | remove `parts`, or remove block.jsonc to make the folder a project that reads its own copies"
    );
}

#[test]
fn each_block_file_names_its_own_licence() {
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
        for file in &card.files {
            let want = if file.path.ends_with(".hex") || file.path == "firmware/LICENSE-runtime" {
                "LGPL-2.1-or-later"
            } else if file.path == "firmware/LICENSE-libc" {
                "BSD-3-Clause"
            } else {
                "MIT OR Apache-2.0"
            };
            assert_eq!(file.license, want, "{id}/{}", file.path);
        }
    }
}
