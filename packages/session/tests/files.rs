//! Each Block, read into a set of files in memory, reads, lints and runs its tests as it does
//! from disk, and so does each Block installed into a project. Paths in memory start from the
//! repo, so a disk path shows the same once the repo's folder is taken off it. A Block is a
//! folder under `blocks/` that holds `block.jsonc`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use orbe_session::{Files, test};
use tempfile::TempDir;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Every Block in `dir`, by folder name. A Block is a folder that holds `block.jsonc`.
fn blocks_in(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("reading {}: {err}", dir.display()))
    {
        let path = entry.unwrap().path();
        if path.join(orbe_scene::BLOCK).is_file() {
            found.push(path.file_name().unwrap().to_str().unwrap().to_string());
        }
    }
    found.sort();
    assert!(
        !found.is_empty(),
        "no Block in {}: a Block is a folder with {}",
        dir.display(),
        orbe_scene::BLOCK
    );
    found
}

fn blocks() -> Vec<String> {
    blocks_in(&repo().join("blocks"))
}

/// The Block's folder, by its paths from `base`.
fn memory(base: &Path, block: &str) -> Files {
    let mut files = Vec::new();
    walk(base, &base.join("blocks").join(block), &mut files);
    Files::memory(files)
}

/// The Block installed by `orbe add` into `<base>/blocks/<block>`, so it is no longer a Block.
fn install(base: &Path, block: &str) -> PathBuf {
    let dir = base.join("blocks").join(block);
    fs::create_dir_all(&dir).unwrap();
    let request = orbe_scene::Request {
        ids: vec![block.to_string()],
        overwrite: false,
        diff: false,
        relock: false,
    };
    let reply = orbe_scene::add_parts(Some(&dir), &request).unwrap();
    assert_eq!(reply.code(), 0, "{}", reply.text());
    dir
}

fn walk(base: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            walk(base, &entry.unwrap().path(), out);
        }
    } else if path.exists() {
        let key = path.strip_prefix(base).unwrap().to_path_buf();
        out.push((key, fs::read(path).unwrap()));
    }
}

/// What the reader makes of one Block, as text. This does not play the Block's tests.
fn reading(files: &Files, base: &Path, block: &str) -> Vec<String> {
    let scene = base.join("blocks").join(block).join("scene.jsonc");
    vec![
        match orbe_scene::read(files, &scene) {
            Ok(netlist) => format!(
                "{}\n{} {:?} {} {:?}\n{:?}\n{}",
                netlist.json,
                netlist.parts,
                netlist.nets,
                netlist.ground,
                netlist.modified,
                netlist.refs,
                serde_json::to_string(&netlist.view).unwrap()
            ),
            Err(found) => format!("{found:#?}"),
        },
        format!("{:#?}", orbe_scene::lint(files, &scene)),
    ]
}

/// What `orbe test` makes of one Block. The test text is the Block's own, in the repo.
fn played(files: &Files, base: &Path, block: &str) -> Vec<String> {
    let dir = base.join("blocks").join(block);
    let tests = repo().join("blocks").join(block).join("tests");
    let mut paths: Vec<_> = fs::read_dir(&tests)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let file = dir.join("tests").join(path.file_name().unwrap());
        let report = test::run(
            files.clone(),
            &file,
            &fs::read_to_string(&path).unwrap(),
            &dir,
        );
        let passed = if report.passed() { "passed" } else { "failed" };
        out.push(format!("{} {passed}", report.file.display()));
        for line in &report.lines {
            let reply = serde_json::to_string(&line.reply).unwrap();
            out.push(format!("{} {}: {reply}", line.number, line.text));
        }
        out.extend(
            report
                .refused
                .map(|reply| serde_json::to_string(&reply).unwrap()),
        );
    }
    out
}

/// The read, the lint and the test run.
fn outcome(files: &Files, base: &Path, block: &str) -> Vec<String> {
    let mut out = reading(files, base, block);
    out.extend(played(files, base, block));
    out
}

/// Disk paths with `base/` taken off, so they match the paths memory stored.
fn shown(base: &Path, lines: Vec<String>) -> Vec<String> {
    let prefix = format!("{}/", base.display());
    lines
        .into_iter()
        .map(|text| text.replace(&prefix, ""))
        .collect()
}

fn same_lines(block: &str, disk: Vec<String>, memory: Vec<String>) -> Vec<String> {
    assert_eq!(disk.len(), memory.len(), "{block}");
    for (disk, memory) in disk.iter().zip(&memory) {
        assert_eq!(disk, memory, "{block}");
    }
    disk
}

