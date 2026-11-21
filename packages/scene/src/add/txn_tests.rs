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

/// `text` with the paths below `root` written from the project, as a run with no `-C` names them.
fn unrooted(root: &Path, text: String) -> String {
    text.replace(&format!("{}/", root.display()), "")
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
fn again() -> Again {
    Again::new(None, "orbe add tactile-switch-6mm".into())
}

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
    txn::recover(root, &again(), &mut |_| None)
        .unwrap()
        .lines
        .join("\n")
}

#[test]
fn run_copies_a_real_part_and_a_second_run_keeps_the_bytes() {
    let dir = project();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1\nwrote parts.lock");
    let before = tree(dir.path());
    let again = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(
        text(&again),
        "skipped parts/led-5mm 0.1.1\nparts.lock unchanged"
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
                        || text(&reply).contains("injected failure at log clearing")
                        || text(&reply).contains("injected failure at do delete parts/.orbe-add"),
                    "{}",
                    text(&reply)
                );
                let line = recover(dir.path());
                let log_gone = text(&reply).contains("orbe had deleted its log");
                assert!(
                    line.starts_with(if log_gone {
                        "deleted parts/.orbe-add, an empty folder an earlier add left"
                    } else {
                        "finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm, parts.lock"
                    }),
                    "{line}"
                );
                assert_replaced(dir.path());
            }
            Ok(reply) => {
                assert!(point > 10, "only {point} points");
                assert!(
                    text(&reply)
                        .contains("wrote parts/led-5mm 0.1.1, replacing 0.1.1 with local edits"),
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

fn step_kind(point: &str) -> &str {
    if let Some(rest) = point.strip_prefix("do delete")
        && (rest.is_empty() || rest.starts_with(' '))
    {
        return "delete";
    }
    let mut words = point.split_whitespace();
    let Some(first) = words.next() else {
        return point;
    };
    let Some(second) = words.next() else {
        return first;
    };
    let end = first.len() + 1 + second.len();
    &point[..end]
}

fn kind_edges(points: &[String]) -> Vec<usize> {
    let mut chosen = Vec::new();
    let mut start = 0;
    while start < points.len() {
        let kind = step_kind(&points[start]);
        let mut end = start + 1;
        while end < points.len() && step_kind(&points[end]) == kind {
            end += 1;
        }
        let last = end - 1;
        chosen.push(start);
        if last > start {
            if start + 1 < last {
                chosen.push(start + 1);
            }
            if last > start + 2 {
                chosen.push(last - 1);
            }
            chosen.push(last);
        }
        start = end;
    }
    chosen
}

fn duplicate(src: &Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_into(src, dir.path());
    dir
}

fn copy_into(src: &Path, dest: &Path) {
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let meta = fs::symlink_metadata(&from).unwrap();
        if meta.file_type().is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(&from).unwrap(), &to).unwrap();
        } else if meta.is_dir() {
            fs::create_dir(&to).unwrap();
            copy_into(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

fn kill_add(root: &Path, add_point: usize) -> bool {
    let mut seen = 0;
    run_with(Some(root), &ask(REPLACE, true), &mut |_| {
        seen += 1;
        (seen == add_point + 1).then_some(Inject::Kill)
    })
    .is_err()
}

fn kept_after(
    root: &Path,
    before: &BTreeMap<String, Vec<u8>>,
    killed: &BTreeMap<String, Vec<u8>>,
    first: Result<txn::Recovered, txn::Stop>,
    add_point: usize,
    recover_point: usize,
) {
    let line = match &first {
        Ok(recovered) => recovered.lines.join("\n"),
        Err(_) => recover(root),
    };
    let at = format!("add killed at {add_point}, recovery at {recover_point}: {line}");
    if line.starts_with("finished an interrupted add") {
        assert_replaced(root);
    } else if line.starts_with("deleted parts/.orbe-add, an empty folder") {
        if tree(root) != *before {
            assert_replaced(root);
        }
    } else {
        assert_eq!(tree(root), *before, "{at}");
        if killed != before {
            assert!(
                line.starts_with("rolled back an interrupted add") || line.starts_with("deleted"),
                "{at}"
            );
        }
    }
    assert!(!root.join("parts").join(txn::STAGE).exists(), "{at}");
    assert!(recover(root).is_empty(), "{at}");
}

/// The add is killed, then the recovery after it is killed, then a third run recovers.
/// The kills are the first, second, next-to-last, and last step of each kind, and each
/// case copies one built project. Every edit is kept unless the add had committed, and
/// the reply says which.
#[test]
fn a_kill_in_the_add_then_a_kill_in_recovery_keeps_every_edit() {
    let template = edited_project();
    let before = tree(template.path());
    let recorded = duplicate(template.path());
    let mut add_points = Vec::new();
    run_with(Some(recorded.path()), &ask(REPLACE, true), &mut |at| {
        add_points.push(at.to_string());
        None
    })
    .unwrap();
    assert!(add_points.len() > 10, "{}", add_points.len());
    let mut cases = 0;
    for add_point in kind_edges(&add_points) {
        let work = duplicate(template.path());
        assert!(kill_add(work.path(), add_point), "add point {add_point}");
        let saved = duplicate(work.path());
        let killed = tree(saved.path());
        let mut recover_points = Vec::new();
        let first = txn::recover(work.path(), &again(), &mut |at| {
            recover_points.push(at.to_string());
            None
        });
        kept_after(
            work.path(),
            &before,
            &killed,
            first,
            add_point,
            recover_points.len(),
        );
        cases += 1;
        for recover_point in kind_edges(&recover_points) {
            let dir = duplicate(saved.path());
            let mut seen = 0;
            let first = txn::recover(dir.path(), &again(), &mut |_| {
                seen += 1;
                (seen == recover_point + 1).then_some(Inject::Kill)
            });
            assert!(
                first.is_err(),
                "add {add_point} recovery {recover_point} did not stop"
            );
            kept_after(
                dir.path(),
                &before,
                &killed,
                first,
                add_point,
                recover_point,
            );
            cases += 1;
        }
    }
    assert!(cases > 40, "{cases}");
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
        let error = unrooted(
            dir.path(),
            add(dir.path(), &["tactile-switch-6mm"], false)
                .err()
                .unwrap()
                .to_string(),
        );
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
            "finished an interrupted add, which had written parts/resistor-axial, parts/tactile-switch-6mm, parts.lock"
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
    let error = unrooted(
        dir.path(),
        add(dir.path(), &["led-5mm"], false)
            .err()
            .unwrap()
            .to_string(),
    );
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
    let error = unrooted(
        dir.path(),
        add(dir.path(), &["led-5mm"], false)
            .err()
            .unwrap()
            .to_string(),
    );
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
        "skipped parts/led-5mm 0.1.1\nparts.lock unchanged\ndeleted parts/.orbe-add, an empty folder an earlier add left"
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
    assert!(text(&reply).starts_with("wrote parts/led-5mm 0.1.1, replacing"));
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
    let first = txn::recover(dir.path(), &again(), &mut |at| {
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
    let error = unrooted(
        dir.path(),
        add(dir.path(), &["led-5mm"], false)
            .err()
            .unwrap()
            .to_string(),
    );
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
        let error = unrooted(
            dir.path(),
            add(dir.path(), &["resistor-0603"], false)
                .err()
                .unwrap()
                .to_string(),
        );
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
    let first = txn::recover(dir.path(), &again(), &mut |at| {
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
        let error = unrooted(
            dir.path(),
            add(dir.path(), &["led-5mm"], false)
                .err()
                .unwrap()
                .to_string(),
        );
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
    let error = unrooted(
        dir.path(),
        add(dir.path(), &["tactile-switch-6mm"], false)
            .err()
            .unwrap()
            .to_string(),
    );
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
            let error = unrooted(
                dir.path(),
                add(dir.path(), &["led-5mm"], false)
                    .err()
                    .unwrap()
                    .to_string(),
            );
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
            text(&reply).starts_with("wrote parts/led-5mm 0.1.1"),
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
    let error = unrooted(
        dir.path(),
        add(dir.path(), &["led-5mm"], false)
            .err()
            .unwrap()
            .to_string(),
    );
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
        text(&reply).starts_with("wrote parts/led-5mm 0.1.1\nwrote parts.lock\nrolled back an interrupted add: removed the new parts/led-5mm"),
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
    let error = unrooted(
        dir.path(),
        add(dir.path(), &["resistor-axial"], false)
            .err()
            .unwrap()
            .to_string(),
    );
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
        text(&reply).contains("wrote parts/led-5mm 0.1.1, replacing a file"),
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
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1\nwrote parts.lock");
}

/// `orbe check` names the folder of a scene of any name with `-C`, so a named folder is trusted.
#[test]
fn a_folder_named_with_c_is_trusted_and_a_missing_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("blinky.jsonc"), "{}\n").unwrap();
    let reply = add(dir.path(), &["led-5mm"], false).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1\nwrote parts.lock");

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
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1\nwrote parts.lock");
    edit(dir.path(), "led-5mm");
    let reply = add(dir.path(), &["led-5mm"], true).unwrap();
    assert!(text(&reply).contains("replacing 0.1.1 with local edits"));
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

/// `--diff` says what the next writing run does with what an interrupted add left, then that
/// run does it: it stops exactly when `--diff` said it would.
fn diff_predicts(root: &Path) -> Result<String, RunError> {
    let request = Request {
        diff: true,
        ..ask(&["resistor-0603"], false)
    };
    let preview = text(&run(Some(root), &request).unwrap());
    let real = txn::recover(root, &again(), &mut |_| None)
        .map(|recovered| recovered.lines.join("\n"))
        .map_err(|stop| match stop {
            txn::Stop::Failed(error) => error,
            txn::Stop::Killed => unreachable!(),
        });
    assert_eq!(
        preview.contains("and stops"),
        real.is_err(),
        "--diff said: {preview}\nthe run did: {real:?}"
    );
    if preview.contains("deletes it") || preview.contains("finishes") {
        assert!(
            real.is_ok(),
            "--diff said: {preview}\nthe run did: {real:?}"
        );
    }
    real
}

fn kept(root: &Path) -> Vec<String> {
    tree(root)
        .into_iter()
        .filter(|(_, bytes)| bytes.windows(7).any(|w| w == b"KEEP-ME"))
        .map(|(path, _)| path)
        .collect()
}

/// A kill at each point after the commit, while the staging folder is deleted, then a file
/// put in what is left, or a file there changed. Those bytes are never deleted: the next add
/// names them and stops, `--diff` says it will, and once they are moved out the add finishes.
#[test]
fn bytes_put_in_a_half_deleted_staging_folder_are_never_deleted() {
    let mut cases = 0;
    for point in 0.. {
        for change in ["plant", "edit"] {
            let dir = edited_project();
            let mut committed = false;
            let mut seen = 0;
            let killed = run_with(Some(dir.path()), &ask(REPLACE, true), &mut |at| {
                if committed {
                    seen += 1;
                }
                committed |= at == "log committed";
                (committed && seen == point + 1).then_some(Inject::Kill)
            });
            if killed.is_ok() {
                assert!(cases > 4, "only {cases} cases");
                return;
            }
            let parts = dir.path().join("parts");
            let left: Vec<_> = fs::read_dir(&parts)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".orbe-add")
                })
                .collect();
            let target = match change {
                "plant" => left
                    .iter()
                    .find(|path| path.is_dir())
                    .map(|dir| dir.join("mine.txt")),
                _ => left.iter().find_map(|path| {
                    tree(path)
                        .into_keys()
                        .map(|rel| path.join(rel))
                        .find(|file| file.is_file() && file.file_name().unwrap() != "log")
                }),
            };
            let Some(target) = target else {
                continue;
            };
            cases += 1;
            if target.exists() {
                append_to(&target, "KEEP-ME\n");
            } else {
                fs::write(&target, "KEEP-ME\n").unwrap();
            }
            let at = format!("point {point}, {change} {}", target.display());
            let Err(error) = diff_predicts(dir.path()) else {
                panic!("{at}: deleted {:?}", kept(dir.path()));
            };
            let error = error.to_string();
            assert_eq!(kept(dir.path()).len(), 1, "{at}: {error}");
            if error.contains("parts/.orbe-add has no log") {
                assert!(
                    error.contains("\n  hint: move anything you want to keep out of parts/.orbe-add, delete parts/.orbe-add, then run"),
                    "{at}: {error}"
                );
                fs::rename(&target, dir.path().join("moved-0")).unwrap();
                fs::remove_dir(&left[0]).unwrap();
            } else {
                move_as_hinted(dir.path(), &error, "sh");
                let line = diff_predicts(dir.path()).unwrap();
                assert!(
                    line.starts_with("finished an interrupted add, which had written"),
                    "{at}: {line}"
                );
            }
            assert_replaced(dir.path());
            let found = kept(dir.path());
            assert!(
                found.len() == 1 && found[0].starts_with("moved-"),
                "{at}: {found:?}"
            );
        }
    }
}

/// `parts/.orbe-add.trash` and `parts/.orbe-add.trash-log` are names an earlier orbe used. A
/// log there proves nothing about the folder beside it, so an add leaves both as they are.
#[test]
fn a_folder_and_log_at_the_old_trash_names_are_left_alone() {
    let dir = project();
    let trash = dir.path().join("parts/.orbe-add.trash");
    fs::create_dir_all(trash.join("mine")).unwrap();
    fs::write(trash.join("mine/note.txt"), "KEEP-ME\n").unwrap();
    fs::write(trash.join("log"), "orbe add log 1\nplan existed -\n").unwrap();
    fs::write(
        dir.path().join("parts/.orbe-add.trash-log"),
        "orbe add log 1\nplan existed - led-5mm\nplaced led-5mm sha256:0000000000000000000000000000000000000000000000000000000000000000\ncommitted\n",
    )
    .unwrap();
    let before = tree(&dir.path().join("parts"));
    assert_eq!(diff_predicts(dir.path()).unwrap(), "");
    let reply = text(&add(dir.path(), &["led-5mm"], false).unwrap());
    assert_eq!(reply, "wrote parts/led-5mm 0.1.1\nwrote parts.lock");
    let mut after = tree(&dir.path().join("parts"));
    after.retain(|path, _| !path.starts_with("led-5mm"));
    assert_eq!(after, before);
}

/// A clearing list in a log proves only the bytes it hashes. A file it does not list, and a
/// listed file with other bytes, both stay.
#[test]
fn a_clearing_list_deletes_only_the_bytes_it_hashes() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    let stage = dir.path().join("parts/.orbe-add");
    fs::create_dir_all(stage.join("old/led-5mm")).unwrap();
    fs::write(stage.join("old/led-5mm/part.jsonc"), "KEEP-ME\n").unwrap();
    fs::write(stage.join("mine.txt"), "KEEP-ME\n").unwrap();
    let wrong = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    fs::write(
        stage.join("log"),
        format!("orbe add log 1\nplan existed - led-5mm\ncommitted\nclearing d:old d:old/led-5mm f:old/led-5mm/part.jsonc:{wrong}\n"),
    )
    .unwrap();
    for _ in 0..2 {
        let error = diff_predicts(dir.path()).err().unwrap().to_string();
        assert!(
            error.contains("parts/.orbe-add/mine.txt, parts/.orbe-add/old/led-5mm/part.jsonc are not on the list"),
            "{error}"
        );
        assert!(error.contains("orbe deleted nothing"), "{error}");
        assert_eq!(kept(dir.path()).len(), 2);
    }
}

/// The reply after a rollback names a lock line left with no folder only when that is still so
/// once the run's own add is done, and it names the add that copies the folder back.
#[test]
fn a_lock_line_left_with_no_folder_is_named_with_the_add_that_fixes_it() {
    for (command, holed) in [("tactile-switch-6mm", true), ("led-5mm", false)] {
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
        for dir_name in ["parts/.orbe-add/new", "parts/.orbe-add/old"] {
            fs::create_dir_all(dir.path().join(dir_name)).unwrap();
        }
        fs::write(dir.path().join("parts/.orbe-add/log"), &old_log).unwrap();
        let error = unrooted(
            dir.path(),
            add(dir.path(), &[command], false)
                .err()
                .unwrap()
                .to_string(),
        );
        assert!(error.contains("\n  hint: delete parts/led-5mm"), "{error}");
        fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
        let got = text(&add(dir.path(), &[command], false).unwrap());
        assert!(!got.contains("as it was"), "{got}");
        assert!(!got.contains("orbe check"), "{got}");
        let fix = format!(
            "parts.lock lists led-5mm, and parts/led-5mm is missing. run `{} led-5mm` to copy it back",
            project::add_command(Some(dir.path()))
        );
        assert_eq!(got.contains(&fix), holed, "{got}");
        assert_eq!(dir.path().join("parts/led-5mm").exists(), !holed);
        assert!(!dir.path().join("parts/.orbe-add").exists());
        if holed {
            add(dir.path(), &["led-5mm"], false).unwrap();
        }
        assert_lock_agrees(dir.path());
    }
}

/// `--diff` names a file at a staging name as something the next add stops on when that add
/// would stop on it, and as a leftover it deletes only when it would.
#[test]
fn diff_says_what_the_next_add_does_with_a_staging_name() {
    let dir = project();
    fs::create_dir_all(dir.path().join("parts/.orbe-add")).unwrap();
    fs::write(dir.path().join("parts/.orbe-add/mine.txt"), "KEEP-ME\n").unwrap();
    diff_predicts(dir.path()).err().unwrap();
    assert_eq!(kept(dir.path()).len(), 1);
    fs::remove_file(dir.path().join("parts/.orbe-add/mine.txt")).unwrap();
    fs::write(dir.path().join("parts/.orbe-add/log"), "orbe add").unwrap();
    diff_predicts(dir.path()).unwrap();
    assert!(!dir.path().join("parts/.orbe-add").exists());
    fs::write(dir.path().join("parts/.orbe-add"), "KEEP-ME\n").unwrap();
    diff_predicts(dir.path()).err().unwrap();
    assert_eq!(kept(dir.path()).len(), 1);
}

/// Runs each path the hint says to move through `shell`, as printed, from `cwd`, into a new
/// `moved-<n>` there.
fn move_as_hinted(cwd: &Path, error: &str, shell: &str) {
    let hint = error.split("\n  hint: move ").nth(1).expect(error);
    let named = hint.split(" out of ").next().unwrap();
    for word in named.split(", ") {
        let n = (0..)
            .find(|n| !cwd.join(format!("moved-{n}")).exists())
            .unwrap();
        let status = held::forks::quiet(|| {
            std::process::Command::new(shell)
                .arg("-c")
                .arg(format!("mv -- {word} moved-{n}"))
                .current_dir(cwd)
                .status()
                .unwrap()
        });
        assert!(status.success(), "{shell}: mv -- {word} failed: {error}");
    }
}

/// A run killed after it writes `new/<id>` or the saved lock, and before it logs it, leaves
/// bytes no line hashes. Recovery deletes them only while they are a copy of what the add
/// writes there. Anything else is named and stays.
#[test]
fn bytes_put_in_a_copy_the_log_has_not_named_yet_are_never_deleted() {
    for (kill, path) in [
        (
            "log staged led-5mm",
            "parts/.orbe-add/new/led-5mm/notes.txt",
        ),
        (
            "log staged led-5mm",
            "parts/.orbe-add/new/led-5mm/part.jsonc",
        ),
        ("log saved-lock", "parts/.orbe-add/parts.lock"),
    ] {
        for planted in [false, true] {
            let dir = project();
            add(dir.path(), &["resistor-axial"], false).unwrap();
            let before = tree(dir.path());
            stop_add_of(dir.path(), &["led-5mm"], kill_at(kill));
            let target = dir.path().join(path);
            let at = format!("{kill}, {path}, planted {planted}");
            if planted {
                if target.exists() {
                    append_to(&target, "KEEP-ME\n");
                } else {
                    fs::write(&target, "KEEP-ME\n").unwrap();
                }
            } else if target.exists() {
                let bytes = fs::read(&target).unwrap();
                fs::write(&target, &bytes[..bytes.len() / 2]).unwrap();
            }
            if !planted {
                diff_predicts(dir.path()).unwrap();
                assert_eq!(tree(dir.path()), before, "{at}");
                continue;
            }
            let error = diff_predicts(dir.path()).expect_err(&at).to_string();
            assert_eq!(kept(dir.path()).len(), 1, "{at}: {error}");
            move_as_hinted(dir.path(), &error, "sh");
            diff_predicts(dir.path()).unwrap();
            let found = kept(dir.path());
            assert!(
                found.len() == 1 && found[0].starts_with("moved-"),
                "{at}: {found:?}"
            );
            assert!(!dir.path().join("parts/.orbe-add").exists(), "{at}");
        }
    }
}

/// Whether this volume stores `name`. A plain name is written first, so a temp dir that cannot
/// be used is not mistaken for a name the volume refused.
fn volume_stores(name: &[u8]) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("plain"), b"").unwrap();
    fs::write(dir.path().join(std::ffi::OsStr::from_bytes(name)), b"").is_ok()
}

