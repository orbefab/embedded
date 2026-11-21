//! `orbe add <block>` installs a Block into a project. It copies each part the scene names into
//! `parts/` and lists them in `parts.lock`, then writes the scene, firmware, tests and README.

#![cfg(unix)]

mod live;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use live::{common, orbe};

const BLOCKS: [&str; 4] = [
    "nano-button-led",
    "uno-button-led",
    "nano-led-fade",
    "nano-light-comparator",
];

/// Every file below `dir` with its bytes, by its path from `dir`.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
                files.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    files
}

#[test]
fn a_block_installs_into_an_empty_folder_and_passes_there() {
    for id in BLOCKS {
        let project = tempfile::tempdir().unwrap();
        let dir = project.path();
        let run = orbe(dir, &["add", id]);
        assert_eq!(run.code, 0, "{id}: {}{}", run.stdout, run.stderr);
        assert!(
            run.stdout.ends_with(&format!(
                "installed the Block `{id}`. run `orbe check scene.jsonc` and `orbe test`\n"
            )),
            "{}",
            run.stdout
        );

        let block = common::repo().join("blocks").join(id);
        let installed = tree(dir);
        for (rel, bytes) in tree(&block) {
            if rel == "block.jsonc" {
                assert!(
                    !installed.contains_key(&rel),
                    "{id}: block.jsonc stays in the repo"
                );
            } else {
                assert_eq!(installed.get(&rel), Some(&bytes), "{id}: {rel}");
            }
        }
        let lock = fs::read_to_string(dir.join("parts.lock")).unwrap();
        let locked: Vec<&str> = lock.lines().filter_map(|l| l.split(' ').next()).collect();
        let mut copied: Vec<String> = fs::read_dir(dir.join("parts"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        copied.sort();
        assert_eq!(locked, copied, "{id}: parts.lock lists each copy");
        assert!(copied.iter().any(|part| part == "led-5mm"), "{id}");
        for part in &copied {
            assert_eq!(
                tree(&dir.join("parts").join(part)),
                tree(&common::repo().join("parts").join(part)),
                "{id}: parts/{part} is the catalog's"
            );
        }

        let check = orbe(dir, &["check", "scene.jsonc"]);
        assert_eq!(check.code, 0, "{id}: {}", check.stdout);
        let test = orbe(dir, &["test"]);
        assert!(
            test.stdout.ends_with("1 passed, 0 failed\n"),
            "{id}: {}",
            test.stdout
        );

        let again = orbe(dir, &["add", id]);
        assert_eq!(again.code, 0, "{id}: {}", again.stdout);
        assert!(!again.stdout.contains("wrote"), "{id}: {}", again.stdout);
        assert_eq!(
            tree(dir),
            installed,
            "{id}: a second install changes nothing"
        );
    }
}

#[test]
fn a_block_json_names_the_same_next_as_the_text() {
    let text = orbe(
        tempfile::tempdir().unwrap().path(),
        &["add", "nano-button-led"],
    );
    assert_eq!(text.code, 0, "{}", text.stdout);
    let next = text
        .stdout
        .lines()
        .find(|line| line.starts_with("installed the Block "))
        .unwrap()
        .split_once(". ")
        .unwrap()
        .1;
    let envelope = orbe(
        tempfile::tempdir().unwrap().path(),
        &["add", "--json", "nano-button-led"],
    )
    .json();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["next"], next);
}

#[test]
fn a_second_add_writes_the_scene_a_stopped_install_left_out() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let first = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(first.code, 0, "{}", first.stdout);
    fs::remove_file(dir.join("scene.jsonc")).unwrap();
    let again = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(again.code, 0, "{}{}", again.stdout, again.stderr);
    assert!(
        again.stdout.contains("wrote scene.jsonc\n"),
        "{}",
        again.stdout
    );
    assert!(
        again.stdout.contains("kept firmware/button.hex\n"),
        "{}",
        again.stdout
    );
    assert!(
        again.stdout.contains("parts.lock unchanged\n"),
        "{}",
        again.stdout
    );
    assert!(!again.stdout.contains("wrote parts/"), "{}", again.stdout);
    let check = orbe(dir, &["check", "scene.jsonc"]);
    assert_eq!(check.code, 0, "{}", check.stdout);
}

#[test]
fn a_block_refuses_a_folder_with_a_scene_and_writes_nothing() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let installed = orbe(dir, &["add", "uno-button-led"]);
    assert_eq!(installed.code, 0, "{}", installed.stdout);
    let before = tree(dir);

    let run = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stdout,
        "error[scene_exists]: scene.jsonc already holds a scene, and the Block `nano-button-led` brings its own. orbe does not merge scenes. nothing was written\n  hint: install it into an empty folder: `mkdir nano-button-led && orbe add -C nano-button-led nano-button-led`\n"
    );
    assert_eq!(tree(dir), before, "the refusal wrote nothing");

    let envelope = orbe(dir, &["add", "nano-button-led", "--json"]).json();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "scene_exists");
    assert_eq!(tree(dir), before);

    let run = orbe(dir, &["add", "nano-led-fade", "--json"]).json();
    assert_eq!(
        run["error"]["code"], "scene_exists",
        "the scene is named even when the firmware differs too"
    );
    assert_eq!(tree(dir), before);
}

