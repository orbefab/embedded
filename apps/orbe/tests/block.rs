//! `orbe add <block>` installs a Block into a project. It copies each part the scene names into
//! `parts/`, then writes the scene, firmware, tests and README.

#![cfg(unix)]

mod live;
#[path = "registry_env.rs"]
mod registry_env;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use live::{common, orbe};

const BLOCKS: [&str; 10] = [
    "devkitc-button-led",
    "nano-7seg-count",
    "nano-button-led",
    "nano-buzzer",
    "uno-button-led",
    "nano-led-fade",
    "nano-light-comparator",
    "nano-neopixel-stick",
    "nano-oled",
    "nano-shift-leds",
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
        let mut copied: Vec<String> = fs::read_dir(dir.join("parts"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|name| dir.join("parts").join(name).join("part.jsonc").is_file())
            .collect();
        copied.sort();
        // The rainbow Block drives a pixel module, the OLED Block a screen module, the buzzer
        // Block a buzzer, the 7-segment Block a display and the DevKitC-1 Block its board's own
        // LED. The others light a 5 mm LED.
        let marker = match id {
            "nano-7seg-count" => "display-7seg-cc",
            "nano-neopixel-stick" => "neopixel-stick-8",
            "nano-oled" => "oled-ssd1306-128x64",
            "devkitc-button-led" => "esp32s3-devkitc-1",
            "nano-buzzer" => "buzzer-12mm",
            _ => "led-5mm",
        };
        assert!(copied.iter().any(|part| part == marker), "{id}: {copied:?}");
        for part in &copied {
            let catalog = tree(&common::repo().join("parts").join(part));
            let got = tree(&dir.join("parts").join(part));
            for (rel, bytes) in &catalog {
                if orbe_scene::is_published(rel) {
                    assert_eq!(got.get(rel), Some(bytes), "{id}: parts/{part}/{rel}");
                } else {
                    assert!(
                        !got.contains_key(rel),
                        "{id}: parts/{part}/{rel} is not copied"
                    );
                }
            }
        }
        let chip = dir
            .join("parts/nano-board/parts/atmega328p-tqfp32/part.jsonc")
            .is_file()
            || dir
                .join("parts/uno-board/parts/atmega328p-dip28/part.jsonc")
                .is_file()
            || dir
                .join("parts/esp32s3-devkitc-1/parts/esp32s3-wroom-1-n16r8/part.jsonc")
                .is_file();
        assert!(chip, "{id}: the board nests its chip");

        let check = orbe(dir, &["check", "scene.jsonc"]);
        assert_eq!(check.code, 0, "{id}: {}", check.stdout);
        let test = orbe(dir, &["test"]);
        assert!(
            test.stdout.ends_with("1 passed, 0 failed\n"),
            "{id}: {}",
            test.stdout
        );

        let again = orbe(dir, &["add", id]);
        assert_eq!(again.code, 1, "{id}: {}{}", again.stdout, again.stderr);
        assert!(
            again.stderr.contains("error[part_exists]"),
            "{id}: {}",
            again.stderr
        );
        assert!(
            again.stderr.contains(&format!("run `orbe add {id}`")),
            "{id}: {}",
            again.stderr
        );
        assert_eq!(
            tree(dir),
            installed,
            "{id}: a second install writes nothing"
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
fn a_second_add_refuses_when_the_part_folders_are_already_there() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let first = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(first.code, 0, "{}", first.stdout);
    fs::remove_file(dir.join("scene.jsonc")).unwrap();
    let again = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(again.code, 1, "{}{}", again.stdout, again.stderr);
    assert!(
        again.stderr.contains("error[part_exists]"),
        "{}",
        again.stderr
    );
    assert!(
        again.stderr.contains("run `orbe add nano-button-led`"),
        "{}",
        again.stderr
    );
    assert!(again.stderr.contains("parts/led-5mm"), "{}", again.stderr);
    assert!(!dir.join("scene.jsonc").exists());
    assert!(!again.stdout.contains("wrote "), "{}", again.stdout);
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
        run.stderr,
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
        run.stderr.starts_with(
            "error[file_exists]: tests/button.test is already there and is not the Block `nano-button-led`'s. nothing was written\n"
        ),
        "{}",
        run.stderr
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
        run.stderr,
        format!(
            "error[symlink]: {link} is a link, and orbe add does not write through a link. nothing was written\n  hint: remove the link {link}, then run `orbe add nano-button-led` again\n"
        ),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("kept"), "{}", run.stderr);
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
fn a_leftover_add_folder_is_removed_and_the_block_installs() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let leftover = dir.join("parts/.add-led-5mm");
    fs::create_dir_all(&leftover).unwrap();
    fs::write(leftover.join("note"), b"OLD").unwrap();
    let run = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!leftover.exists());
    assert!(dir.join("scene.jsonc").is_file());
}

/// A destination that is a link is refused again, even when the target already holds the
/// Block's bytes. The link stays, and the target is not rewritten.
#[test]
fn a_second_add_refuses_a_symlinked_destination() {
    let outside = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir(dir.join("build")).unwrap();
    let hex = fs::read(common::repo().join("blocks/nano-button-led/build/button.ino.hex")).unwrap();
    let target = outside.path().join("button.ino.hex");
    fs::write(&target, &hex).unwrap();
    symlink(&target, dir.join("build/button.ino.hex")).unwrap();
    refuses_the_link(dir, outside.path(), "build/button.ino.hex");
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
    assert!(real.join("build/button.ino.hex").is_file());
    assert!(
        fs::symlink_metadata(parent.path().join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_block_installs_over_a_leftover_staging_directory() {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    fs::create_dir_all(dir.join("parts/.add-nano-board")).unwrap();
    fs::write(dir.join("parts/.add-nano-board/part.jsonc"), b"OLD").unwrap();
    let run = orbe(dir, &["add", "nano-button-led"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!dir.join("parts/.add-nano-board").exists());
    let hex = fs::read(common::repo().join("blocks/nano-button-led/build/button.ino.hex")).unwrap();
    assert_eq!(fs::read(dir.join("build/button.ino.hex")).unwrap(), hex);
}

#[test]
fn a_block_installs_alone() {
    let project = tempfile::tempdir().unwrap();
    let mixed = orbe(project.path(), &["add", "nano-button-led", "led-5mm"]);
    assert_eq!(mixed.code, 2, "{}{}", mixed.stdout, mixed.stderr);
    assert!(
        mixed
            .stderr
            .starts_with("error[usage]: `nano-button-led` is a Block"),
        "{}",
        mixed.stderr
    );
    let versioned = orbe(project.path(), &["add", "nano-button-led@0.1.3"]);
    assert_eq!(
        versioned.code, 2,
        "{}{}",
        versioned.stdout, versioned.stderr
    );
    assert!(
        versioned.stderr.contains("names a version"),
        "{}",
        versioned.stderr
    );
    assert!(
        versioned.stderr.contains("hint: nano-button-led\n"),
        "{}",
        versioned.stderr
    );
    assert_eq!(fs::read_dir(project.path()).unwrap().count(), 0);
}

#[test]
fn each_block_folder_is_in_the_block_list() {
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
