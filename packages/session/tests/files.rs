//! Each example, read into a set of files in memory, reads, lints and runs its tests as it does
//! from disk. Paths in memory start from the repo, so a disk path shows the same once the repo's
//! folder is taken off it.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_session::{Files, test};
use tempfile::TempDir;

const EXAMPLES: [&str; 2] = ["first-target", "uno-button"];
const FIRMWARE: &str = "packages/engine/tests/firmware/sketch/button.hex";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The example's folder and the firmware it names, by their paths from `base`.
fn memory(base: &Path, example: &str) -> Files {
    let mut files = Vec::new();
    walk(base, &base.join("examples").join(example), &mut files);
    walk(base, &base.join(FIRMWARE), &mut files);
    Files::memory(files)
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

/// What the reader and `orbe test` make of one example, as text.
fn outcome(files: &Files, base: &Path, example: &str) -> Vec<String> {
    let dir = base.join("examples").join(example);
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
    let tests = repo().join("examples").join(example).join("tests");
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

fn same_both_ways(base: &Path, example: &str) -> Vec<String> {
    let prefix = format!("{}/", base.display());
    let disk: Vec<String> = outcome(&Files::disk(), base, example)
        .into_iter()
        .map(|text| text.replace(&prefix, ""))
        .collect();
    let memory = outcome(&memory(base, example), Path::new(""), example);
    assert_eq!(disk.len(), memory.len(), "{example}");
    for (disk, memory) in disk.iter().zip(&memory) {
        assert_eq!(disk, memory, "{example}");
    }
    disk
}

#[test]
fn each_example_reads_and_runs_the_same_from_memory() {
    for example in EXAMPLES {
        let found = same_both_ways(&repo(), example);
        assert!(found[0].starts_with('{'), "{example} reads: {}", found[0]);
        assert!(found[1].starts_with("Ok("), "{example} lints: {}", found[1]);
        assert!(found[2].ends_with(".test passed"), "{example}: {found:#?}");
    }
}

/// A missing part, edited and extra files in a sealed copy, and no firmware, each found the same
/// way in memory as on disk.
#[test]
fn a_broken_example_finds_the_same_from_memory() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    for example in EXAMPLES {
        let dir = base.join("examples").join(example);
        copy(&repo().join("examples").join(example), &dir);
        fs::remove_dir_all(dir.join("parts/resistor-0603")).ok();
        fs::remove_dir_all(dir.join("parts/resistor-axial")).ok();
        let glb = dir.join("parts/led-5mm/led-5mm.glb");
        let mut bytes = fs::read(&glb).unwrap();
        bytes.push(0);
        fs::write(&glb, bytes).unwrap();
        fs::write(dir.join("parts/tactile-switch-6mm/notes.txt"), "mine").unwrap();
        let found = same_both_ways(&base, example);
        for what in ["PartMissing", "PartModified", "FirmwareMissing"] {
            assert!(
                found[0].contains(what),
                "{example} finds {what}: {}",
                found[0]
            );
        }
    }
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}
