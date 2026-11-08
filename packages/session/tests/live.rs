//! The session loop as a session process drives it: a step that reports each slice and can be
//! stopped between slices, and commands and replies that cross a process boundary unchanged.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use orbe_session::{Code, Command, Data, Live, Ref, Reply};

fn first_target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/first-target")
}

fn command(line: &str) -> Command {
    Command::parse(line, &first_target()).unwrap().unwrap()
}

fn pressed_at_100ms() -> Live {
    let mut live = Live::new("t");
    for line in ["open", "step 100ms", "press @btn1"] {
        let reply = live.run(&command(line));
        assert!(reply.ok(), "{line}: {}", reply.text());
    }
    live
}

#[test]
fn a_step_reports_each_slice_and_ends_where_an_unwatched_step_does() {
    let mut watched = pressed_at_100ms();
    let mut seen = Vec::new();
    let reply = watched.run_with(&command("step 5ms"), &mut |progress| {
        assert_eq!(
            (progress.from_ps, progress.end_ps),
            (100_000_000_000, 105_000_000_000)
        );
        let led = progress.get(&Ref::parse("@led1").unwrap()).unwrap();
        seen.push((progress.t_ps(), led.value.text()));
        ControlFlow::Continue(())
    });
    let times: Vec<u64> = seen.iter().map(|(t, _)| *t).collect();
    assert_eq!(
        times,
        (101..=105).map(|ms| ms * 1_000_000_000).collect::<Vec<_>>()
    );
    assert!(seen[4].1.starts_with("on i="), "{seen:?}");

    let mut plain = pressed_at_100ms();
    assert_eq!(reply, plain.run(&command("step 5ms")));
    let get = command("get @led1");
    assert_eq!(watched.run(&get), plain.run(&get));
}

#[test]
fn a_cancelled_step_stops_between_slices_and_changes_no_result() {
    let mut cancelled = pressed_at_100ms();
    let reply = cancelled.run_with(&command("step 5ms"), &mut |progress| {
        if progress.t_ps() == 102_000_000_000 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    let error = reply.error.as_ref().unwrap();
    assert_eq!(error.code, Code::Cancelled);
    assert_eq!(error.message, "the step was cancelled at t=102ms");
    assert_eq!(reply.t_ps, 102_000_000_000);
    let Data::Stepped(stepped) = &reply.data else {
        panic!("{reply:?}")
    };
    assert!(!stepped.changes.is_empty(), "the changes so far");
    assert!(cancelled.run(&command("step 3ms")).ok());

    let mut whole = pressed_at_100ms();
    assert!(whole.run(&command("step 5ms")).ok());
    for line in ["get @led1", "get @nano.D13", "get @btn1.i"] {
        assert_eq!(
            cancelled.run(&command(line)),
            whole.run(&command(line)),
            "{line}"
        );
    }
}

fn round_trip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let wire = serde_json::to_string(value).unwrap();
    serde_json::from_str(&wire).unwrap_or_else(|e| panic!("{e}: {wire}"))
}

#[test]
fn commands_and_replies_cross_the_wire_unchanged() {
    let lines = [
        "open",
        "get @led1",
        "step 100ms",
        "expect @led1 off",
        "press @btn1",
        "step --until @led1 on --timeout 50ms",
        "get @nano.D13",
        "get @led1.i",
        "expect @led1.i > 3.2687677014840023e-7A",
        "expect serial contains \"ready\"",
        "snapshot",
        "release @btn1",
        "step 10ms",
        "get @nano",
        "expect no diagnostics",
        "close",
        "open missing.jsonc",
    ];
    let mut live = Live::new("t");
    let mut codes = Vec::new();
    for line in lines {
        let command = command(line);
        assert_eq!(round_trip(&command), command, "{line}");
        let reply: Reply = live.run(&command);
        assert_eq!(round_trip(&reply), reply, "{line}");
        codes.extend(reply.error.map(|e| e.code));
    }
    assert_eq!(codes, [Code::ExpectFailed, Code::NoReading, Code::IoError]);

    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("scene.jsonc");
    std::fs::copy(first_target().join("scene.jsonc"), &scene).unwrap();
    let refused = Live::new("t").run(&Command::Open(scene));
    assert!(
        matches!(&refused.data, Data::Invalid(found) if found.len() > 1),
        "{refused:?}"
    );
    assert_eq!(round_trip(&refused), refused);
}
