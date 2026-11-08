use super::*;

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const L: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const M: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

fn again() -> Again {
    Again::new(None, "orbe add x".into())
}

/// One id, `led`, whose new copy hashes to `B` and whose copy from before the add hashes to `A`.
fn log(lock: LockChange, records: Vec<Record>) -> Log {
    Log {
        plan: Plan {
            parts_created: false,
            lock,
            ids: vec!["led".into()],
        },
        records,
    }
}

fn staged() -> Vec<Record> {
    vec![Record::Staged("led".into(), B.into())]
}

fn aside(mut records: Vec<Record>) -> Vec<Record> {
    records.push(Record::Aside("led".into(), Some(A.into())));
    records
}

fn placed(mut records: Vec<Record>) -> Vec<Record> {
    records.push(Record::Placed("led".into(), B.into()));
    records
}

/// `parts/led` holds `dest`. With `new`, `new/led` holds the staged copy `B`, and with `old`,
/// `old/led` holds the copy from before the add, `A`.
fn seen(dest: Option<&str>, new: bool, old: bool) -> Seen {
    let mut seen = Seen::default();
    seen.parts.insert(
        "led".into(),
        Here {
            dest: dest.map(str::to_string),
            new: new.then(|| B.into()),
            old: old.then(|| A.into()),
        },
    );
    seen.names = ["log", "new", "old"].map(OsString::from).to_vec();
    if new {
        seen.names.push("new/led".into());
    }
    if old {
        seen.names.push("old/led".into());
    }
    seen
}

fn act(effect: Effect, record: Record) -> Next {
    Next::Act(effect, record)
}

#[test]
fn the_log_reads_back_what_it_wrote_and_ignores_a_torn_last_line() {
    let plan = Plan {
        parts_created: true,
        lock: LockChange::Replace(L.into()),
        ids: vec!["led-5mm".into(), "resistor-axial".into()],
    };
    let records = vec![
        Record::Staged("led-5mm".into(), B.into()),
        Record::SavedLock(M.into()),
        Record::Aside("led-5mm".into(), Some(A.into())),
        Record::Aside("resistor-axial".into(), None),
        Record::Placed("led-5mm".into(), B.into()),
        Record::LockWritten,
        Record::Committed,
        Record::Unplaced("led-5mm".into()),
        Record::Restored("led-5mm".into()),
        Record::Kept("led-5mm".into()),
        Record::LockRestored,
        Record::LockKept,
    ];
    let mut text = plan.text();
    for record in &records {
        text.push_str(&record.line());
        text.push('\n');
    }
    text.push_str("commit");
    let Read::Log(read) = read(&text) else {
        panic!("{text}")
    };
    assert_eq!(read.plan, plan);
    assert_eq!(read.records, records);

    assert!(matches!(super::read(""), Read::Torn));
    assert!(matches!(super::read("orbe add lo"), Read::Torn));
    assert!(matches!(
        super::read("orbe add log 1\nplan exis"),
        Read::Torn
    ));
    assert!(matches!(
        super::read("orbe add journal 1\nparts existed\n"),
        Read::Unknown(_)
    ));
    assert!(matches!(
        super::read(&format!("{}moved led\n", plan.text())),
        Read::UnknownStep(line) if line == "moved led"
    ));
}

#[test]
fn before_anything_moved_recovery_only_cleans() {
    let log = log(LockChange::Kept, staged());
    assert_eq!(next(&log, &seen(Some(A), true, false)), Next::Clean);
    assert_eq!(summary(&log), None);
}

#[test]
fn a_copy_moved_aside_is_put_back_while_it_has_the_logged_hash() {
    let log = log(LockChange::Kept, aside(staged()));
    assert_eq!(
        next(&log, &seen(None, true, true)),
        act(
            Effect::Restore("led".into(), A.into()),
            Record::Restored("led".into())
        )
    );
    assert_eq!(next(&log, &seen(Some(A), true, false)), Next::Clean);
}

