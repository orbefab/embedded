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
fn stop_add(root: &Path, pick: impl FnMut(&str) -> Option<Inject>) -> RunError {
    stop_add_of(root, REPLACE, pick)
}

fn stop_add_of(
    root: &Path,
    ids: &[&str],
    mut pick: impl FnMut(&str) -> Option<Inject>,
) -> RunError {
    run_with(Some(root), &ask(ids, true), &mut pick)
        .err()
        .expect("the add stopped")
}

fn kill_at(point: &'static str) -> impl FnMut(&str) -> Option<Inject> {
    move |at: &str| at.starts_with(point).then_some(Inject::Kill)
}

/// Kills the add at its first point after the commit line, before it deletes anything.
fn kill_after_commit() -> impl FnMut(&str) -> Option<Inject> {
    let mut committed = false;
    move |at: &str| {
        let kill = committed;
        committed |= at == "log committed";
        kill.then_some(Inject::Kill)
    }
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
    assert!(!dir.path().join(".orbe-add.lock").exists());
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
                    text(&reply).contains("injected failure at do clean")
                        || text(&reply)
                            .contains("injected failure at do delete parts/.orbe-add.trash"),
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
            assert!(!dir.path().join("parts/.orbe-add.trash").exists(), "{at}");
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
    stop_add(dir.path(), kill_after_commit());
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
    stop_add(dir.path(), kill_after_commit());
    let before = tree(dir.path());
    let precious = dir.path().join("parts/.orbe-add/old/precious");
    fs::create_dir(&precious).unwrap();
    fs::write(precious.join("note.txt"), "KEEP-ME\n").unwrap();
    let error = add(dir.path(), &["led-5mm"], false)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.starts_with(
            "parts/.orbe-add/old/precious was not made by an add, so orbe left parts/.orbe-add and its log\n  hint: move parts/.orbe-add/old/precious out of parts/.orbe-add. then run `orbe add -C "
        ),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(precious.join("note.txt")).unwrap(),
        "KEEP-ME\n"
    );
    fs::rename(&precious, dir.path().join("precious")).unwrap();
    assert_eq!(tree(dir.path()).len(), before.len() + 2);
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).ends_with(
            ", and deleted the old copies of parts/led-5mm, parts/resistor-axial it replaced"
        ),
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
    stop_add(dir.path(), kill_after_commit());
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).contains("finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm, parts.lock, and deleted the old copies of parts/led-5mm, parts/resistor-axial it replaced"),
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
        assert_eq!(
            text(&reply),
            format!(
                "error[busy]: another `orbe add` is at work in this project, and holds its lock\n  hint: wait for it to finish, then run `orbe add -C {} led-5mm --overwrite{}`",
                dir.path().display(),
                if request.diff { " --diff" } else { "" }
            )
        );
        assert_eq!(tree(dir.path()), before);
    }
    drop(held);
    assert_eq!(tree(dir.path()), before);
    let reply = add(dir.path(), &["led-5mm"], true).unwrap();
    assert!(text(&reply).starts_with("wrote parts/led-5mm 0.1.0, replacing"));
    assert!(!dir.path().join(".orbe-add.lock").exists());
}

/// The lock is on the project folder, so no file stands for it. Deleting one frees nothing, and
/// a second run, `--diff` included, stays busy.
#[test]
fn a_deleted_lock_file_does_not_let_a_second_run_in() {
    let dir = edited_project();
    let before = tree(dir.path());
    let held = held::hold(dir.path()).ok().unwrap();
    assert!(!dir.path().join(".orbe-add.lock").exists());
    for diff in [false, true] {
        let request = Request {
            diff,
            ..ask(&["tactile-switch-6mm"], false)
        };
        let reply = run(Some(dir.path()), &request).unwrap();
        assert_eq!(reply.code(), 1, "{}", text(&reply));
        assert!(text(&reply).starts_with("error[busy]"), "{}", text(&reply));
    }
    assert_eq!(tree(dir.path()), before);
    drop(held);
}

/// A link where a lock file used to go is never opened, so nothing is made where it points.
#[test]
fn a_link_named_like_a_lock_file_is_never_followed() {
    let dir = project();
    let outside = tempfile::tempdir().unwrap();
    let planted = outside.path().join("planted");
    std::os::unix::fs::symlink(&planted, dir.path().join(".orbe-add.lock")).unwrap();
    let diff = Request {
        diff: true,
        ..ask(&["led-5mm"], false)
    };
    run(Some(dir.path()), &diff).unwrap();
    add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(!planted.exists());
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
}

