use super::*;

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const L: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const M: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

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

fn seen(dest: Option<&str>, new: bool, old: bool) -> Seen {
    let mut seen = Seen::default();
    seen.parts.insert(
        "led".into(),
        Here {
            dest: dest.map(str::to_string),
            new,
            old,
        },
    );
    seen.names = ["log", "new", "old"].map(str::to_string).to_vec();
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
        Record::Cleaned("led-5mm".into()),
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
        Read::Unknown(line) if line == "moved led"
    ));
}

#[test]
fn before_anything_moved_recovery_only_cleans() {
    let log = log(LockChange::Kept, staged());
    assert_eq!(next(&log, &seen(Some(A), true, false)), Next::Clean);
    assert_eq!(summary(&log), None);
}

#[test]
fn a_move_aside_the_log_missed_is_put_back() {
    let log = log(LockChange::Kept, staged());
    assert_eq!(
        next(&log, &seen(None, true, true)),
        act(
            Effect::Restore("led".into()),
            Record::Restored("led".into())
        )
    );
}

#[test]
fn a_placed_copy_with_the_logged_hash_is_moved_back_out() {
    for records in [aside(staged()), placed(aside(staged()))] {
        let log = log(LockChange::Kept, records);
        assert_eq!(
            next(&log, &seen(Some(B), false, true)),
            act(
                Effect::Unplace("led".into()),
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
    let (lines, hint) = problem_text(&log, &[Problem::Conflict("led".into())], "orbe add x");
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
            new: true,
            old: true,
        },
    );
    seen.names.extend(["new/res".into(), "old/res".into()]);
    assert_eq!(
        next(&plan_log, &seen),
        act(
            Effect::Restore("res".into()),
            Record::Restored("res".into())
        )
    );
    plan_log.records.push(Record::Restored("res".into()));
    seen.parts.insert(
        "res".into(),
        Here {
            dest: Some(A.into()),
            new: true,
            old: false,
        },
    );
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
            Effect::Restore("led".into()),
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
            Effect::Unplace("led".into()),
            Record::Unplaced("led".into())
        )
    );
}

/// A recovery killed between a move and its log line: the next one sees the move done.
#[test]
fn a_recovery_step_the_log_missed_is_not_repeated() {
    let log = log(LockChange::Kept, placed(aside(staged())));
    assert_eq!(
        next(&log, &seen(None, true, true)),
        act(
            Effect::Restore("led".into()),
            Record::Restored("led".into())
        )
    );
    assert_eq!(
        next(&log, &seen(Some(A), true, false)),
        act(Effect::None, Record::Restored("led".into()))
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
    assert_eq!(
        next(&log, &seen(Some(B), false, true)),
        act(Effect::Delete("led".into()), Record::Cleaned("led".into()))
    );
    assert_eq!(
        next(&log, &seen(None, false, true)),
        act(
            Effect::Restore("led".into()),
            Record::Restored("led".into())
        )
    );
    assert_eq!(
        next(&log, &seen(Some(C), false, true)),
        Next::Stop(vec![Problem::Conflict("led".into())])
    );
    assert_eq!(
        next(&log, &seen(Some(B), false, false)),
        act(Effect::None, Record::Cleaned("led".into()))
    );
    assert_eq!(
        next(&log, &seen(Some(A), false, false)),
        act(Effect::None, Record::Restored("led".into()))
    );
}

#[test]
fn a_finished_add_says_what_it_wrote_and_deleted() {
    let mut records = placed(aside(staged()));
    records.extend([
        Record::LockWritten,
        Record::Committed,
        Record::Cleaned("led".into()),
    ]);
    let log = log(LockChange::Replace(L.into()), records);
    assert_eq!(next(&log, &seen(Some(B), false, false)), Next::Clean);
    assert_eq!(
        summary(&log).as_deref(),
        Some(
            "finished an interrupted add, which had written parts/led, parts.lock, and deleted the copies it replaced: parts/led"
        )
    );
}

#[test]
fn a_name_no_step_made_is_never_deleted() {
    let mut records = placed(aside(staged()));
    records.push(Record::Committed);
    records.push(Record::Cleaned("led".into()));
    let log = log(LockChange::Kept, records);
    let mut seen = seen(Some(B), false, false);
    seen.names.push("old/precious".into());
    assert_eq!(
        next(&log, &seen),
        Next::Stop(vec![Problem::Foreign(vec!["old/precious".into()])])
    );
    seen.names.retain(|name| name != "old/precious");
    seen.names.push("new/led".into());
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
        names: vec!["log".into(), "parts.lock".into()],
        ..Seen::default()
    };
    assert_eq!(
        next(&lock_log, &with(Some(L), Some(M))),
        act(Effect::LockBack, Record::LockRestored)
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
        act(Effect::LockBack, Record::LockRestored)
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
        act(Effect::LockBack, Record::LockRestored)
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
        act(Effect::LockRemove, Record::LockRestored)
    );
}