/// A user folder an overwrite replaces may hold a name that is not UTF-8. The add moved it in
/// itself, so it deletes it with the old copy.
#[test]
fn an_overwrite_deletes_odd_names_it_moved_aside() {
    use std::os::unix::ffi::OsStrExt;
    let bytes = b"caf\xe9.txt";
    // APFS refuses a name that is not UTF-8, so an overwrite cannot be given one here.
    if !volume_stores(bytes) {
        return;
    }
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    let folder = dir.path().join("parts/led-5mm");
    fs::write(folder.join(std::ffi::OsStr::from_bytes(bytes)), "x").unwrap();
    let reply = text(&add(dir.path(), &["led-5mm"], true).unwrap());
    assert_eq!(
        reply,
        "wrote parts/led-5mm 0.1.1, replacing 0.1.1 with local edits\nparts.lock unchanged"
    );
    assert!(!dir.path().join("parts/.orbe-add").exists());
    assert_lock_agrees(dir.path());
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// An overwrite of a folder that holds a file orbe cannot read, or of such a file, stops before
/// it moves anything: orbe could not tell later that the copy it moved aside is unchanged. It
/// names the file, and once the file is readable the overwrite goes through.
#[test]
fn an_overwrite_of_a_file_orbe_cannot_read_stops_before_it_moves_anything() {
    for in_folder in [true, false] {
        let dir = project();
        add(dir.path(), &["led-5mm"], false).unwrap();
        let secret = if in_folder {
            dir.path().join("parts/led-5mm/secret.txt")
        } else {
            fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
            dir.path().join("parts/led-5mm")
        };
        fs::write(&secret, "KEEP-ME\n").unwrap();
        let before = tree(dir.path());
        set_mode(&secret, 0o000);
        let reply = add(dir.path(), &["led-5mm"], true).unwrap();
        set_mode(&secret, 0o644);
        assert_eq!(reply.code(), 1, "{}", text(&reply));
        let shown = project::shell_path(secret.as_os_str());
        assert!(
            text(&reply).starts_with(&format!("error[unreadable]: {shown} cannot be read")),
            "{}",
            text(&reply)
        );
        assert!(
            text(&reply).contains(&format!(
                "hint: make {shown} readable or move it out of {}, then run `{} led-5mm --overwrite` again",
                project::shell_path(secret.parent().unwrap().as_os_str()),
                project::add_command(Some(dir.path()))
            )),
            "{}",
            text(&reply)
        );
        assert_eq!(tree(dir.path()), before);
        let reply = add(dir.path(), &["led-5mm"], true).unwrap();
        assert_eq!(reply.code(), 0, "{}", text(&reply));
        assert_lock_agrees(dir.path());
    }
}

/// A file that becomes unreadable after the plan, before its folder moves aside, stops the add
/// there, and the add rolls back.
#[test]
fn a_file_made_unreadable_before_its_folder_moves_aside_rolls_the_add_back() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    let secret = dir.path().join("parts/led-5mm/secret.txt");
    fs::write(&secret, "KEEP-ME\n").unwrap();
    let before = tree(dir.path());
    let result = run_with(Some(dir.path()), &ask(&["led-5mm"], true), &mut |at| {
        if at == "do start the log" {
            set_mode(&secret, 0o000);
        }
        None
    });
    set_mode(&secret, 0o644);
    let error = result.err().expect("the add stopped").to_string();
    assert!(
        error.starts_with(&format!(
            "cannot read {}",
            project::shell_path(secret.as_os_str())
        )),
        "{error}"
    );
    assert!(error.contains("nothing was written"), "{error}");
    assert_eq!(tree(dir.path()), before);
}