/// The reply `open` returns for each of the Block's test files. On a broken install the read
/// has already failed, so `open` returns without starting the run.
fn open_replies(files: &Files, base: &Path, block: &str) -> Vec<String> {
    let dir = base.join("blocks").join(block);
    let tests = repo().join("blocks").join(block).join("tests");
    let Ok(entries) = fs::read_dir(&tests) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries.map(|entry| entry.unwrap().path()).collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let file = dir.join("tests").join(path.file_name().unwrap());
        let report = test::run(files.clone(), &file, "open\n", &dir);
        for line in &report.lines {
            let reply = serde_json::to_string(&line.reply).unwrap();
            out.push(format!("{}: {reply}", line.text));
        }
        out.extend(
            report
                .refused
                .map(|reply| serde_json::to_string(&reply).unwrap()),
        );
    }
    out
}

/// The read, the lint and the `open` reply, from disk and from memory.
fn same_reading_and_open(base: &Path, block: &str) -> Vec<String> {
    let disk = Files::disk();
    let held = memory(base, block);
    let mut disk_lines = reading(&disk, base, block);
    disk_lines.extend(open_replies(&disk, base, block));
    let mut memory_lines = reading(&held, Path::new(""), block);
    memory_lines.extend(open_replies(&held, Path::new(""), block));
    same_lines(block, shown(base, disk_lines), memory_lines)
}

fn same_both_ways(base: &Path, block: &str) -> Vec<String> {
    same_lines(
        block,
        shown(base, outcome(&Files::disk(), base, block)),
        outcome(&memory(base, block), Path::new(""), block),
    )
}

fn reads_and_passes(found: &[String], block: &str) {
    assert!(found[0].starts_with('{'), "{block} reads: {}", found[0]);
    assert!(found[1].starts_with("Ok("), "{block} lints: {}", found[1]);
    assert!(found[2].ends_with(".test passed"), "{block}: {found:#?}");
}

#[test]
#[should_panic(expected = "no Block in")]
fn a_directory_with_no_block_is_an_error() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join("skip")).unwrap();
    fs::write(tmp.path().join("readme.txt"), "hi").unwrap();
    blocks_in(tmp.path());
}

#[test]
fn each_folder_with_block_jsonc_is_a_block() {
    let tmp = TempDir::new().unwrap();
    for name in ["zeta", "alpha"] {
        fs::create_dir(tmp.path().join(name)).unwrap();
        fs::write(tmp.path().join(name).join(orbe_scene::BLOCK), "{}\n").unwrap();
    }
    fs::create_dir(tmp.path().join("skip")).unwrap();
    fs::write(tmp.path().join("loose.txt"), "no").unwrap();
    assert_eq!(blocks_in(tmp.path()), ["alpha", "zeta"]);
}

#[test]
fn each_block_reads_and_runs_the_same_from_memory() {
    for block in blocks() {
        let block = block.as_str();
        reads_and_passes(&same_both_ways(&repo(), block), block);
    }
}

/// The repo's part folders, as a catalog handed in, each file by `<id>/<path>`.
fn given_catalog(skip: &str) -> Vec<(String, Vec<u8>)> {
    let parts = repo().join("parts");
    let mut files = Vec::new();
    walk(&parts, &parts, &mut files);
    files
        .into_iter()
        .map(|(path, bytes)| (path.to_str().unwrap().to_string(), bytes))
        .filter(|(path, _)| !path.starts_with(&format!("{skip}/")))
        .collect()
}

/// A Block handed its catalog as part folders, as a browser is, reads and runs as it does with
/// the catalog built in.
#[test]
fn each_block_reads_and_runs_the_same_from_a_given_catalog() {
    for block in blocks() {
        let block = block.as_str();
        let built_in = outcome(&memory(&repo(), block), Path::new(""), block);
        let given = memory(&repo(), block).with_catalog(given_catalog(""));
        assert_eq!(outcome(&given, Path::new(""), block), built_in, "{block}");
    }
}

/// A given catalog replaces the built-in one, so a part it lacks is missing.
#[test]
fn a_given_catalog_without_a_part_finds_it_missing() {
    let files = memory(&repo(), "nano-button-led").with_catalog(given_catalog("led-5mm"));
    let scene = Path::new("blocks/nano-button-led/scene.jsonc");
    assert_eq!(
        messages(&files, scene),
        ["part `led-5mm` is not in the catalog this Block reads"]
    );
}

/// An installed Block reads its own copies, and reads and runs as the Block does.
#[test]
fn each_installed_block_reads_and_runs_the_same_from_memory() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    for block in blocks() {
        let block = block.as_str();
        install(&base, block);
        let found = same_both_ways(&base, block);
        reads_and_passes(&found, block);
        let ours = shown(&repo(), reading(&Files::disk(), &repo(), block));
        let netlist = |found: &[String]| found[0].rsplit_once('\n').unwrap().0.to_string();
        assert_eq!(
            netlist(&found),
            netlist(&ours),
            "{block}: the installed netlist"
        );
    }
}

