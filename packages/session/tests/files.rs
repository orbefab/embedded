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

#[path = "../../scene/tests/common/mod.rs"]
mod layout;

/// The Block installed by `orbe add` into `<base>/blocks/<block>`, so it is no longer a Block.
/// `block.jsonc` stays behind. The parts the scene names are copied, and each part's own
/// parts are nested inside its folder.
fn install(base: &Path, block: &str) -> PathBuf {
    let src = repo().join("blocks").join(block);
    let dir = base.join("blocks").join(block);
    fs::create_dir_all(&dir).unwrap();
    for entry in fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .is_some_and(|name| name == orbe_scene::BLOCK)
        {
            continue;
        }
        let target = dir.join(path.file_name().unwrap());
        if path.is_dir() {
            layout::copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
    let scene = fs::read_to_string(src.join("scene.jsonc")).unwrap_or_default();
    let catalog = repo().join("parts");
    for id in scene_part_ids(&scene) {
        let from = catalog.join(&id);
        if !from.join("part.jsonc").is_file() {
            continue;
        }
        let dest = dir.join("parts").join(&id);
        layout::copy_dir(&from, &dest);
        layout::nest_uses(&catalog, &dest, &mut Vec::new());
    }
    dir
}

/// The catalog ids a scene's `parts` name by `"type"`. A path and a repeated id are skipped.
fn scene_part_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("\"type\"") {
        rest = &rest[at + "\"type\"".len()..];
        let Some(colon) = rest.find(':') else { break };
        let after = rest[colon + 1..].trim_start();
        if !after.starts_with('"') {
            continue;
        }
        let body = &after[1..];
        let Some(end) = body.find('"') else { break };
        let id = &body[..end];
        if orbe_scene::is_id(id) && !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
        rest = &body[end + 1..];
    }
    ids
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
                "{}\n{} {:?} {} {:?}\n{}",
                netlist.json,
                netlist.parts,
                netlist.nets,
                netlist.ground,
                netlist.refs,
                serde_json::to_string(&netlist.view).unwrap()
            ),
            Err(found) => format!("{found:#?}"),
        },
        format!("{:#?}", orbe_scene::lint(files, &scene)),
    ]
}

/// What `orbe test` makes of one Block. The test text is the Block's own, in the repo.
/// A missing `tests/` directory contributes nothing, and so does an entry that is not a file.
fn played(files: &Files, base: &Path, block: &str) -> Vec<String> {
    let dir = base.join("blocks").join(block);
    let mut out = Vec::new();
    for path in test_files(block) {
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

/// The `.test` files directly in the Block's `tests/` folder, in sorted order. A missing directory is
/// none, and so is an entry that is not a file.
fn test_files(block: &str) -> Vec<PathBuf> {
    let tests = repo().join("blocks").join(block).join("tests");
    let Ok(entries) = fs::read_dir(&tests) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "test"))
        .collect();
    paths.sort();
    paths
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

fn same_both_ways(base: &Path, block: &str) -> Vec<String> {
    let folder = base.join("blocks").join(block);
    if folder.join(orbe_scene::BLOCK).is_file() {
        // A Block reads the `parts/` beside `blocks/`. Memory holds those files at the same
        // absolute paths disk reads, so the two readings name the same files.
        return same_lines(
            block,
            outcome(&Files::disk(), base, block),
            outcome(&memory_with_parts(base, block), base, block),
        );
    }
    same_lines(
        block,
        shown(base, outcome(&Files::disk(), base, block)),
        outcome(&memory(base, block), Path::new(""), block),
    )
}

/// The Block's files and the `parts/` beside `blocks/`, each at its absolute path.
fn memory_with_parts(base: &Path, block: &str) -> Files {
    let mut files = Vec::new();
    let folder = base.join("blocks").join(block);
    walk_abs(&folder, &mut files);
    if let Some(parts) = folder
        .parent()
        .and_then(|dir| dir.parent())
        .map(|dir| dir.join("parts"))
        && parts.is_dir()
    {
        walk_abs(&parts, &mut files);
    }
    Files::memory(files)
}

fn walk_abs(path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            walk_abs(&entry.unwrap().path(), out);
        }
    } else if path.is_file() {
        out.push((path.to_path_buf(), fs::read(path).unwrap()));
    }
}