/// Every path a hint names is one word that `sh` and `bash` both read back as that path, from
/// the folder the user ran in, whatever bytes the name holds.
#[test]
fn every_hinted_path_moves_its_entry_in_sh_and_bash_from_where_the_user_ran() {
    use std::os::unix::ffi::OsStrExt;
    let names: [&[u8]; 14] = [
        b"\xff\xfe it's",
        b"caf\xe9'x\\y",
        b"new\nline",
        b"tab\tx",
        b"ends in a newline\n",
        b"100%d\n%s",
        b"a'b",
        b"$HOME",
        b"`id`",
        b"sp ace",
        b"-rf",
        b"\\101",
        b"caf\xc3\xa9\x7f",
        b"\xe2\x80\xae",
    ];
    for name in names {
        // APFS refuses a name that is not UTF-8. The names it can store still run.
        if std::str::from_utf8(name).is_err() && !volume_stores(name) {
            continue;
        }
        for shell in ["sh", "bash"] {
            let dir = edited_project();
            stop_add(dir.path(), kill_at("do delete"));
            let planted = dir
                .path()
                .join("parts/.orbe-add")
                .join(std::ffi::OsStr::from_bytes(name));
            fs::write(&planted, "KEEP-ME\n").unwrap();
            let error = run(Some(dir.path()), &ask(&["resistor-0603"], false))
                .err()
                .expect("the add stopped")
                .to_string();
            let cwd = tempfile::tempdir().unwrap();
            move_as_hinted(cwd.path(), &error, shell);
            assert_eq!(kept(cwd.path()), vec!["moved-0".to_string()], "{error}");
            assert!(!planted.exists(), "{error}");
            add(dir.path(), &["resistor-0603"], false).unwrap();
            assert_replaced(dir.path());
        }
    }
}