/// A run killed inside an append leaves part of a line. Recovery drops it before its own first
/// line, so a recovery killed after that line is still read by the next run.
#[test]
fn a_torn_last_log_line_is_dropped_before_the_next_append() {
    let dir = edited_project();
    let before = tree(dir.path());
    stop_add(dir.path(), kill_at("do place resistor-axial"));
    append_to(
        &dir.path().join("parts/.orbe-add/log"),
        "placed resistor-axial sha256:4f49ecfd39d5195100ee",
    );
    let mut logged = 0;
    let first = txn::recover(dir.path(), AGAIN, &mut |at| {
        logged += usize::from(at.starts_with("log "));
        (logged == 2).then_some(Inject::Kill)
    });
    assert!(first.is_err());
    let line = recover(dir.path());
    assert!(
        line.starts_with("rolled back an interrupted add: put back parts/resistor-axial"),
        "{line}"
    );
    assert_eq!(tree(dir.path()), before);
}

/// A log line orbe cannot read stops recovery. The hint never moves a saved copy back over a
/// finished add, and following it as written finishes that add.
#[test]
fn an_unreadable_log_line_gets_a_hint_that_keeps_a_finished_add() {
    let dir = edited_project();
    stop_add(dir.path(), kill_after_commit());
    let log = dir.path().join("parts/.orbe-add/log");
    let kept = fs::read(&log).unwrap();
    append_to(&log, "garbage\n");
    let error = add(dir.path(), &["led-5mm"], false)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.starts_with(
            "parts/.orbe-add/log has a line this orbe does not write, `garbage`, so orbe left parts/.orbe-add as it is\n  hint: delete that line and the lines after it from parts/.orbe-add/log, then run `orbe add -C "
        ),
        "{error}"
    );
    assert!(!error.contains("back to parts/"), "{error}");
    fs::write(&log, kept).unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).contains("finished an interrupted add"),
        "{}",
        text(&reply)
    );
    assert_replaced(dir.path());
}

/// After the commit, the saved copy is the user's only copy of what the add replaced. An edit
/// to it, or a name added inside it, stops recovery, and both copies stay.
#[test]
fn an_edit_to_a_saved_copy_after_the_commit_is_kept() {
    let dir = edited_project();
    stop_add(dir.path(), kill_after_commit());
    let old = dir.path().join("parts/.orbe-add/old/led-5mm");
    fs::write(old.join("notes.txt"), "KEEP-ME\n").unwrap();
    append_to(&old.join("part.jsonc"), "// saved copy edit\n");
    for _ in 0..2 {
        let error = add(dir.path(), &["resistor-0603"], false)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.starts_with(
                "parts/.orbe-add/old/led-5mm changed after the add moved the copy from parts/led-5mm there. orbe left it, and parts/led-5mm as it is\n  hint: move parts/.orbe-add/old/led-5mm out of parts/.orbe-add to keep it. then run `orbe add -C "
            ),
            "{error}"
        );
    }
    assert_eq!(
        fs::read_to_string(old.join("notes.txt")).unwrap(),
        "KEEP-ME\n"
    );
    assert!(
        fs::read_to_string(old.join("part.jsonc"))
            .unwrap()
            .ends_with("// my edit\n// saved copy edit\n")
    );
    assert!(!part_file(dir.path(), "led-5mm").contains("my edit"));

    let kept = dir.path().join("kept-led");
    fs::rename(&old, &kept).unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).ends_with(
            "finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm, parts.lock, and deleted the old copy of parts/resistor-axial it replaced"
        ),
        "{}",
        text(&reply)
    );
    fs::remove_dir_all(&kept).unwrap();
    assert_replaced(dir.path());
}

/// A recovery killed after it moved a placed copy out, before its log line, leaves the log
/// saying `placed`. The next run says recovery removed it, not that it was gone.
#[test]
fn a_placed_copy_a_killed_recovery_moved_out_is_said_to_be_removed() {
    let dir = edited_project();
    let before = tree(dir.path());
    stop_add(dir.path(), kill_at("log lock-written"));
    let first = txn::recover(dir.path(), AGAIN, &mut |at| {
        at.starts_with("log unplaced tactile-switch-6mm")
            .then_some(Inject::Kill)
    });
    assert!(first.is_err());
    let line = recover(dir.path());
    assert!(
        line.contains("removed the new parts/tactile-switch-6mm"),
        "{line}"
    );
    assert!(!line.contains("already gone"), "{line}");
    assert_eq!(tree(dir.path()), before);
}

#[test]
fn diff_names_the_leftover_it_finds() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    fs::write(dir.path().join(".parts.lock.orbe-add"), "half a lock").unwrap();
    let request = Request {
        diff: true,
        ..ask(&["led-5mm"], false)
    };
    let reply = run(Some(dir.path()), &request).unwrap();
    assert!(
        text(&reply).starts_with("an interrupted add left .parts.lock.orbe-add. --diff leaves it, and the next `orbe add -C "),
        "{}",
        text(&reply)
    );
    assert!(text(&reply).contains(" without --diff deletes it"));
    assert!(
        !text(&reply).contains("parts/.orbe-add"),
        "{}",
        text(&reply)
    );
}