/// Part folders `install` wrote, sorted by folder name. A directory with no `part.jsonc` is not one.
fn part_folders(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir.join("parts")) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("part.jsonc").is_file())
        .collect();
    found.sort();
    found
}

/// Every file under `dir`, in sorted order.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.is_file() {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Appends a byte to one file of `part` and writes an extra file beside it. The file is the
/// first in sorted order that is not `part.jsonc`, so the part still parses. Returns whether
/// a file was there to edit.
fn edit_part(part: &Path) -> bool {
    let files = files_in(part);
    let Some(file) = files
        .iter()
        .find(|path| path.file_name().is_some_and(|name| name != "part.jsonc"))
        .or(files.first())
    else {
        return false;
    };
    let mut bytes = fs::read(file).unwrap();
    bytes.push(0);
    fs::write(file, bytes).unwrap();
    fs::write(part.join("notes.txt"), "mine").unwrap();
    true
}

/// Removes one installed part, edits a file of a sealed copy the reader still compares with
/// its lock line, and removes `firmware/` when `install` wrote it. Each flag is whether that
/// break can produce its finding. `lack` says why it cannot.
fn break_install(dir: &Path) -> [(&'static str, bool, &'static str); 3] {
    let parts = part_folders(dir);
    let missing = if let Some(part) = parts.first() {
        fs::remove_dir_all(part).unwrap();
        true
    } else {
        false
    };
    let (modified, modified_lack) = if parts.is_empty() {
        (false, "no part folder")
    } else if parts.len() == 1 {
        (false, "a single part")
    } else {
        match sealed_to_edit(dir, &parts) {
            Ok(part) => {
                if edit_part(part) {
                    (true, "")
                } else {
                    (false, "no file in the sealed part")
                }
            }
            Err(lack) => (false, lack),
        }
    };
    let firmware = dir.join("firmware");
    let firmware_missing = if firmware.is_dir() {
        fs::remove_dir_all(&firmware).unwrap();
        true
    } else {
        false
    };
    [
        ("PartMissing", missing, "no part folder"),
        ("PartModified", modified, modified_lack),
        ("FirmwareMissing", firmware_missing, "no firmware folder"),
    ]
}

fn finds(block: &str, reading: &str, code: &str, present: bool, lack: &str) {
    if present {
        assert!(reading.contains(code), "{block} finds {code}: {reading}");
    } else {
        assert!(
            !reading.contains(code),
            "{block} has {lack}, so it does not find {code}: {reading}"
        );
    }
}

/// A missing part, an edited file and an extra file in a sealed copy, and no firmware, each
/// found the same way in memory as on disk, and so is the `open` reply. The part removed is the
/// first folder `install` wrote. The part edited is a sealed copy the reader still compares with
/// its lock line. A Block with no part folder, a single part, no such sealed copy, or no firmware
/// folder says so, and the read still has to show every finding that Block can produce.
#[test]
fn a_broken_install_finds_the_same_from_memory() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    for block in blocks() {
        let block = block.as_str();
        let dir = install(&base, block);
        let broken = break_install(&dir);
        let found = same_reading_and_open(&base, block);
        for (code, present, lack) in broken {
            finds(block, &found[0], code, present, lack);
        }
    }
}

/// `part.jsonc` as JSON. A part file may use comments and trailing commas.
fn jsonc(path: &Path) -> Option<serde_json::Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&strip_trailing_commas(&strip_jsonc(&text))).ok()
}

/// Drops `//` and `/* */` comments. A string is copied through, so a comment mark inside one stays.
fn strip_jsonc(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut string = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            string = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn strip_trailing_commas(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut string = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            string = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == b',' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn folder_name(path: &Path) -> &str {
    path.file_name().unwrap().to_str().unwrap()
}

/// Ids on `parts.lock`. The reader compares a sealed copy with its line.
fn lock_ids(dir: &Path) -> BTreeSet<String> {
    let Ok(text) = fs::read_to_string(dir.join("parts.lock")) else {
        return BTreeSet::new();
    };
    text.lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect()
}

/// Catalog ids named by `parts` in a scene or a part file.
fn named_parts(value: &serde_json::Value) -> Vec<String> {
    let Some(parts) = value.get("parts").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    parts
        .values()
        .filter_map(|inst| inst.get("type").and_then(serde_json::Value::as_str))
        .filter(|ty| orbe_scene::is_id(ty))
        .map(str::to_string)
        .collect()
}

/// A sealed copy has a model, or no parts of its own. An unsealed board has `parts` and no model,
/// and an edit of one is not `PartModified`.
fn is_sealed_copy(part: &Path) -> bool {
    let Some(value) = jsonc(&part.join("part.jsonc")) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.contains_key("model") || !obj.contains_key("parts")
}

fn has_other_file(part: &Path) -> bool {
    files_in(part)
        .iter()
        .any(|path| path.file_name().is_some_and(|name| name != "part.jsonc"))
}

/// Parts the reader still loads once `removed` is gone. A part only the removed folder named
/// is not compared with its lock line.
fn still_loaded(dir: &Path, removed: &str) -> BTreeSet<String> {
    let mut children = BTreeMap::new();
    if let Ok(entries) = fs::read_dir(dir.join("parts")) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name == removed {
                continue;
            }
            let Some(value) = jsonc(&path.join("part.jsonc")) else {
                continue;
            };
            children.insert(name.to_string(), named_parts(&value));
        }
    }
    let mut stack = jsonc(&dir.join("scene.jsonc"))
        .map(|value| named_parts(&value))
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    while let Some(id) = stack.pop() {
        if id == removed || !seen.insert(id.clone()) {
            continue;
        }
        if let Some(kids) = children.get(&id) {
            stack.extend(kids.iter().cloned());
        }
    }
    seen
}