/// Without an `aside` line nothing was moved aside, so a folder in `old/` is not the add's.
#[test]
fn a_folder_in_old_that_no_aside_line_made_is_named_and_left() {
    for records in [staged(), {
        let mut records = staged();
        records.push(Record::Aside("led".into(), None));
        records.push(Record::Placed("led".into(), B.into()));
        records
    }] {
        let log = log(LockChange::Kept, records);
        assert_eq!(
            next(&log, &seen(Some(B), false, true)),
            Next::Stop(vec![Problem::Foreign(vec!["old/led".into()])])
        );
    }
}

#[test]
fn a_placed_copy_with_the_logged_hash_is_moved_back_out() {
    for records in [aside(staged()), placed(aside(staged()))] {
        let log = log(LockChange::Kept, records);
        assert_eq!(
            next(&log, &seen(Some(B), false, true)),
            act(
                Effect::Unplace("led".into(), B.into()),
                Record::Unplaced("led".into())
            )
        );
    }
}

#[test]
fn an_edit_after_the_crash_stops_the_rollback_of_that_id() {
    let log = log(LockChange::Kept, placed(aside(staged())));
    let next = next(&log, &seen(Some(C), false, true));
    assert_eq!(next, Next::Stop(vec![Problem::Conflict("led".into())]));
    let (lines, hint) = problem_text(&log, &[Problem::Conflict("led".into())], &again());
    assert_eq!(
        lines,
        "parts/led changed after the interrupted add placed its new copy there, and the copy from before the add is in parts/.orbe-add/old/led. orbe left both"
    );
    assert_eq!(
        hint,
        "keep one: delete parts/led to get back parts/.orbe-add/old/led, or delete parts/.orbe-add/old/led to keep parts/led as it is. then run `orbe add x` again"
    );
}

/// The id in conflict waits, and every other id is put back before recovery stops.
#[test]
fn a_conflict_holds_only_its_own_id() {
    let mut plan_log = log(LockChange::Kept, placed(aside(staged())));
    plan_log.plan.ids.push("res".into());
    plan_log.records.extend([
        Record::Staged("res".into(), B.into()),
        Record::Aside("res".into(), Some(A.into())),
    ]);
    let mut seen = seen(Some(C), false, true);
    seen.parts.insert(
        "res".into(),
        Here {
            dest: None,
            new: Some(B.into()),
            old: Some(A.into()),
        },
    );
    seen.names.extend(["new/res".into(), "old/res".into()]);
    assert_eq!(
        next(&plan_log, &seen),
        act(
            Effect::Restore("res".into(), A.into()),
            Record::Restored("res".into())
        )
    );
    plan_log.records.push(Record::Restored("res".into()));
    seen.parts.insert(
        "res".into(),
        Here {
            dest: Some(A.into()),
            new: Some(B.into()),
            old: None,
        },
    );
    seen.names.retain(|name| name != "old/res");
    assert_eq!(
        next(&plan_log, &seen),
        Next::Stop(vec![Problem::Conflict("led".into())])
    );
    assert_eq!(steps_taken(&plan_log, 4), ["put back parts/res"]);
}

#[test]
fn each_way_out_of_a_conflict_finishes_the_rollback() {
    let log = log(LockChange::Kept, placed(aside(staged())));
    assert_eq!(
        next(&log, &seen(None, false, true)),
        act(
            Effect::Restore("led".into(), A.into()),
            Record::Restored("led".into())
        )
    );
    assert_eq!(
        next(&log, &seen(Some(C), false, false)),
        act(Effect::None, Record::Kept("led".into()))
    );
}

#[test]
fn an_edit_where_nothing_was_before_is_kept() {
    let mut records = staged();
    records.push(Record::Aside("led".into(), None));
    records.push(Record::Placed("led".into(), B.into()));
    let log = log(LockChange::Kept, records);
    assert_eq!(
        next(&log, &seen(Some(C), false, false)),
        act(Effect::None, Record::Kept("led".into()))
    );
    assert_eq!(
        next(&log, &seen(Some(B), false, false)),
        act(
            Effect::Unplace("led".into(), B.into()),
            Record::Unplaced("led".into())
        )
    );
}