#[test]
fn a_block_refuses_a_file_in_its_way_and_writes_nothing() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir(dir.join("tests")).unwrap();
    fs::write(dir.join("tests/button.test"), "open\n").unwrap();
    let before = tree(dir);
    let run = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(
        run.stdout.starts_with(
            "error[file_exists]: tests/button.test is already there and is not the Block `nano-button-led`'s. nothing was written\n"
        ),
        "{}",
        run.stdout
    );
    assert_eq!(tree(dir), before);
}

/// Files under `dir`, without following a link. A link is recorded as the bytes `link`.
fn shallow(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(dir)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                files.insert(rel, b"link".to_vec());
            } else if meta.is_dir() {
                stack.push(path);
            } else {
                files.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    files
}

fn refuses_the_link(dir: &Path, outside: &Path, link: &str) {
    let before = shallow(outside);
    let project_before = shallow(dir);
    let run = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "error[symlink]: {link} is a link, and orbe add does not write through a link. nothing was written\n  hint: remove the link {link}, then run `orbe add nano-button-led` again\n"
        ),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("kept"), "{}", run.stdout);
    assert_eq!(shallow(outside), before, "the link's target changed");
    assert_eq!(
        shallow(dir),
        project_before,
        "the refusal wrote in the project"
    );
}

#[test]
fn a_block_refuses_a_symlinked_firmware_directory() {
    let outside = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    symlink(outside.path(), project.path().join("firmware")).unwrap();
    refuses_the_link(project.path(), outside.path(), "firmware");
}

#[test]
fn a_block_refuses_a_symlink_at_its_temp_file() {
    let outside = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir(dir.join("firmware")).unwrap();
    let seed = outside.path().join("seed");
    fs::write(&seed, b"SEED").unwrap();
    symlink(&seed, dir.join("firmware/.button.hex.orbe-add")).unwrap();
    refuses_the_link(dir, outside.path(), "firmware/.button.hex.orbe-add");
    assert_eq!(fs::read(&seed).unwrap(), b"SEED");
}

/// A destination that is a link is refused again, even when the target already holds the
/// Block's bytes. The link stays, and the target is not rewritten.
#[test]
fn a_second_add_refuses_a_symlinked_destination() {
    let outside = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir(dir.join("firmware")).unwrap();
    let hex = fs::read(common::repo().join("blocks/nano-button-led/firmware/button.hex")).unwrap();
    let target = outside.path().join("button.hex");
    fs::write(&target, &hex).unwrap();
    symlink(&target, dir.join("firmware/button.hex")).unwrap();
    refuses_the_link(dir, outside.path(), "firmware/button.hex");
    assert_eq!(fs::read(&target).unwrap(), hex);
}

#[test]
fn a_block_refuses_a_symlinked_nested_directory() {
    let outside = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir(dir.join("firmware")).unwrap();
    symlink(outside.path(), dir.join("firmware/button")).unwrap();
    refuses_the_link(dir, outside.path(), "firmware/button");
}

#[test]
fn a_block_installs_when_the_project_path_is_a_symlink() {
    let parent = tempfile::tempdir().unwrap();
    let real = parent.path().join("real");
    fs::create_dir(&real).unwrap();
    symlink(&real, parent.path().join("link")).unwrap();
    let run = orbe(parent.path(), &["add", "-C", "link", "nano-button-led"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(real.join("scene.jsonc").is_file());
    assert!(real.join("firmware/button.hex").is_file());
    assert!(
        fs::symlink_metadata(parent.path().join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_block_replaces_a_regular_temp_file_left_by_a_stopped_install() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir(dir.join("firmware")).unwrap();
    fs::write(dir.join("firmware/.button.hex.orbe-add"), b"OLD").unwrap();
    let run = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!dir.join("firmware/.button.hex.orbe-add").exists());
    let hex = fs::read(common::repo().join("blocks/nano-button-led/firmware/button.hex")).unwrap();
    assert_eq!(fs::read(dir.join("firmware/button.hex")).unwrap(), hex);
    assert!(
        !fs::symlink_metadata(dir.join("firmware/button.hex"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_block_installs_alone() {
    let project = tempfile::tempdir().unwrap();
    for args in [
        &["add", "nano-button-led", "led-5mm"][..],
        &["add", "nano-button-led", "--overwrite"],
        &["add", "nano-button-led@0.1.3"],
    ] {
        let run = orbe(project.path(), args);
        assert_eq!(run.code, 2, "{args:?}: {}{}", run.stdout, run.stderr);
        assert!(
            run.stderr
                .starts_with("error[usage]: `nano-button-led` is a Block"),
            "{}",
            run.stderr
        );
    }
    assert_eq!(fs::read_dir(project.path()).unwrap().count(), 0);
}

#[test]
fn the_core_skill_names_each_block() {
    let skill = fs::read_to_string(common::repo().join("apps/orbe/skills/core.md")).unwrap();
    let ids: Vec<String> = BLOCKS.iter().map(|id| format!("`{id}`")).collect();
    let listed = format!(
        "The Blocks are {}, {}, {} and {}.",
        ids[0], ids[1], ids[2], ids[3]
    );
    assert!(skill.contains(&listed), "{listed}");
    let mut found: Vec<String> = fs::read_dir(common::repo().join("blocks"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    found.sort();
    let mut want = BLOCKS.map(String::from).to_vec();
    want.sort();
    assert_eq!(found, want);
}
