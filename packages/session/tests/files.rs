//! Each Block, read into a set of files in memory, reads, lints and runs its tests as it does
//! from disk, and so does each Block installed into a project. Paths in memory start from the
//! repo, so a disk path shows the same once the repo's folder is taken off it. A Block is a
//! folder under `blocks/` that holds `block.jsonc`.

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

/// Removes one installed part, edits a file of a part the reader reports as `PartModified`,
/// and removes `firmware/` when `install` wrote it. Each flag is whether that break can
/// produce its finding. `lack` says why it cannot.
fn break_install(dir: &Path) -> [(&'static str, bool, &'static str); 3] {
    let parts = part_folders(dir);
    let missing = if let Some(part) = parts.first() {
        fs::remove_dir_all(part).unwrap();
        true
    } else {
        false
    };
    let (modified, modified_lack) = edit_until_modified(dir, &parts);
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

/// A missing part, an edited file and an extra file in a part the reader reports as
/// `PartModified`, and no firmware, each found the same way in memory as on disk, and so is
/// the `open` reply. The part removed is the first folder `install` wrote. The part edited is
/// the first remaining folder for which the reader reports `PartModified`; a folder it does
/// not is put back and the next is tried. A Block with no part folder, a single part, no such
/// folder, or no firmware folder says so, and the read still has to show every finding that
/// Block can produce.
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

fn folder_name(path: &Path) -> &str {
    path.file_name().unwrap().to_str().unwrap()
}

/// Every file under `part`, so an edit the reader does not report can be put back.
fn saved_files(part: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    files_in(part)
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

fn put_back(part: &Path, saved: &[(PathBuf, Vec<u8>)]) {
    for path in files_in(part) {
        if !saved.iter().any(|(kept, _)| kept == &path) {
            fs::remove_file(path).unwrap();
        }
    }
    for (path, bytes) in saved {
        fs::write(path, bytes).unwrap();
    }
}

/// The reader reports `PartModified` for `id` in this installed project.
fn reader_reports_modified(dir: &Path, id: &str) -> bool {
    let scene = dir.join("scene.jsonc");
    let Err(found) = orbe_scene::read(&Files::disk(), &scene) else {
        return false;
    };
    let named = format!("`{id}`");
    found
        .iter()
        .any(|d| d.code == orbe_scene::Code::PartModified && d.message.contains(&named))
}

/// Edits the first remaining folder for which the reader reports `PartModified`. The folders
/// are the ones `install` wrote, in sorted order, after the one this test removed. A folder
/// the reader does not report that way is put back, and the next is tried.
fn edit_until_modified(dir: &Path, parts: &[PathBuf]) -> (bool, &'static str) {
    if parts.is_empty() {
        return (false, "no part folder");
    }
    if parts.len() == 1 {
        return (false, "a single part");
    }
    let mut edited = false;
    for part in parts.iter().skip(1) {
        let saved = saved_files(part);
        if !edit_part(part) {
            continue;
        }
        edited = true;
        if reader_reports_modified(dir, folder_name(part)) {
            return (true, "");
        }
        put_back(part, &saved);
    }
    (
        false,
        if edited {
            "no sealed part"
        } else {
            "no file in the sealed part"
        },
    )
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