/// A recovery killed between a move and its log line: the next one sees the move done, and
/// logs it as its own step.
#[test]
fn a_recovery_step_the_log_missed_is_not_repeated() {
    let log = log(LockChange::Kept, placed(aside(staged())));
    assert_eq!(
        next(&log, &seen(None, true, true)),
        act(Effect::None, Record::Unplaced("led".into()))
    );
    let mut unplaced = placed(aside(staged()));
    unplaced.push(Record::Unplaced("led".into()));
    let unplaced = super::Log {
        records: unplaced,
        ..log
    };
    assert_eq!(
        summary(&unplaced).as_deref(),
        Some("removed the new parts/led")
    );
}

/// Before the commit, a placed copy is moved out only while the copy it replaced is in `old/`
/// with its logged hash. Otherwise the project's only copy stays.
#[test]
fn a_placed_copy_stays_when_the_copy_it_replaced_is_not_there_to_put_back() {
    let log = log(LockChange::Kept, placed(aside(staged())));
    assert_eq!(
        next(&log, &seen(Some(B), false, false)),
        Next::Stop(vec![Problem::Missing("led".into())])
    );
    let mut changed = seen(Some(B), false, true);
    changed.parts.get_mut("led").unwrap().old = Some(C.into());
    assert_eq!(
        next(&log, &changed),
        Next::Stop(vec![Problem::Changed(vec!["old/led".into()])])
    );
    let (lines, hint) = problem_text(&log, &[Problem::Missing("led".into())], &again());
    assert_eq!(
        lines,
        "parts/led holds the new copy the interrupted add placed, and the copy it replaced is not in parts/.orbe-add/old/led, so orbe cannot put that back. orbe left parts/led"
    );
    assert_eq!(
        hint,
        "delete parts/led, whose files are the catalog's, to finish the rollback. then run `orbe add x` again"
    );
}

/// The user's copy, untouched at `parts/led`, is never taken for the new one, whatever the
/// staging folder holds.
#[test]
fn a_folder_the_add_never_moved_is_left_alone() {
    let log = log(LockChange::Kept, staged());
    assert_eq!(next(&log, &seen(Some(A), false, false)), Next::Clean);
    let log = super::Log {
        records: vec![],
        ..log
    };
    assert_eq!(next(&log, &seen(Some(A), false, false)), Next::Clean);
}

#[test]
fn a_done_rollback_says_what_it_put_back() {
    let mut records = placed(aside(staged()));
    records.push(Record::Unplaced("led".into()));
    records.push(Record::Restored("led".into()));
    let log = log(LockChange::Kept, records);
    assert_eq!(next(&log, &seen(Some(A), true, false)), Next::Clean);
    assert_eq!(summary(&log).as_deref(), Some("put back parts/led"));
}

#[test]
fn after_the_commit_a_saved_copy_is_deleted_only_beside_the_new_copy() {
    let mut records = placed(aside(staged()));
    records.push(Record::Committed);
    let log = log(LockChange::Kept, records);
    assert_eq!(next(&log, &seen(Some(B), false, true)), Next::Clean);
    assert_eq!(
        next(&log, &seen(None, false, true)),
        act(
            Effect::Restore("led".into(), A.into()),
            Record::Restored("led".into())
        )
    );
    assert_eq!(
        next(&log, &seen(Some(C), false, true)),
        Next::Stop(vec![Problem::Conflict("led".into())])
    );
    assert_eq!(next(&log, &seen(Some(B), false, false)), Next::Clean);
    assert_eq!(
        next(&log, &seen(Some(A), false, false)),
        act(Effect::None, Record::Restored("led".into()))
    );
    let summary = finished(&log, &["parts/led".into()], &[]).unwrap();
    assert!(!summary.contains("deleted"), "{summary}");
}