/// Follows the hint in `first` as printed, from another folder, then runs
/// `orbe add -C <root> resistor-0603` and follows each hint the same way until the add goes
/// through.
fn follow_hints(root: &Path, first: &str) -> tempfile::TempDir {
    let cwd = tempfile::tempdir().unwrap();
    move_as_hinted(cwd.path(), first, "sh");
    for _ in 0..8 {
        match run(Some(root), &ask(&["resistor-0603"], false)) {
            Ok(reply) if !text(&reply).contains("stopped") => return cwd,
            Ok(reply) => move_as_hinted(cwd.path(), &text(&reply), "sh"),
            Err(error) => move_as_hinted(cwd.path(), &error.to_string(), "sh"),
        }
    }
    panic!("the hints never let the add through");
}

/// A folder swapped for a link while the staging folder is deleted is named as that folder.
/// Following the hint moves the link, and nothing where it points.
#[test]
fn a_folder_swapped_for_a_link_mid_delete_is_named_and_nothing_it_points_at_moves() {
    for swapped in ["old", "old/led-5mm"] {
        let dir = project();
        add(dir.path(), &["led-5mm"], false).unwrap();
        for name in ["u1.txt", "u2.txt"] {
            fs::write(dir.path().join("parts/led-5mm").join(name), "mine\n").unwrap();
        }
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(outside.path().join("led-5mm")).unwrap();
        for name in ["part.jsonc", "u1.txt", "u2.txt"] {
            fs::write(outside.path().join("led-5mm").join(name), "OUTSIDE\n").unwrap();
        }
        let target = match swapped {
            "old" => outside.path().to_path_buf(),
            _ => outside.path().join("led-5mm"),
        };
        let before = tree(outside.path());
        let stage = dir.path().join("parts/.orbe-add");
        let mut done = false;
        let reply = run_with(Some(dir.path()), &ask(&["led-5mm"], true), &mut |at| {
            if at.starts_with("do delete") && !done {
                done = true;
                fs::rename(stage.join(swapped), dir.path().join("moved-away")).unwrap();
                std::os::unix::fs::symlink(&target, stage.join(swapped)).unwrap();
            }
            None
        })
        .unwrap();
        let reply = text(&reply);
        assert!(
            reply.contains(&format!(
                "hint: move {} out of",
                project::shell_path(stage.join(swapped).as_os_str())
            )),
            "{swapped}: {reply}"
        );
        follow_hints(dir.path(), &reply);
        assert_eq!(tree(outside.path()), before, "{swapped}: {reply}");
        assert!(!stage.exists());
    }
}