/// The read starts with `{` and the lint with `Ok(`. The first played line passed. A Block
/// with no test file has nothing to play, and the assertion names that lack.
fn reads_and_passes(found: &[String], block: &str) {
    assert!(found[0].starts_with('{'), "{block} reads: {}", found[0]);
    assert!(found[1].starts_with("Ok("), "{block} lints: {}", found[1]);
    if test_files(block).is_empty() {
        assert!(
            found.get(2).is_none(),
            "{block} has no test file, so there is no play to compare: {found:#?}"
        );
        return;
    }
    assert!(
        found
            .get(2)
            .is_some_and(|line| line.ends_with(".test passed")),
        "{block}: {found:#?}"
    );
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

/// A Block handed its catalog as part folders, as a browser is, reads and runs as it does from
/// the `parts/` beside `blocks/`. The scene hash names the files that were read, and a handed-in
/// catalog records those files at `parts/<id>/` inside the Block, so that hash is set aside.
#[test]
fn each_block_reads_and_runs_the_same_from_a_given_catalog() {
    for block in blocks() {
        let block = block.as_str();
        let disk = shown(&repo(), outcome(&Files::disk(), &repo(), block));
        let given = memory(&repo(), block).with_catalog(given_catalog(""));
        let from_catalog = outcome(&given, Path::new(""), block);
        assert_eq!(
            scrub_scene_hash(&from_catalog),
            scrub_scene_hash(&disk),
            "{block}"
        );
    }
}

/// `scene_hash` covers the paths of the files that were read. A handed-in catalog and a Block
/// on disk name those files differently, and the rest of the reading is what this compares.
fn scrub_scene_hash(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let mut out = String::new();
            let mut rest = line.as_str();
            let key = "\"scene_hash\":\"";
            while let Some(at) = rest.find(key) {
                out.push_str(&rest[..at + key.len()]);
                out.push_str("sha256:0");
                rest = &rest[at + key.len()..];
                let Some(end) = rest.find('"') else { break };
                rest = &rest[end..];
            }
            out.push_str(rest);
            out
        })
        .collect()
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
/// first in sorted order that is not `part.jsonc`, so the part still parses. When `part.jsonc`
/// is the only file, the extra file is the whole edit and `part.jsonc` is left as it is. The
/// reader decides whether the edit is `PartModified`. Returns whether the part had a file.
fn edit_part(part: &Path) -> bool {
    let files = files_in(part);
    if files.is_empty() {
        return false;
    }
    if let Some(file) = files
        .iter()
        .find(|path| path.file_name().is_some_and(|name| name != "part.jsonc"))
    {
        let mut bytes = fs::read(file).unwrap();
        bytes.push(0);
        fs::write(file, bytes).unwrap();
    }
    fs::write(part.join("notes.txt"), "mine").unwrap();
    true
}

/// Removes one installed part, edits another copy, and removes `build/` when `install`
/// wrote it. The scene names its image under `build/`, so that removal is `FirmwareMissing`.
/// An edit is the project's, so the read does not report it. Each flag is whether that break
/// can produce its finding. `lack` says why it cannot.
fn break_install(dir: &Path) -> [(&'static str, bool, &'static str); 3] {
    let parts = part_folders(dir);
    let missing = if let Some(part) = parts.first() {
        fs::remove_dir_all(part).unwrap();
        true
    } else {
        false
    };
    if let Some(part) = parts.get(1) {
        edit_part(part);
    }
    let image = dir.join("build");
    let firmware_missing = if image.is_dir() {
        fs::remove_dir_all(&image).unwrap();
        true
    } else {
        false
    };
    [
        ("PartMissing", missing, "no part folder"),
        ("PartModified", false, "an edited copy is the project's"),
        ("FirmwareMissing", firmware_missing, "no build folder"),
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

/// A missing part, an edited copy, and no firmware, each found the same way in memory as on
/// disk, and so is the test transcript. The part removed is the first folder `install` wrote.
/// The next folder is edited, and that edit is not a finding. A Block with no part folder or
/// no image under `build/` says so, and the read still has to show every finding that Block can
/// produce. The transcript is the Block's own test file: its passed or failed line, each
/// source line's number, and every reply. The read has already failed, so the run does not
/// start.
#[test]
fn a_broken_install_finds_the_same_from_memory() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    for block in blocks() {
        let block = block.as_str();
        let dir = install(&base, block);
        let broken = break_install(&dir);
        let found = same_both_ways(&base, block);
        for (code, present, lack) in broken {
            finds(block, &found[0], code, present, lack);
        }
    }
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
