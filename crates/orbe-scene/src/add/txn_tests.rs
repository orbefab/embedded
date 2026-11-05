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

fn add(root: &Path, ids: &[&str], overwrite: bool) -> Result<Reply, RunError> {
    run(Some(root), &ask(ids, overwrite))
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

fn part_file(root: &Path, id: &str) -> String {
    fs::read_to_string(root.join("parts").join(id).join("part.jsonc")).unwrap()
}

fn append_to(path: &Path, line: &str) {
    let mut bytes = fs::read(path).unwrap();
    bytes.extend_from_slice(line.as_bytes());
    fs::write(path, bytes).unwrap();
}

fn edit(root: &Path, id: &str) {
    append_to(
        &root.join("parts").join(id).join("part.jsonc"),
        "// my edit\n",
    );
}

/// Two edited copies, `led-5mm` and `resistor-axial`. The add replaces both and adds
/// `tactile-switch-6mm`, so it moves folders aside, places three, and rewrites the lock.
fn edited_project() -> tempfile::TempDir {
    let dir = project();
    add(dir.path(), &["led-5mm", "resistor-axial"], false).unwrap();
    edit(dir.path(), "led-5mm");
    edit(dir.path(), "resistor-axial");
    dir
}

const REPLACE: &[&str] = &["led-5mm", "resistor-axial", "tactile-switch-6mm"];
const AGAIN: &str = "orbe add tactile-switch-6mm";

fn assert_replaced(root: &Path) {
    for id in REPLACE {
        assert!(!part_file(root, id).contains("my edit"), "{id}");
    }
    assert_lock_agrees(root);
    assert!(!root.join("parts").join(txn::STAGE).exists());
}

/// Runs the replacing add and stops it at the point the hook picks.
fn stop_add(root: &Path, mut pick: impl FnMut(&str) -> Option<Inject>) -> RunError {
    run_with(Some(root), &ask(REPLACE, true), &mut pick)
        .err()
        .expect("the add stopped")
}

fn kill_at(point: &'static str) -> impl FnMut(&str) -> Option<Inject> {
    move |at: &str| at.starts_with(point).then_some(Inject::Kill)
}

fn recover(root: &Path) -> String {
    txn::recover(root, AGAIN, &mut |_| None).unwrap().join("\n")
}

#[test]
fn run_copies_a_real_part_and_a_second_run_keeps_the_bytes() {
    let dir = project();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.0\nwrote parts.lock");
    let before = tree(dir.path());
    let again = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(
        text(&again),
        "skipped parts/led-5mm 0.1.0\nparts.lock unchanged"
    );
    assert_eq!(tree(dir.path()), before);
    assert_lock_agrees(dir.path());
    assert!(!dir.path().join(held::LOCK_FILE).exists());
}

#[test]
fn a_failure_at_any_point_rolls_every_move_back() {
    let mut point = 0;
    loop {
        let dir = edited_project();
        let before = tree(dir.path());
        let mut seen = 0;
        let result = run_with(Some(dir.path()), &ask(REPLACE, true), &mut |_| {
            seen += 1;
            (seen == point + 1).then_some(Inject::Fail)
        });
        match result {
            Err(error) => {
                let message = error.to_string();
                assert!(message.contains("injected failure"), "{point}: {message}");
                assert!(
                    message.contains("nothing was written")
                        || message.contains("rolled back, so the project is as it was"),
                    "{point}: {message}"
                );
                assert_eq!(tree(dir.path()), before, "point {point}: {message}");
            }
            Ok(reply) if text(&reply).contains("finishing it stopped") => {
                assert!(
                    text(&reply).contains("injected failure at do cleaned led-5mm")
                        || text(&reply).contains("injected failure at log cleaned")
                        || text(&reply).contains("injected failure at do clean"),
                    "{}",
                    text(&reply)
                );
                let line = recover(dir.path());
                assert!(
                    line.starts_with("finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm, parts.lock"),
                    "{line}"
                );
                assert_replaced(dir.path());
            }
            Ok(reply) => {
                assert!(point > 10, "only {point} points");
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
        point += 1;
    }
}

#[test]
fn a_rollback_names_what_it_put_back() {
    let dir = edited_project();
    let before = tree(dir.path());
    let error = run_with(Some(dir.path()), &ask(REPLACE, true), &mut |at| {
        (at == "do place tactile-switch-6mm").then_some(Inject::Fail)
    })
    .err()
    .unwrap();
    assert_eq!(
        error.to_string(),
        "injected failure at do place tactile-switch-6mm\nrolled back, so the project is as it was: put back parts/resistor-axial, put back parts/led-5mm"
    );
    assert_eq!(tree(dir.path()), before);
}

/// The add is killed at each point, then the recovery after it is killed at each of its points,
/// then a third run recovers. Every edit is kept unless the add had committed, and the reply
/// says which.
#[test]
fn a_kill_in_the_add_then_a_kill_in_recovery_keeps_every_edit() {
    let mut cases = 0;
    for add_point in 0.. {
        let killed_add = |root: &Path| {
            let mut seen = 0;
            run_with(Some(root), &ask(REPLACE, true), &mut |_| {
                seen += 1;
                (seen == add_point + 1).then_some(Inject::Kill)
            })
            .is_err()
        };
        let probe = edited_project();
        if !killed_add(probe.path()) {
            assert!(add_point > 10, "only {add_point} points");
            break;
        }
        for recover_point in 0.. {
            let dir = edited_project();
            let before = tree(dir.path());
            assert!(killed_add(dir.path()));
            let killed = tree(dir.path());
            let mut seen = 0;
            let first = txn::recover(dir.path(), AGAIN, &mut |_| {
                seen += 1;
                (seen == recover_point + 1).then_some(Inject::Kill)
            });
            let line = match &first {
                Ok(lines) => lines.join("\n"),
                Err(_) => recover(dir.path()),
            };
            cases += 1;
            let at = format!("add killed at {add_point}, recovery at {recover_point}: {line}");
            if line.starts_with("finished an interrupted add") {
                assert_replaced(dir.path());
            } else {
                assert_eq!(tree(dir.path()), before, "{at}");
                if killed != before {
                    assert!(
                        line.starts_with("rolled back an interrupted add")
                            || line.starts_with("deleted"),
                        "{at}"
                    );
                }
            }
            assert!(!dir.path().join("parts").join(txn::STAGE).exists(), "{at}");
            assert!(recover(dir.path()).is_empty(), "{at}");
            if first.is_ok() {
                break;
            }
        }
    }
    assert!(cases > 100, "{cases}");
}

#[test]
fn the_next_add_says_it_rolled_back_an_interrupted_one() {
    let dir = edited_project();
    stop_add(dir.path(), kill_at("do place resistor-axial"));
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).starts_with("error[part_differs]: nothing was written"),
        "{}",
        text(&reply)
    );
    assert!(
        text(&reply).contains(
            "rolled back an interrupted add: put back parts/resistor-axial, removed the new parts/led-5mm"
        ) || text(&reply).contains(
            "rolled back an interrupted add: put back parts/resistor-axial, put back parts/led-5mm"
        ),
        "{}",
        text(&reply)
    );
    assert!(part_file(dir.path(), "led-5mm").contains("my edit"));
    assert!(part_file(dir.path(), "resistor-axial").contains("my edit"));
}

/// An edit to the placed copy after a crash stops the rollback. Both copies stay, and each way
/// out the hint names finishes it.
#[test]
fn an_edit_after_a_crash_is_never_deleted() {
    for keep_new in [true, false] {
        let dir = edited_project();
        stop_add(dir.path(), kill_at("do aside resistor-axial"));
        let led = dir.path().join("parts/led-5mm/part.jsonc");
        append_to(&led, "// after the crash\n");
        let error = add(dir.path(), &["tactile-switch-6mm"], false)
            .err()
            .unwrap()
            .to_string();
        assert_eq!(
            error,
            "parts/led-5mm changed after the interrupted add placed its new copy there, and the copy from before the add is in parts/.orbe-add/old/led-5mm. orbe left both\n  hint: keep one: delete parts/led-5mm to get back parts/.orbe-add/old/led-5mm, or delete parts/.orbe-add/old/led-5mm to keep parts/led-5mm as it is. then run `orbe add -C "
                .to_string()
                + &dir.path().display().to_string()
                + " tactile-switch-6mm` again"
        );
        assert!(part_file(dir.path(), "led-5mm").ends_with("// after the crash\n"));
        let saved = dir.path().join("parts/.orbe-add/old/led-5mm/part.jsonc");
        assert!(
            fs::read_to_string(&saved)
                .unwrap()
                .ends_with("// my edit\n")
        );

        if keep_new {
            fs::remove_dir_all(dir.path().join("parts/.orbe-add/old/led-5mm")).unwrap();
        } else {
            fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
        }
        let reply = add(dir.path(), &["tactile-switch-6mm"], false).unwrap();
        let expect = if keep_new {
            "rolled back an interrupted add: left parts/led-5mm as it is, since it changed after the add"
        } else {
            "rolled back an interrupted add: put back parts/led-5mm"
        };
        assert!(text(&reply).ends_with(expect), "{}", text(&reply));
        let led = part_file(dir.path(), "led-5mm");
        if keep_new {
            assert!(led.ends_with("// after the crash\n"), "{led}");
        } else {
            assert!(led.ends_with("// my edit\n"), "{led}");
        }
        assert!(part_file(dir.path(), "resistor-axial").ends_with("// my edit\n"));
        assert!(!dir.path().join("parts/.orbe-add").exists());
    }
}

/// After the commit, a saved copy is deleted only while the new copy is in place.
#[test]
fn a_committed_add_puts_back_the_copy_it_replaced_when_the_new_one_is_gone() {
    let dir = edited_project();
    stop_add(dir.path(), kill_at("do cleaned"));
    fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
    let line = recover(dir.path());
    assert!(
        line.starts_with(
            "finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm, parts.lock"
        ),
        "{line}"
    );
    assert!(
        line.ends_with(". parts/led-5mm was gone, so orbe put back the copy the add had replaced"),
        "{line}"
    );
    assert!(part_file(dir.path(), "led-5mm").ends_with("// my edit\n"));
    assert!(!part_file(dir.path(), "resistor-axial").contains("my edit"));
}

#[test]
fn a_name_no_add_made_is_never_deleted() {
    let dir = edited_project();
    stop_add(dir.path(), kill_at("do cleaned"));
    let precious = dir.path().join("parts/.orbe-add/old/precious");
    fs::create_dir(&precious).unwrap();
    fs::write(precious.join("note.txt"), "KEEP-ME\n").unwrap();
    let error = add(dir.path(), &["led-5mm"], false)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.starts_with(
            "parts/.orbe-add/old/precious was not made by an add, so orbe left parts/.orbe-add and its log\nbefore it stopped, orbe deleted parts/.orbe-add/old/resistor-axial, deleted parts/.orbe-add/old/led-5mm\n  hint: move parts/.orbe-add/old/precious out of parts/.orbe-add. then run `orbe add -C "
        ),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(precious.join("note.txt")).unwrap(),
        "KEEP-ME\n"
    );
    fs::rename(&precious, dir.path().join("precious")).unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply)
            .ends_with(", and deleted the copies it replaced: parts/led-5mm, parts/resistor-axial"),
        "{}",
        text(&reply)
    );
    assert_replaced(dir.path());
}