/// The staging folder renamed and made again, or swapped for a link, while orbe deletes it.
/// The reply never calls it an empty folder, and following the hint keeps what is there.
#[test]
fn a_staging_folder_swapped_mid_delete_is_described_as_it_is_now() {
    for link in [false, true] {
        let dir = project();
        add(dir.path(), &["led-5mm"], false).unwrap();
        edit(dir.path(), "led-5mm");
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("SECRET"), "KEEP-ME\n").unwrap();
        let before = tree(outside.path());
        let stage = dir.path().join("parts/.orbe-add");
        let mut done = false;
        let reply = run_with(Some(dir.path()), &ask(&["led-5mm"], true), &mut |at| {
            if at.starts_with("do delete") && !done {
                done = true;
                fs::rename(&stage, dir.path().join("moved-away")).unwrap();
                if link {
                    std::os::unix::fs::symlink(outside.path(), &stage).unwrap();
                } else {
                    fs::create_dir(&stage).unwrap();
                    fs::write(stage.join("SECRET"), "KEEP-ME\n").unwrap();
                }
            }
            None
        })
        .unwrap();
        let reply = text(&reply);
        assert!(!reply.contains("empty folder"), "{reply}");
        assert!(
            reply.contains(&format!(
                "hint: move {} out of",
                project::shell_path(stage.as_os_str())
            )),
            "{reply}"
        );
        let cwd = follow_hints(dir.path(), &reply);
        assert_eq!(tree(outside.path()), before, "{reply}");
        if !link {
            assert_eq!(kept(cwd.path()), vec!["moved-0/SECRET".to_string()]);
        }
        assert!(!stage.exists());
    }
}