/// After the commit, a saved copy that changed, or a staging folder that is not a folder, is
/// named and left, and nothing is moved.
#[test]
fn after_the_commit_a_changed_saved_copy_or_a_linked_old_is_left() {
    let mut records = placed(aside(staged()));
    records.push(Record::Committed);
    let log = log(LockChange::Kept, records);
    let mut changed = seen(Some(B), false, true);
    changed.parts.get_mut("led").unwrap().old = Some(C.into());
    assert_eq!(
        next(&log, &changed),
        Next::Stop(vec![Problem::Changed(vec!["old/led".into()])])
    );
    let (lines, hint) = problem_text(&log, &[Problem::Changed(vec!["old/led".into()])], &again());
    assert_eq!(
        lines,
        "parts/.orbe-add/old/led changed after the add moved the copy from parts/led there. orbe left it, and parts/led as it is"
    );
    assert_eq!(
        hint,
        "move parts/.orbe-add/old/led out of parts/.orbe-add to keep it. then run `orbe add x` again"
    );

    let mut linked = seen(Some(B), false, false);
    linked.odd = vec!["old".into()];
    assert_eq!(
        next(&log, &linked),
        Next::Stop(vec![Problem::Foreign(vec!["old".into()])])
    );
}

#[test]
fn a_finished_add_says_what_it_wrote_and_deleted() {
    let mut records = placed(aside(staged()));
    records.extend([Record::LockWritten, Record::Committed]);
    let log = log(LockChange::Replace(L.into()), records);
    assert_eq!(next(&log, &seen(Some(B), false, true)), Next::Clean);
    assert_eq!(
        finished(
            &log,
            &["parts/led".into(), "parts.lock".into()],
            &["led".into()]
        )
        .as_deref(),
        Some(
            "finished an interrupted add, which had written parts/led, parts.lock, and deleted the old copy of parts/led it replaced"
        )
    );
}

#[test]
fn a_name_no_step_made_is_never_deleted() {
    let mut records = placed(aside(staged()));
    records.push(Record::Committed);
    let log = log(LockChange::Kept, records);
    let mut seen = seen(Some(B), false, false);
    seen.names.push("old/precious".into());
    assert_eq!(
        next(&log, &seen),
        Next::Stop(vec![Problem::Foreign(vec!["old/precious".into()])])
    );
    seen.names.retain(|name| name != "old/precious");
    seen.names.push("new/led".into());
    seen.parts.get_mut("led").unwrap().new = Some(B.into());
    assert_eq!(
        next(&log, &seen),
        Next::Stop(vec![Problem::Foreign(vec!["new/led".into()])])
    );
}

#[test]
fn the_lock_is_put_back_only_while_it_holds_what_the_add_wrote() {
    let mut records = vec![Record::SavedLock(M.into())];
    records.push(Record::LockWritten);
    let lock_log = Log {
        plan: Plan {
            parts_created: false,
            lock: LockChange::Replace(L.into()),
            ids: vec![],
        },
        records,
    };
    let with = |lock: Option<&str>, saved: Option<&str>| Seen {
        lock: lock.map(str::to_string),
        saved_lock: saved.map(str::to_string),
        names: ["log"]
            .into_iter()
            .chain(saved.map(|_| "parts.lock"))
            .map(OsString::from)
            .collect(),
        ..Seen::default()
    };
    assert_eq!(
        next(&lock_log, &with(Some(L), Some(M))),
        act(
            Effect::LockBack(M.into(), Some(L.into())),
            Record::LockRestored
        )
    );
    assert_eq!(
        next(&lock_log, &with(Some(M), Some(M))),
        act(Effect::None, Record::LockRestored)
    );
    assert_eq!(
        next(&lock_log, &with(Some(C), Some(M))),
        Next::Stop(vec![Problem::LockConflict])
    );
    assert_eq!(
        next(&lock_log, &with(None, Some(M))),
        act(Effect::LockBack(M.into(), None), Record::LockRestored)
    );
    assert_eq!(
        next(&lock_log, &with(Some(C), None)),
        act(Effect::None, Record::LockKept)
    );

    let unwritten = Log {
        records: vec![Record::SavedLock(M.into())],
        ..lock_log
    };
    assert_eq!(next(&unwritten, &with(Some(C), Some(M))), Next::Clean);
    assert_eq!(
        next(&unwritten, &with(Some(L), Some(M))),
        act(
            Effect::LockBack(M.into(), Some(L.into())),
            Record::LockRestored
        )
    );
    assert_eq!(
        next(&unwritten, &with(Some(L), Some(C))),
        Next::Stop(vec![Problem::Changed(vec!["parts.lock".into()])])
    );
    assert_eq!(
        next(&unwritten, &with(Some(L), None)),
        act(Effect::None, Record::LockStays)
    );
    let stays = Log {
        plan: unwritten.plan.clone(),
        records: vec![Record::SavedLock(M.into()), Record::LockStays],
    };
    assert_eq!(
        summary(&stays).as_deref(),
        Some(
            "left the parts.lock the add wrote, since the copy from before it was not in parts/.orbe-add"
        )
    );

    let added = Log {
        plan: Plan {
            parts_created: false,
            lock: LockChange::Add(L.into()),
            ids: vec![],
        },
        records: vec![],
    };
    assert_eq!(
        next(&added, &with(Some(L), None)),
        act(Effect::LockRemove(L.into()), Record::LockRestored)
    );
}