/// A link at `parts/.orbe-add/old` is never followed, and recovery stops before it moves
/// anything. It never says the next run picks up when that run would stop the same way.
#[test]
fn a_link_at_old_is_never_followed() {
    let dir = edited_project();
    stop_add(dir.path(), kill_after_commit());
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(outside.path().join("led-5mm")).unwrap();
    fs::write(outside.path().join("led-5mm/secret"), "SECRET\n").unwrap();
    let old = dir.path().join("parts/.orbe-add/old");
    let saved = dir.path().join("saved-old");
    fs::rename(&old, &saved).unwrap();
    std::os::unix::fs::symlink(outside.path(), &old).unwrap();
    for _ in 0..2 {
        let error = add(dir.path(), &["led-5mm"], false)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.starts_with(
                "parts/.orbe-add/old was not made by an add, so orbe left parts/.orbe-add and its log\n  hint: move parts/.orbe-add/old out of parts/.orbe-add. then run `orbe add -C "
            ),
            "{error}"
        );
        assert!(!error.contains("picks up"), "{error}");
        assert_eq!(
            fs::read_to_string(outside.path().join("led-5mm/secret")).unwrap(),
            "SECRET\n"
        );
    }
    fs::remove_file(&old).unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).ends_with(
            "finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm, parts.lock"
        ),
        "{}",
        text(&reply)
    );
    assert!(
        fs::read_to_string(saved.join("led-5mm/part.jsonc"))
            .unwrap()
            .ends_with("// my edit\n")
    );
    fs::remove_dir_all(&saved).unwrap();
    assert_replaced(dir.path());
}

/// A log from an earlier run, put back with empty `new/` and `old/`, has no saved copy to put
/// back, so the project's only copy of the part stays.
#[test]
fn an_older_log_never_removes_the_only_copy_of_a_part() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    edit(dir.path(), "led-5mm");
    run_with(
        Some(dir.path()),
        &ask(&["led-5mm"], true),
        &mut kill_at("do place led-5mm"),
    )
    .err()
    .unwrap();
    let old_log = fs::read(dir.path().join("parts/.orbe-add/log")).unwrap();
    add(dir.path(), &["led-5mm"], false).unwrap();
    add(dir.path(), &["led-5mm"], true).unwrap();
    let catalog = part_file(dir.path(), "led-5mm");
    let lock = fs::read(dir.path().join("parts.lock")).unwrap();
    for dir_name in ["parts/.orbe-add/new", "parts/.orbe-add/old"] {
        fs::create_dir_all(dir.path().join(dir_name)).unwrap();
    }
    fs::write(dir.path().join("parts/.orbe-add/log"), &old_log).unwrap();
    let error = add(dir.path(), &["tactile-switch-6mm"], false)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.starts_with(
            "parts/led-5mm holds the new copy the interrupted add placed, and the copy it replaced is not in parts/.orbe-add/old/led-5mm, so orbe cannot put that back. orbe left parts/led-5mm\n  hint: delete parts/led-5mm, whose files are the catalog's, to finish the rollback. then run `orbe add -C "
        ),
        "{error}"
    );
    assert_eq!(part_file(dir.path(), "led-5mm"), catalog);
    assert_eq!(fs::read(dir.path().join("parts.lock")).unwrap(), lock);
    assert!(!dir.path().join("parts/tactile-switch-6mm").exists());
}

/// Truncating `committed` and what follows off the log, with `new/` and `old/` gone or empty,
/// leaves the part in place, and the reply never says the next run picks up.
#[test]
fn a_log_cut_before_its_commit_never_removes_the_part() {
    for empty_folders in [false, true] {
        let dir = project();
        add(dir.path(), &["led-5mm"], false).unwrap();
        edit(dir.path(), "led-5mm");
        stop_add_of(dir.path(), &["led-5mm"], kill_at("do clean"));
        let stage = dir.path().join("parts/.orbe-add");
        let log = fs::read_to_string(stage.join("log")).unwrap();
        let cut = log
            .find("lock-written\n")
            .or(log.find("committed\n"))
            .unwrap();
        fs::write(stage.join("log"), &log[..cut]).unwrap();
        for name in ["new", "old"] {
            let _ = fs::remove_dir_all(stage.join(name));
            if empty_folders {
                fs::create_dir(stage.join(name)).unwrap();
            }
        }
        for _ in 0..2 {
            let error = add(dir.path(), &["led-5mm"], false)
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("orbe left parts/led-5mm\n  hint: delete parts/led-5mm"),
                "{error}"
            );
            assert!(!error.contains("picks up"), "{error}");
            assert!(dir.path().join("parts/led-5mm/part.jsonc").is_file());
        }
        fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
        let reply = add(dir.path(), &["led-5mm"], false).unwrap();
        assert!(
            text(&reply).starts_with("wrote parts/led-5mm 0.1.0"),
            "{}",
            text(&reply)
        );
        assert_lock_agrees(dir.path());
        assert!(!stage.exists());
    }
}