#[test]
fn a_kill_after_the_commit_is_finished_and_said_to_be() {
    let dir = edited_project();
    stop_add(dir.path(), kill_at("log committed"));
    let line = recover(dir.path());
    assert!(
        line.starts_with("rolled back an interrupted add: "),
        "the commit line was not written, so it rolls back: {line}"
    );

    let dir = edited_project();
    stop_add(dir.path(), kill_at("do cleaned led-5mm"));
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).contains("finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm, parts.lock, and deleted the copies it replaced: parts/led-5mm, parts/resistor-axial"),
        "{}",
        text(&reply)
    );
    assert_replaced(dir.path());
}

/// With its log gone, nothing proves what the add moved, so recovery moves nothing.
#[test]
fn a_staging_folder_with_no_log_is_left_and_named() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    let lock = fs::read(dir.path().join("parts.lock")).unwrap();
    run_with(
        Some(dir.path()),
        &ask(&["resistor-axial"], false),
        &mut |at| at.starts_with("log placed").then_some(Inject::Kill),
    )
    .err()
    .unwrap();
    fs::remove_file(dir.path().join("parts/.orbe-add/log")).unwrap();
    let before = tree(dir.path());
    let error = add(dir.path(), &["led-5mm"], false)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.starts_with("parts/.orbe-add has no log, so orbe cannot tell what the add that left it changed, and it left parts/.orbe-add/new, parts/.orbe-add/old, parts/.orbe-add/parts.lock as they are. that add may have written folders in parts/ and parts.lock"),
        "{error}"
    );
    assert_eq!(tree(dir.path()), before);
    assert!(dir.path().join("parts/resistor-axial").is_dir());
    assert_eq!(fs::read(dir.path().join("parts.lock")).unwrap(), lock);
}

