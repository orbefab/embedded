use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::*;

fn ask(ids: &[&str]) -> Request {
    Request {
        ids: ids.iter().map(|id| (*id).to_string()).collect(),
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("scene.jsonc"), "{}\n").unwrap();
    dir
}

fn add(root: &Path, ids: &[&str]) -> Result<Reply, RunError> {
    run(Some(root), &ask(ids))
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

/// `orbe add` does not write `parts.lock`.
fn assert_lock_agrees(root: &Path) {
    assert!(!root.join("parts.lock").exists());
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

/// A project with a scene and no part folders, so an add copies into empty destinations.
fn edited_project() -> tempfile::TempDir {
    project()
}

const REPLACE: &[&str] = &["led-5mm", "resistor-axial", "tactile-switch-6mm"];
fn again() -> Again {
    Again::new(None, "orbe add tactile-switch-6mm".into())
}

fn assert_replaced(root: &Path) {
    for id in REPLACE {
        assert!(part_file(root, id).contains("\"version\""), "{id}");
        assert!(!part_file(root, id).contains("my edit"), "{id}");
    }
    assert_lock_agrees(root);
    assert!(!root.join("parts").join(txn::STAGE).exists());
}

/// Runs the add and stops it at the point the hook picks.
fn stop_add(root: &Path, pick: impl FnMut(&str) -> Option<Inject>) -> RunError {
    stop_add_of(root, REPLACE, pick)
}

fn stop_add_of(
    root: &Path,
    ids: &[&str],
    mut pick: impl FnMut(&str) -> Option<Inject>,
) -> RunError {
    run_with(Some(root), &ask(ids), &mut pick)
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
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1");
    let before = tree(dir.path());
    let again = add(dir.path(), &["led-5mm"]).unwrap();
    assert_eq!(
        text(&again),
        "error[part_exists]: parts/led-5mm is already led-5mm 0.1.1\n  hint: remove it and run `orbe add -C {} led-5mm`"
            .replace("{}", &dir.path().display().to_string())
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
        let result = run_with(Some(dir.path()), &ask(REPLACE), &mut |_| {
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
                        "finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm"
                    }),
                    "{line}"
                );
                assert_replaced(dir.path());
            }
            Ok(reply) => {
                assert!(point > 10, "only {point} points");
                assert!(
                    text(&reply).contains("wrote parts/led-5mm 0.1.1"),
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
    let error = run_with(Some(dir.path()), &ask(REPLACE), &mut |at| {
        (at == "do place tactile-switch-6mm").then_some(Inject::Fail)
    })
    .err()
    .unwrap();
    let message = error.to_string();
    assert!(
        message.contains("rolled back, so the project is as it was"),
        "{message}"
    );
    assert!(
        message.contains("removed the new parts/led-5mm"),
        "{message}"
    );
    assert!(
        message.contains("removed the new parts/resistor-axial"),
        "{message}"
    );
    assert_eq!(tree(dir.path()), before);
}

#[test]
fn the_next_add_says_it_rolled_back_an_interrupted_one() {
    let dir = edited_project();
    stop_add(dir.path(), kill_at("do place resistor-axial"));
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    let shown = text(&reply);
    assert!(shown.contains("rolled back an interrupted add"), "{shown}");
    assert!(shown.contains("wrote parts/led-5mm 0.1.1"), "{shown}");
    assert!(part_file(dir.path(), "led-5mm").contains("\"version\""));
    assert!(!dir.path().join("parts/.orbe-add").exists());
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
        add(dir.path(), &["led-5mm"]).err().unwrap().to_string(),
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
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert!(
        text(&reply).contains(
            "finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm"
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
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert!(
        text(&reply).contains("finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm"),
        "{}",
        text(&reply)
    );
    assert_replaced(dir.path());
}

/// With its log gone, nothing proves what the add moved, so recovery moves nothing.
#[test]
fn a_staging_folder_with_no_log_is_left_and_named() {
    let dir = project();
    add(dir.path(), &["led-5mm"]).unwrap();
    run_with(Some(dir.path()), &ask(&["resistor-axial"]), &mut |at| {
        at.starts_with("log placed").then_some(Inject::Kill)
    })
    .err()
    .unwrap();
    fs::remove_file(dir.path().join("parts/.orbe-add/log")).unwrap();
    let before = tree(dir.path());
    let error = unrooted(
        dir.path(),
        add(dir.path(), &["led-5mm"]).err().unwrap().to_string(),
    );
    assert!(
        error.starts_with("parts/.orbe-add has no log, so orbe cannot tell what the add that left it changed, and it left parts/.orbe-add/new, parts/.orbe-add/old as they are. that add may have written folders in parts/"),
        "{error}"
    );
    assert_eq!(tree(dir.path()), before);
    assert!(dir.path().join("parts/resistor-axial").is_dir());
    assert_lock_agrees(dir.path());
}

#[test]
fn an_empty_staging_folder_is_removed_without_a_claim() {
    let dir = project();
    add(dir.path(), &["led-5mm"]).unwrap();
    fs::create_dir(dir.path().join("parts/.orbe-add")).unwrap();
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    let shown = text(&reply);
    assert!(
        shown.starts_with("error[part_exists]: parts/led-5mm is already led-5mm 0.1.1\n"),
        "{shown}"
    );
    assert!(
        shown.contains("deleted parts/.orbe-add, an empty folder an earlier add left"),
        "{shown}"
    );
    assert!(shown.contains("remove it and run `orbe add"), "{shown}");
}

/// A second run while one holds the lock refuses at once and touches nothing.
#[test]
fn a_second_run_is_busy_while_one_holds_the_lock() {
    let dir = edited_project();
    let before = tree(dir.path());
    let held = held::hold(dir.path()).ok().unwrap();
    let request = ask(&["led-5mm"]);
    let reply = run(Some(dir.path()), &request).unwrap();
    assert_eq!(reply.code(), 1);
    assert_eq!(
        text(&reply),
        format!(
            "error[busy]: another `orbe add` is at work in this project, and holds its lock\n  hint: wait for it to finish, then run `orbe add -C {} led-5mm`",
            dir.path().display()
        )
    );
    assert_eq!(tree(dir.path()), before);
    drop(held);
    assert_eq!(tree(dir.path()), before);
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1");
    assert!(!dir.path().join(".orbe-add.lock").exists());
}

/// The lock is on the project folder, so no file stands for it. Deleting one frees nothing, and
/// a second run stays busy.
#[test]
fn a_deleted_lock_file_does_not_let_a_second_run_in() {
    let dir = edited_project();
    let before = tree(dir.path());
    let held = held::hold(dir.path()).ok().unwrap();
    assert!(!dir.path().join(".orbe-add.lock").exists());
    let request = ask(&["tactile-switch-6mm"]);
    let reply = run(Some(dir.path()), &request).unwrap();
    assert_eq!(reply.code(), 1, "{}", text(&reply));
    assert!(text(&reply).starts_with("error[busy]"), "{}", text(&reply));
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
    let diff = ask(&["led-5mm"]);
    run(Some(dir.path()), &diff).unwrap();
    add(dir.path(), &["led-5mm"]).unwrap();
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
    assert!(line.contains("removed the new parts/led-5mm"), "{line}");
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
        add(dir.path(), &["led-5mm"]).err().unwrap().to_string(),
    );
    assert!(
        error.starts_with(
            "parts/.orbe-add/log has a line this orbe does not write, `garbage`, so orbe left parts/.orbe-add as it is\n  hint: delete that line and the lines after it from parts/.orbe-add/log, then run `orbe add -C "
        ),
        "{error}"
    );
    assert!(!error.contains("back to parts/"), "{error}");
    fs::write(&log, kept).unwrap();
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert!(
        text(&reply).contains("finished an interrupted add"),
        "{}",
        text(&reply)
    );
    assert_replaced(dir.path());
}
#[test]
fn a_placed_copy_a_killed_recovery_moved_out_is_said_to_be_removed() {
    let dir = edited_project();
    let before = tree(dir.path());
    stop_add(dir.path(), kill_at("log committed"));
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
            add(dir.path(), &["led-5mm"]).err().unwrap().to_string(),
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
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert!(
        text(&reply).contains(
            "finished an interrupted add, which had written parts/led-5mm, parts/resistor-axial, parts/tactile-switch-6mm"
        ),
        "{}",
        text(&reply)
    );
    fs::remove_dir_all(&saved).unwrap();
    assert_replaced(dir.path());
}

/// `aside <id> -` says nothing was moved aside, so a folder at `old/<id>` is not the add's: it
/// is never put in place, never deleted, and the add's copy stays where the log placed it.
#[test]
fn a_folder_planted_in_old_is_never_installed_or_deleted() {
    let dir = project();
    run_with(
        Some(dir.path()),
        &ask(&["led-5mm"]),
        &mut kill_at("log committed"),
    )
    .err()
    .unwrap();
    let planted = dir.path().join("parts/.orbe-add/old/led-5mm");
    fs::create_dir_all(&planted).unwrap();
    fs::write(planted.join("note.txt"), "KEEP-ME\n").unwrap();
    let error = unrooted(
        dir.path(),
        add(dir.path(), &["led-5mm"]).err().unwrap().to_string(),
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
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert!(
        text(&reply).starts_with("wrote parts/led-5mm 0.1.1\nrolled back an interrupted add: removed the new parts/led-5mm"),
        "{}",
        text(&reply)
    );
    assert_eq!(
        fs::read_to_string(kept.join("note.txt")).unwrap(),
        "KEEP-ME\n"
    );
}
#[test]
fn a_link_at_a_part_path_is_left_alone_and_its_target_kept() {
    let dir = project();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(elsewhere.path().join("part.jsonc"), "mine\n").unwrap();
    fs::create_dir(dir.path().join("parts")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("parts/led-5mm")).unwrap();
    for _ in 0..2 {
        let reply = add(dir.path(), &["led-5mm"]).unwrap();
        let shown = text(&reply);
        assert!(shown.contains("error[part_exists]"), "{shown}");
        assert!(
            shown.contains("parts/led-5mm is already a link, not a folder"),
            "{shown}"
        );
    }
    assert_eq!(
        fs::read_to_string(elsewhere.path().join("part.jsonc")).unwrap(),
        "mine\n"
    );
    assert!(
        fs::symlink_metadata(dir.path().join("parts/led-5mm"))
            .unwrap()
            .file_type()
            .is_symlink()
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
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1");
}

/// `orbe check` names the folder of a scene of any name with `-C`, so a named folder is trusted.
#[test]
fn a_folder_named_with_c_is_trusted_and_a_missing_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("blinky.jsonc"), "{}\n").unwrap();
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1");

    let missing = dir.path().join("nowhere");
    let reply = add(&missing, &["led-5mm"]).unwrap();
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
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    assert_eq!(text(&reply), "wrote parts/led-5mm 0.1.1");
    edit(dir.path(), "led-5mm");
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    let shown = text(&reply);
    assert!(shown.contains("error[part_exists]"), "{shown}");
    assert!(
        shown.contains("parts/led-5mm is already led-5mm 0.1.1"),
        "{shown}"
    );
    assert!(part_file(dir.path(), "led-5mm").ends_with("// my edit\n"));
    assert_lock_agrees(dir.path());
}

/// An edit to the new copy after a crash past the commit stays. The next add does not replace it.
#[test]
fn an_edit_to_the_new_copy_after_the_commit_is_kept() {
    let dir = edited_project();
    stop_add(dir.path(), kill_after_commit());
    append_to(
        &dir.path().join("parts/led-5mm/part.jsonc"),
        "// after the crash\n",
    );
    let reply = add(dir.path(), &["led-5mm"]).unwrap();
    let shown = text(&reply);
    assert!(shown.contains("error[part_exists]"), "{shown}");
    assert!(part_file(dir.path(), "led-5mm").ends_with("// after the crash\n"));
    assert!(!dir.path().join("parts/.orbe-add").exists(), "{shown}");
}

/// What recovery of an interrupted add reports, without writing a new part.
fn diff_predicts(root: &Path) -> Result<String, RunError> {
    txn::recover(root, &again(), &mut |_| None)
        .map(|recovered| recovered.lines.join("\n"))
        .map_err(|stop| match stop {
            txn::Stop::Failed(error) => error,
            txn::Stop::Killed => unreachable!(),
        })
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
/// names them and stops. Once they are moved out, recovery finishes the add.
#[test]
fn bytes_put_in_a_half_deleted_staging_folder_are_never_deleted() {
    let mut cases = 0;
    for point in 0.. {
        for change in ["plant", "edit"] {
            let dir = edited_project();
            let mut committed = false;
            let mut seen = 0;
            let killed = run_with(Some(dir.path()), &ask(REPLACE), &mut |at| {
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
    let reply = text(&add(dir.path(), &["led-5mm"]).unwrap());
    assert_eq!(reply, "wrote parts/led-5mm 0.1.1");
    let mut after = tree(&dir.path().join("parts"));
    after.retain(|path, _| !path.starts_with("led-5mm"));
    assert_eq!(after, before);
}

/// A clearing list in a log proves only the bytes it hashes. A file it does not list, and a
/// listed file with other bytes, both stay.
#[test]
fn a_clearing_list_deletes_only_the_bytes_it_hashes() {
    let dir = project();
    add(dir.path(), &["led-5mm"]).unwrap();
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
    ] {
        for planted in [false, true] {
            let dir = project();
            add(dir.path(), &["resistor-axial"]).unwrap();
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
            let error = run(Some(dir.path()), &ask(&["resistor-0603"]))
                .err()
                .expect("the add stopped")
                .to_string();
            let cwd = tempfile::tempdir().unwrap();
            move_as_hinted(cwd.path(), &error, shell);
            assert_eq!(kept(cwd.path()), vec!["moved-0".to_string()], "{error}");
            assert!(!planted.exists(), "{error}");
            add(dir.path(), &["resistor-0603"]).unwrap();
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
        match run(Some(root), &ask(&["resistor-0603"])) {
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
    let dir = project();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(outside.path().join("led-5mm")).unwrap();
    for name in ["part.jsonc", "u1.txt", "u2.txt"] {
        fs::write(outside.path().join("led-5mm").join(name), "OUTSIDE\n").unwrap();
    }
    let before = tree(outside.path());
    let stage = dir.path().join("parts/.orbe-add");
    let mut done = false;
    let reply = run_with(Some(dir.path()), &ask(&["led-5mm"]), &mut |at| {
        if at.starts_with("do delete") && !done {
            done = true;
            fs::rename(stage.join("old"), dir.path().join("moved-away")).unwrap();
            std::os::unix::fs::symlink(outside.path(), stage.join("old")).unwrap();
        }
        None
    })
    .unwrap();
    let reply = text(&reply);
    assert!(
        reply.contains(&format!(
            "hint: move {} out of",
            project::shell_path(stage.join("old").as_os_str())
        )),
        "{reply}"
    );
    follow_hints(dir.path(), &reply);
    assert_eq!(tree(outside.path()), before, "{reply}");
    assert!(!stage.exists());
}

/// The staging folder renamed and made again, or swapped for a link, while orbe deletes it.
/// The reply never calls it an empty folder, and following the hint keeps what is there.
#[test]
fn a_staging_folder_swapped_mid_delete_is_described_as_it_is_now() {
    for link in [false, true] {
        let dir = project();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("SECRET"), "KEEP-ME\n").unwrap();
        let before = tree(outside.path());
        let stage = dir.path().join("parts/.orbe-add");
        let mut done = false;
        let reply = run_with(Some(dir.path()), &ask(&["led-5mm"]), &mut |at| {
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

#[test]
fn a_forged_or_stale_log_claims_only_what_the_run_did() {
    let dir = project();
    add(dir.path(), &["led-5mm"]).unwrap();
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
    let reply = run_with(Some(dir.path()), &ask(&["led-5mm"]), &mut |at| {
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
