use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::*;
use crate::project::folder_hash;

fn ask(ids: &[&str], overwrite: bool) -> Request {
    Request {
        ids: ids.iter().map(|id| (*id).to_string()).collect(),
        overwrite,
        diff: false,
        relock: false,
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("scene.jsonc"), "{}\n").unwrap();
    dir
}

fn text(reply: &Reply) -> String {
    reply.text()
}

/// Every entry under `root`: a file's bytes, a link's target, or a folder.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap();
                out.insert(rel, format!("link {}", target.display()).into_bytes());
            } else if meta.is_dir() {
                out.insert(rel, b"folder".to_vec());
                walk(root, &path, out);
            } else {
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Each lock line's hash is its folder's, and each folder under `parts/` has a line.
fn assert_lock_agrees(root: &Path) {
    let lock = fs::read_to_string(root.join("parts.lock")).unwrap();
    let mut listed = Vec::new();
    for line in lock.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let hash = folder_hash(&root.join("parts").join(fields[0])).unwrap();
        assert_eq!(hash, fields[3], "{line}");
        listed.push(fields[0].to_string());
    }
    let mut folders: Vec<String> = fs::read_dir(root.join("parts"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    folders.sort();
    assert_eq!(folders, listed);
}

fn edit(root: &Path, id: &str) {
    let path = root.join("parts").join(id).join("part.jsonc");
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(b"// my edit\n");
    fs::write(&path, bytes).unwrap();
}

/// Two edited copies, `led-5mm` and `resistor-axial`. The add replaces both and adds
/// `tactile-switch-6mm`, so it moves folders aside, places three, and rewrites the lock.
fn edited_project() -> tempfile::TempDir {
    let dir = project();
    run(dir.path(), &ask(&["led-5mm", "resistor-axial"], false)).unwrap();
    edit(dir.path(), "led-5mm");
    edit(dir.path(), "resistor-axial");
    dir
}

const REPLACE: &[&str] = &["led-5mm", "resistor-axial", "tactile-switch-6mm"];

fn assert_replaced(root: &Path) {
    for id in REPLACE {
        let bytes = fs::read(root.join("parts").join(id).join("part.jsonc")).unwrap();
        assert!(
            !String::from_utf8(bytes).unwrap().contains("my edit"),
            "{id}"
        );
    }
    assert_lock_agrees(root);
    assert!(!root.join("parts").join(txn::STAGE).exists());
}

#[test]
fn run_copies_a_real_part_and_a_second_run_keeps_the_bytes() {
    let dir = project();
    let reply = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
    assert!(
        text(&reply).contains("wrote parts/led-5mm 0.1.0"),
        "{}",
        text(&reply)
    );
    let before = tree(dir.path());
    let again = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
    assert_eq!(
        text(&again),
        "skipped parts/led-5mm 0.1.0\nparts.lock unchanged"
    );
    assert_eq!(tree(dir.path()), before);
    assert_lock_agrees(dir.path());
}

#[test]
fn a_failure_at_any_step_rolls_every_move_back() {
    let mut step = 0;
    loop {
        let dir = edited_project();
        let before = tree(dir.path());
        let result = run_with(dir.path(), &ask(REPLACE, true), &mut |at| {
            (at == step).then_some(Inject::Fail)
        });
        match result {
            Err(error) => {
                let message = error.to_string();
                assert!(message.contains("injected failure"), "{step}: {message}");
                assert!(
                    message.contains("nothing was written") || message.contains("rolled back"),
                    "{step}: {message}"
                );
                assert_eq!(tree(dir.path()), before, "step {step}: {message}");
            }
            Ok(reply) if text(&reply).contains("stay in parts/.orbe-add") => {
                let next = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
                assert!(
                    text(&next).contains("finished an interrupted add, and deleted the copies it replaced: parts/led-5mm, parts/resistor-axial"),
                    "{}",
                    text(&next)
                );
                assert_replaced(dir.path());
            }
            Ok(reply) => {
                assert!(step > 5, "only {step} steps");
                assert!(
                    text(&reply)
                        .contains("wrote parts/led-5mm 0.1.0, replacing 0.1.0 with local edits"),
                    "{}",
                    text(&reply)
                );
                assert_replaced(dir.path());
                return;
            }
        }
        step += 1;
    }
}

#[test]
fn a_rollback_names_what_it_put_back() {
    let dir = edited_project();
    let before = tree(dir.path());
    let fail_at_third_place = |at: usize| (at == 8).then_some(Inject::Fail);
    let mut fault = fail_at_third_place;
    let error = run_with(dir.path(), &ask(REPLACE, true), &mut fault)
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "injected failure at Place(2)\nrolled back, so the project is as it was: put back parts/resistor-axial, put back parts/led-5mm"
    );
    assert_eq!(tree(dir.path()), before);
}

#[test]
fn a_kill_at_any_step_is_rolled_back_or_finished_by_the_next_run() {
    let mut step = 0;
    loop {
        let dir = edited_project();
        let before = tree(dir.path());
        let result = run_with(dir.path(), &ask(REPLACE, true), &mut |at| {
            (at == step).then_some(Inject::Kill)
        });
        let Err(_) = result else {
            assert!(step > 5, "only {step} steps");
            return;
        };
        let lines = txn::recover(dir.path()).unwrap();
        let said = lines.join("\n");
        if said.starts_with("finished an interrupted add") {
            assert_replaced(dir.path());
        } else {
            assert_eq!(tree(dir.path()), before, "step {step}: {said}");
            if step >= 4 {
                assert!(
                    said.contains("rolled back an interrupted add"),
                    "{step}: {said}"
                );
            }
            if step >= 6 {
                assert!(said.contains("put back parts/led-5mm"), "{step}: {said}");
            }
        }
        assert!(txn::recover(dir.path()).unwrap().is_empty(), "{step}");
        step += 1;
    }
}

#[test]
fn the_next_add_says_it_rolled_back_an_interrupted_one() {
    let dir = edited_project();
    let killed = run_with(dir.path(), &ask(REPLACE, true), &mut |at| {
        (at == 6).then_some(Inject::Kill)
    });
    assert!(killed.is_err());
    let reply = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
    assert!(
        text(&reply).starts_with("error[part_differs]: nothing was written"),
        "{}",
        text(&reply)
    );
    assert!(
        text(&reply).contains(
            "rolled back an interrupted add: put back parts/resistor-axial, put back parts/led-5mm"
        ),
        "{}",
        text(&reply)
    );
    assert!(
        fs::read_to_string(dir.path().join("parts/led-5mm/part.jsonc"))
            .unwrap()
            .contains("my edit")
    );
}

#[test]
fn diff_leaves_an_interrupted_add_for_the_next_writing_run() {
    let dir = edited_project();
    let killed = run_with(dir.path(), &ask(REPLACE, true), &mut |at| {
        (at == 6).then_some(Inject::Kill)
    });
    assert!(killed.is_err());
    let pending = tree(dir.path());
    let request = Request {
        diff: true,
        ..ask(&["led-5mm"], false)
    };
    let reply = run(dir.path(), &request).unwrap();
    assert!(
        text(&reply).contains("an interrupted add is pending in parts/.orbe-add. --diff leaves it, and the next `orbe add -C "),
        "{}",
        text(&reply)
    );
    assert!(text(&reply).contains("without --diff rolls it back"));
    assert_eq!(tree(dir.path()), pending);
    let reply = run(dir.path(), &ask(&["led-5mm"], true)).unwrap();
    assert!(
        text(&reply).contains("rolled back an interrupted add"),
        "{}",
        text(&reply)
    );
}

#[test]
fn a_file_at_a_part_path_is_kept_unless_overwrite_names_it() {
    let dir = project();
    fs::create_dir(dir.path().join("parts")).unwrap();
    let file = dir.path().join("parts/led-5mm");
    fs::write(&file, "my notes, not a part\n").unwrap();
    let reply = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
    assert_eq!(reply.code(), 1);
    assert!(
        text(&reply).contains("parts/led-5mm is a file, not a part folder"),
        "{}",
        text(&reply)
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "my notes, not a part\n");
    assert!(!dir.path().join("parts.lock").exists());

    let reply = run(dir.path(), &ask(&["led-5mm"], true)).unwrap();
    assert!(
        text(&reply).contains("wrote parts/led-5mm 0.1.0, replacing a file"),
        "{}",
        text(&reply)
    );
    assert_lock_agrees(dir.path());
}

#[test]
fn a_link_at_a_part_path_is_replaced_and_its_target_kept() {
    let dir = project();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(elsewhere.path().join("part.jsonc"), "mine\n").unwrap();
    fs::create_dir(dir.path().join("parts")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("parts/led-5mm")).unwrap();
    let reply = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
    assert!(
        text(&reply).contains("parts/led-5mm is a link, not a part folder"),
        "{}",
        text(&reply)
    );
    let reply = run(dir.path(), &ask(&["led-5mm"], true)).unwrap();
    assert!(
        text(&reply).contains("replacing a link"),
        "{}",
        text(&reply)
    );
    assert_eq!(
        fs::read_to_string(elsewhere.path().join("part.jsonc")).unwrap(),
        "mine\n"
    );
    assert_lock_agrees(dir.path());
}

#[test]
fn a_broken_folder_of_the_users_own_does_not_block_an_add() {
    let dir = project();
    let mine = dir.path().join("parts/my-sensor");
    fs::create_dir_all(&mine).unwrap();
    fs::write(mine.join("part.jsonc"), "{}\n").unwrap();
    std::os::unix::fs::symlink("../../datasheets/sensor.pdf", mine.join("datasheet.pdf")).unwrap();
    let reply = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.0\nwrote parts.lock");
}

#[test]
fn a_folder_that_is_not_a_project_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let reply = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
    assert_eq!(reply.code(), 1);
    assert!(
        text(&reply).starts_with(&format!(
            "error[not_a_project]: `{}` is not an orbe project. it has no scene.jsonc and no parts.lock",
            dir.path().display()
        )),
        "{}",
        text(&reply)
    );
    assert!(!dir.path().join("parts").exists());
}

#[test]
fn parts_on_another_filesystem_still_takes_an_add() {
    let shm = Path::new("/dev/shm");
    let dir = project();
    let Ok(other) = tempfile::tempdir_in(shm) else {
        eprintln!("no /dev/shm here, so the cross-device case cannot be set up");
        return;
    };
    use std::os::unix::fs::MetadataExt;
    let same = fs::metadata(dir.path()).unwrap().dev() == fs::metadata(other.path()).unwrap().dev();
    if same {
        eprintln!("/dev/shm is on the same device as the temp folder here");
        return;
    }
    std::os::unix::fs::symlink(other.path(), dir.path().join("parts")).unwrap();
    let reply = run(dir.path(), &ask(&["led-5mm"], false)).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.0\nwrote parts.lock");
    edit(dir.path(), "led-5mm");
    let reply = run(dir.path(), &ask(&["led-5mm"], true)).unwrap();
    assert!(text(&reply).contains("replacing 0.1.0 with local edits"));
    assert_lock_agrees(dir.path());
}
