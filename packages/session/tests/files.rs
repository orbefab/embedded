//! Each Block, read into a set of files in memory, reads, lints and runs its tests as it does
//! from disk, and so does each Block installed into a project. Paths in memory start from the
//! repo, so a disk path shows the same once the repo's folder is taken off it.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_session::{Files, test};
use tempfile::TempDir;

const BLOCKS: [&str; 4] = [
    "nano-button-led",
    "uno-button-led",
    "nano-led-fade",
    "nano-light-comparator",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
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

/// What the reader and `orbe test` make of one Block, as text.
fn outcome(files: &Files, base: &Path, block: &str) -> Vec<String> {
    let dir = base.join("blocks").join(block);
    let scene = dir.join("scene.jsonc");
    let mut out = vec![
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
    ];
    let tests = repo().join("blocks").join(block).join("tests");
    for entry in fs::read_dir(&tests).unwrap() {
        let path = entry.unwrap().path();
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

fn same_both_ways(base: &Path, block: &str) -> Vec<String> {
    let prefix = format!("{}/", base.display());
    let disk: Vec<String> = outcome(&Files::disk(), base, block)
        .into_iter()
        .map(|text| text.replace(&prefix, ""))
        .collect();
    let memory = outcome(&memory(base, block), Path::new(""), block);
    assert_eq!(disk.len(), memory.len(), "{block}");
    for (disk, memory) in disk.iter().zip(&memory) {
        assert_eq!(disk, memory, "{block}");
    }
    disk
}

fn reads_and_passes(found: &[String], block: &str) {
    assert!(found[0].starts_with('{'), "{block} reads: {}", found[0]);
    assert!(found[1].starts_with("Ok("), "{block} lints: {}", found[1]);
    assert!(found[2].ends_with(".test passed"), "{block}: {found:#?}");
}

#[test]
fn each_block_reads_and_runs_the_same_from_memory() {
    for block in BLOCKS {
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
    for block in BLOCKS {
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
    for block in BLOCKS {
        install(&base, block);
        let found = same_both_ways(&base, block);
        reads_and_passes(&found, block);
        let ours = same_both_ways(&repo(), block);
        let netlist = |found: &[String]| found[0].rsplit_once('\n').unwrap().0.to_string();
        assert_eq!(
            netlist(&found),
            netlist(&ours),
            "{block}: the installed netlist"
        );
    }
}

/// A missing part, edited and extra files in a sealed copy, and no firmware, each found the same
/// way in memory as on disk.
#[test]
fn a_broken_install_finds_the_same_from_memory() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    for block in BLOCKS {
        let dir = install(&base, block);
        fs::remove_dir_all(dir.join("parts/resistor-0603")).unwrap();
        fs::remove_dir_all(dir.join("parts/resistor-axial")).unwrap();
        let glb = dir.join("parts/led-5mm/led-5mm.glb");
        let mut bytes = fs::read(&glb).unwrap();
        bytes.push(0);
        fs::write(&glb, bytes).unwrap();
        fs::write(dir.join("parts/led-5mm/notes.txt"), "mine").unwrap();
        fs::remove_dir_all(dir.join("firmware")).unwrap();
        let found = same_both_ways(&base, block);
        for what in ["PartMissing", "PartModified", "FirmwareMissing"] {
            assert!(
                found[0].contains(what),
                "{block} finds {what}: {}",
                found[0]
            );
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