/// `aside <id> -` says nothing was moved aside, so a folder at `old/<id>` is not the add's: it
/// is never put in place, never deleted, and the add's copy stays where the log placed it.
#[test]
fn a_folder_planted_in_old_is_never_installed_or_deleted() {
    let dir = project();
    run_with(
        Some(dir.path()),
        &ask(&["led-5mm"], false),
        &mut kill_at("do write parts.lock"),
    )
    .err()
    .unwrap();
    let planted = dir.path().join("parts/.orbe-add/old/led-5mm");
    fs::create_dir_all(&planted).unwrap();
    fs::write(planted.join("note.txt"), "KEEP-ME\n").unwrap();
    let error = add(dir.path(), &["led-5mm"], false)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.starts_with(
            "parts/.orbe-add/old/led-5mm was not made by an add, so orbe left parts/.orbe-add and its log\n  hint: move parts/.orbe-add/old/led-5mm out of parts/.orbe-add. then run `orbe add -C "
        ),
        "{error}"
    );
    assert!(dir.path().join("parts/led-5mm/part.jsonc").is_file());
    let kept = dir.path().join("kept");
    fs::rename(&planted, &kept).unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert!(
        text(&reply).starts_with("wrote parts/led-5mm 0.1.0\nwrote parts.lock\nrolled back an interrupted add: removed the new parts/led-5mm"),
        "{}",
        text(&reply)
    );
    assert_eq!(
        fs::read_to_string(kept.join("note.txt")).unwrap(),
        "KEEP-ME\n"
    );
}

/// The add wrote `parts.lock`, then the saved copy of the old lock was edited. Recovery keeps
/// both and never says the live lock changed, since it is the one the add wrote.
#[test]
fn an_edited_saved_lock_is_kept_beside_the_lock_the_add_wrote() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    run_with(
        Some(dir.path()),
        &ask(&["resistor-axial"], false),
        &mut kill_at("log lock-written"),
    )
    .err()
    .unwrap();
    let saved = dir.path().join("parts/.orbe-add/parts.lock");
    append_to(&saved, "# SAVED-LOCK-EDIT\n");
    let live = fs::read(dir.path().join("parts.lock")).unwrap();
    let error = add(dir.path(), &["resistor-axial"], false)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.starts_with(
            "parts/.orbe-add/parts.lock changed after the add saved the old parts.lock there. orbe left it, and parts.lock as it is\n  hint: move parts/.orbe-add/parts.lock out of parts/.orbe-add to keep it. then run `orbe add -C "
        ),
        "{error}"
    );
    assert!(
        fs::read_to_string(&saved)
            .unwrap()
            .ends_with("# SAVED-LOCK-EDIT\n")
    );
    assert_eq!(fs::read(dir.path().join("parts.lock")).unwrap(), live);

    fs::rename(&saved, dir.path().join("saved.lock")).unwrap();
    let reply = add(dir.path(), &["resistor-axial"], false).unwrap();
    assert!(
        text(&reply).contains(
            "rolled back an interrupted add: removed the new parts/resistor-axial, left the parts.lock the add wrote, since the copy from before it was not in parts/.orbe-add"
        ),
        "{}",
        text(&reply)
    );
    assert!(!text(&reply).contains("since it changed after the add"));
    assert_lock_agrees(dir.path());
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

/// An edit to the new copy after a crash past the commit keeps the copy the add replaced.
#[test]
fn an_edit_to_the_new_copy_after_the_commit_keeps_the_saved_copy() {
    let dir = edited_project();
    stop_add(dir.path(), kill_after_commit());
    append_to(
        &dir.path().join("parts/led-5mm/part.jsonc"),
        "// after the crash\n",
    );
    let error = add(dir.path(), &["led-5mm"], false)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.starts_with(
            "parts/led-5mm changed after the add replaced it, and the copy it replaced is in parts/.orbe-add/old/led-5mm. orbe left both"
        ),
        "{error}"
    );
    assert!(part_file(dir.path(), "led-5mm").ends_with("// after the crash\n"));
    assert!(
        fs::read_to_string(dir.path().join("parts/.orbe-add/old/led-5mm/part.jsonc"))
            .unwrap()
            .ends_with("// my edit\n")
    );
}