#[test]
fn an_empty_staging_folder_is_removed_without_a_claim() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    fs::create_dir(dir.path().join("parts/.orbe-add")).unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(
        text(&reply),
        "skipped parts/led-5mm 0.1.0\nparts.lock unchanged\ndeleted parts/.orbe-add, an empty folder an earlier add left"
    );
}

/// A second run while one holds the lock refuses at once and touches nothing, `--diff` too.
#[test]
fn a_second_run_is_busy_while_one_holds_the_lock() {
    let dir = edited_project();
    let before = tree(dir.path());
    let held = held::hold(dir.path()).ok().unwrap();
    for diff in [false, true] {
        let request = Request {
            diff,
            ..ask(&["led-5mm"], true)
        };
        let reply = run(Some(dir.path()), &request).unwrap();
        assert_eq!(reply.code(), 1);
        let lock = dir.path().join(".orbe-add.lock");
        assert_eq!(
            text(&reply),
            format!(
                "error[busy]: another `orbe add` is at work in this project. it holds the lock on {}\n  hint: wait for it to finish, then run `orbe add -C {} led-5mm --overwrite{}`",
                lock.display(),
                dir.path().display(),
                if request.diff { " --diff" } else { "" }
            )
        );
        assert_eq!(
            tree(dir.path())
                .keys()
                .filter(|path| *path != ".orbe-add.lock")
                .count(),
            before.len()
        );
    }
    drop(held);
    assert_eq!(tree(dir.path()), before);
    let reply = add(dir.path(), &["led-5mm"], true).unwrap();
    assert!(text(&reply).starts_with("wrote parts/led-5mm 0.1.0, replacing"));
    assert!(!dir.path().join(".orbe-add.lock").exists());
}