/// The sealed copy to edit, or why this Block has none the reader would still compare.
fn sealed_to_edit<'a>(dir: &Path, parts: &'a [PathBuf]) -> Result<&'a Path, &'static str> {
    let loaded = still_loaded(dir, folder_name(&parts[0]));
    let locked = lock_ids(dir);
    let mut saw = false;
    for part in parts.iter().skip(1) {
        let name = folder_name(part);
        if !loaded.contains(name) || !locked.contains(name) || !is_sealed_copy(part) {
            continue;
        }
        saw = true;
        if has_other_file(part) {
            return Ok(part);
        }
    }
    Err(if saw {
        "no file in the sealed part"
    } else {
        "no sealed part"
    })
}

fn messages(files: &Files, scene: &Path) -> Vec<String> {
    match orbe_scene::read(files, scene) {
        Ok(_) => vec!["read ok".to_string()],
        Err(found) => found.into_iter().map(|d| d.message).collect(),
    }
}

fn memory_of(root: &Path) -> Files {
    let mut files = Vec::new();
    walk(root, root, &mut files);
    Files::memory(files)
}

/// Extra files in a sealed copy are named in sorted order, from disk and from memory.
#[test]
fn extra_files_in_a_sealed_copy_are_named_in_the_same_order() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let block = "nano-button-led";
    let dir = install(&base, block);
    let part = dir.join("parts/led-5mm");
    fs::write(part.join("z-extra.txt"), "z").unwrap();
    fs::write(part.join("a-extra.txt"), "a").unwrap();
    let scene = dir.join("scene.jsonc");
    let prefix = format!("{}/", base.display());
    let disk: Vec<String> = messages(&Files::disk(), &scene)
        .into_iter()
        .map(|text| text.replace(&prefix, ""))
        .collect();
    let memory = messages(
        &memory(&base, block),
        &Path::new("blocks").join(block).join("scene.jsonc"),
    );
    assert_eq!(disk, memory, "{block}");
    let message = disk
        .iter()
        .find(|line| line.contains("a-extra.txt"))
        .unwrap_or_else(|| panic!("{disk:#?}"));
    let a = message.find("`a-extra.txt`").expect(message);
    let z = message.find("`z-extra.txt`").expect(message);
    assert!(a < z, "{message}");
}

/// `./missing/../mine.jsonc` is not a part, from disk and from memory.
#[test]
fn a_missing_parent_is_not_a_part_from_memory_either() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    fs::write(
        root.join("scene.jsonc"),
        "{ \"format\": 1, \"parts\": { \"m\": { \"type\": \"./missing/../mine.jsonc\" } } }\n",
    )
    .unwrap();
    fs::write(root.join("mine.jsonc"), "{}\n").unwrap();
    let disk = messages(&Files::disk(), &root.join("scene.jsonc"));
    let memory = messages(&memory_of(&root), Path::new("scene.jsonc"));
    assert_eq!(disk, memory);
    assert_eq!(
        disk,
        vec!["`./missing/../mine.jsonc` is not a part file or a folder holding part.jsonc"]
    );
}

/// `./mine.jsonc/` is not a part, from disk and from memory.
#[test]
fn a_trailing_slash_on_a_part_file_is_not_a_part_from_memory_either() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    fs::write(
        root.join("scene.jsonc"),
        "{ \"format\": 1, \"parts\": { \"m\": { \"type\": \"./mine.jsonc/\" } } }\n",
    )
    .unwrap();
    fs::write(root.join("mine.jsonc"), "{}\n").unwrap();
    let disk = messages(&Files::disk(), &root.join("scene.jsonc"));
    let memory = messages(&memory_of(&root), Path::new("scene.jsonc"));
    assert_eq!(disk, memory);
    assert_eq!(
        disk,
        vec!["`./mine.jsonc/` is not a part file or a folder holding part.jsonc"]
    );
}