/// `--diff` says the next add stops on an error it hits reading the staging folder, as that
/// add does.
#[test]
fn diff_names_a_read_error_in_the_staging_folder_as_the_next_add_hits_it() {
    let dir = edited_project();
    stop_add(dir.path(), kill_at("do delete"));
    let old = dir.path().join("parts/.orbe-add/old");
    set_mode(&old, 0o000);
    let request = Request {
        diff: true,
        ..ask(&["resistor-0603"], false)
    };
    let preview = text(&run(Some(dir.path()), &request).unwrap());
    let error = run(Some(dir.path()), &ask(&["resistor-0603"], false))
        .err()
        .expect("the add stopped")
        .to_string();
    set_mode(&old, 0o755);
    let first = error.lines().next().unwrap();
    assert!(
        first.ends_with("Permission denied (os error 13)"),
        "{error}"
    );
    assert!(
        preview.contains(&format!("without --diff hits `{first}` and stops")),
        "{preview}\n{error}"
    );
    add(dir.path(), &["resistor-0603"], false).unwrap();
    assert_replaced(dir.path());
}

/// A name that is not UTF-8, put in the staging folder before or after the clearing list, is
/// named so that the printed `mv` moves it, and the add then finishes.
#[test]
fn a_name_that_is_not_utf8_is_printed_so_a_shell_finds_it() {
    use std::os::unix::ffi::OsStrExt;
    let bytes = b"\xff\xfe it's";
    // APFS refuses a name that is not UTF-8, so the printed `mv` cannot be tried here.
    if !volume_stores(bytes) {
        return;
    }
    for point in ["do clean", "do delete"] {
        let dir = edited_project();
        stop_add(dir.path(), kill_at(point));
        let name = std::ffi::OsStr::from_bytes(bytes);
        fs::write(dir.path().join("parts/.orbe-add").join(name), "KEEP-ME\n").unwrap();
        let error = diff_predicts(dir.path()).err().unwrap().to_string();
        assert!(!error.contains('\u{fffd}'), "{point}: {error}");
        move_as_hinted(dir.path(), &error, "sh");
        assert_eq!(kept(dir.path()), vec!["moved-0".to_string()], "{point}");
        diff_predicts(dir.path()).unwrap();
        assert_replaced(dir.path());
    }
}