/// A run killed while it held the lock leaves the file, and the next run takes it.
#[test]
fn a_lock_file_a_killed_run_left_does_not_block() {
    let dir = project();
    fs::write(dir.path().join(".orbe-add.lock"), "").unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.0\nwrote parts.lock");
    assert!(!dir.path().join(".orbe-add.lock").exists());
}

#[test]
fn diff_leaves_an_interrupted_add_for_the_next_writing_run() {
    let dir = edited_project();
    stop_add(dir.path(), kill_at("do place resistor-axial"));
    let pending = tree(dir.path());
    let request = Request {
        diff: true,
        ..ask(&["led-5mm"], false)
    };
    let reply = run(Some(dir.path()), &request).unwrap();
    assert!(
        text(&reply).contains("an interrupted add is pending in parts/.orbe-add. --diff leaves it, and the next `orbe add -C "),
        "{}",
        text(&reply)
    );
    assert!(text(&reply).contains("without --diff rolls it back"));
    assert_eq!(tree(dir.path()), pending);
    let reply = add(dir.path(), &["led-5mm"], true).unwrap();
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
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(reply.code(), 1);
    assert!(
        text(&reply).contains("parts/led-5mm is a file, not a part folder"),
        "{}",
        text(&reply)
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "my notes, not a part\n");
    assert!(!dir.path().join("parts.lock").exists());

    let reply = add(dir.path(), &["led-5mm"], true).unwrap();
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
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).contains("parts/led-5mm is a link, not a part folder"),
        "{}",
        text(&reply)
    );
    let reply = add(dir.path(), &["led-5mm"], true).unwrap();
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
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.0\nwrote parts.lock");
}

/// `orbe check` names the folder of a scene of any name with `-C`, so a named folder is trusted.
#[test]
fn a_folder_named_with_c_is_trusted_and_a_missing_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("blinky.jsonc"), "{}\n").unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.0\nwrote parts.lock");

    let missing = dir.path().join("nowhere");
    let reply = add(&missing, &["led-5mm"], false).unwrap();
    assert_eq!(reply.code(), 1);
    assert_eq!(
        text(&reply),
        format!(
            "error[not_a_project]: `{}` is not a folder",
            missing.display()
        )
    );
    assert!(!missing.exists());
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
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.0\nwrote parts.lock");
    edit(dir.path(), "led-5mm");
    let reply = add(dir.path(), &["led-5mm"], true).unwrap();
    assert!(text(&reply).contains("replacing 0.1.0 with local edits"));
    assert_lock_agrees(dir.path());
}