/// A run killed inside staging leaves `new/<id>` part written and unlogged. Only the one write
/// after the last line can be that, so that one goes and any other is named.
#[test]
fn only_the_write_after_the_last_line_may_be_unlogged() {
    let mut half = seen(None, false, false);
    half.names.push("new/led".into());
    half.parts.get_mut("led").unwrap().new = Some(C.into());
    assert_eq!(
        next(&log(LockChange::Kept, vec![]), &half),
        Next::Stop(vec![Problem::Changed(vec!["new/led".into()])])
    );
    half.unlogged_is_prefix = true;
    assert_eq!(next(&log(LockChange::Kept, vec![]), &half), Next::Clean);
    assert_eq!(
        next(
            &log(LockChange::Kept, vec![Record::Aside("led".into(), None)]),
            &half
        ),
        Next::Stop(vec![Problem::Foreign(vec!["new/led".into()])])
    );
    assert_eq!(
        next(&log(LockChange::Kept, staged()), &half),
        Next::Stop(vec![Problem::Changed(vec!["new/led".into()])])
    );
}

/// A clearing line reads back every name it lists, whatever bytes the name holds, and never
/// a path outside the staging folder or the log itself.
#[test]
fn a_clearing_line_reads_back_its_names_and_only_paths_below_the_folder() {
    let entries = vec![
        Entry {
            path: "old/led".into(),
            kind: Kind::Folder,
        },
        Entry {
            path: "old/led/my notes: 100%\nµ.txt".into(),
            kind: Kind::File(A.into()),
        },
        Entry {
            path: "old/led/sheet".into(),
            kind: Kind::Link("../a b/c:d".into()),
        },
        Entry {
            path: "old/led/fifo".into(),
            kind: Kind::Other,
        },
        Entry {
            path: OsString::from_vec(b"old/led/caf\xe9".to_vec()),
            kind: Kind::File(A.into()),
        },
    ];
    let line = Record::Clearing(entries.clone()).line();
    assert!(
        !line.contains('\n') && line.split(' ').count() == 6,
        "{line}"
    );
    assert_eq!(Record::parse(&line), Some(Record::Clearing(entries)));
    assert_eq!(
        Record::parse("clearing"),
        Some(Record::Clearing(Vec::new()))
    );
    for word in [
        "d:../x",
        "d:/x",
        "d:a//b",
        "d:log",
        "f:a",
        "d:a%2",
        "x:a",
        "d:old/./a",
        "u:a:12.3.1700000000.5.1700000001.6",
    ] {
        assert_eq!(Record::parse(&format!("clearing {word}")), None, "{word}");
    }
}