/// Bytes changed after the clearing list, and a link swapped in for a folder, stop the run.
/// The path is named, and what it holds stays.
#[test]
fn a_path_that_changes_while_the_staging_folder_is_deleted_is_left() {
    for swap in [false, true] {
        let dir = project();
        add(dir.path(), &["led-5mm"], false).unwrap();
        for name in ["u1.txt", "u2.txt"] {
            fs::write(dir.path().join("parts/led-5mm").join(name), "mine\n").unwrap();
        }
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(outside.path().join("led-5mm")).unwrap();
        fs::write(outside.path().join("led-5mm/u1.txt"), "KEEP-ME\n").unwrap();
        let stage = dir.path().join("parts/.orbe-add");
        let mut done = false;
        let reply = run_with(Some(dir.path()), &ask(&["led-5mm"], true), &mut |at| {
            if at.starts_with("do delete") && !done {
                done = true;
                if swap {
                    fs::rename(stage.join("old"), dir.path().join("moved-old")).unwrap();
                    std::os::unix::fs::symlink(outside.path(), stage.join("old")).unwrap();
                } else {
                    for name in ["u1.txt", "u2.txt"] {
                        append_to(&stage.join("old/led-5mm").join(name), "KEEP-ME\n");
                    }
                }
            }
            None
        })
        .unwrap();
        let reply = text(&reply);
        assert!(reply.contains("finishing it stopped"), "{reply}");
        assert_eq!(
            fs::read_to_string(outside.path().join("led-5mm/u1.txt")).unwrap(),
            "KEEP-ME\n",
            "{reply}"
        );
        if !swap {
            assert_eq!(kept(dir.path()).len(), 2, "{reply}");
        }
        loop {
            match diff_predicts(dir.path()) {
                Ok(_) => break,
                Err(error) => move_as_hinted(dir.path(), &error.to_string(), "sh"),
            }
        }
        assert!(!stage.exists());
        assert_eq!(
            fs::read_to_string(outside.path().join("led-5mm/u1.txt")).unwrap(),
            "KEEP-ME\n"
        );
        if !swap {
            assert!(
                kept(dir.path())
                    .iter()
                    .all(|path| path.starts_with("moved-"))
            );
        }
    }
}

/// When the next add puts a part back and then stops, `--diff` says both, and the command it
/// names carries the ids it was given.
#[test]
fn diff_names_the_steps_the_next_add_takes_before_it_stops() {
    let dir = edited_project();
    stop_add(dir.path(), kill_after_commit());
    fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
    edit(dir.path(), "resistor-axial");
    let error = diff_predicts(dir.path()).err().unwrap().to_string();
    assert!(
        error.contains("before it stopped, orbe put back parts/led-5mm"),
        "{error}"
    );
    let request = Request {
        diff: true,
        ..ask(&["resistor-0603"], false)
    };
    let preview = text(&run(Some(dir.path()), &request).unwrap());
    let command = project::add_writing(
        &project::add_command(Some(dir.path())),
        &["resistor-0603".to_string()],
        false,
    )
    .expect("the id is named");
    assert!(
        preview.contains(&format!(
            "the next `{command}` without --diff names what it cannot prove the add made, and stops"
        )),
        "{preview}"
    );
    let dir = edited_project();
    stop_add(dir.path(), kill_after_commit());
    fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
    edit(dir.path(), "resistor-axial");
    let preview = text(&run(Some(dir.path()), &request).unwrap());
    let command = project::add_writing(
        &project::add_command(Some(dir.path())),
        &["resistor-0603".to_string()],
        false,
    )
    .expect("the id is named");
    assert!(
        preview.contains(&format!(
            "the next `{command}` without --diff puts back parts/led-5mm, then names what it cannot prove the add made, and stops"
        )),
        "{preview}"
    );
}

#[test]
fn a_forged_or_stale_log_claims_only_what_the_run_did() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    let stage = dir.path().join("parts/.orbe-add");
    fs::create_dir_all(stage.join("old/led-5mm")).unwrap();
    fs::write(
        stage.join("log"),
        "orbe add log 1\nplan existed - led-5mm\ncommitted\nclearing d:old d:old/led-5mm\n",
    )
    .unwrap();
    let line = diff_predicts(dir.path()).unwrap();
    assert_eq!(line, "deleted the staging folder of an interrupted add");

    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    edit(dir.path(), "led-5mm");
    stop_add_of(dir.path(), &["led-5mm"], kill_at("do delete"));
    let log = fs::read(stage_log(dir.path())).unwrap();
    recover(dir.path());
    fs::create_dir(dir.path().join("parts/.orbe-add")).unwrap();
    fs::write(stage_log(dir.path()), log).unwrap();
    let line = diff_predicts(dir.path()).unwrap();
    assert_eq!(
        line,
        "finished an interrupted add, which had written parts/led-5mm"
    );
}

fn stage_log(root: &Path) -> std::path::PathBuf {
    root.join("parts/.orbe-add/log")
}

/// An error on the last delete, the staging folder itself, comes after the log is gone. The
/// reply says so, and the next add deletes the empty folder.
#[test]
fn an_error_on_the_last_delete_says_the_log_is_gone() {
    let dir = project();
    add(dir.path(), &["led-5mm"], false).unwrap();
    edit(dir.path(), "led-5mm");
    let reply = run_with(Some(dir.path()), &ask(&["led-5mm"], true), &mut |at| {
        (at == "do delete parts/.orbe-add").then_some(Inject::Fail)
    })
    .unwrap();
    let reply = text(&reply);
    assert!(!reply.contains("keeps its log"), "{reply}");
    assert!(
        reply.contains("orbe had deleted its log and everything else in parts/.orbe-add, which is left as an empty folder"),
        "{reply}"
    );
    assert!(!stage_log(dir.path()).exists());
    assert_eq!(
        fs::read_dir(dir.path().join("parts/.orbe-add"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        recover(dir.path()),
        "deleted parts/.orbe-add, an empty folder an earlier add left"
    );
}
