//! The live commands between separate shell commands, each run against the real session
//! process. Every test gets its own runtime folder, and kills the processes it started.

#![cfg(target_os = "linux")]

#[path = "session/harness.rs"]
mod harness;
mod live;
#[path = "registry_env.rs"]
mod registry_env;

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

use harness::{Run, Runtime, gone, kill, kill_daemon, wait_until};
use live::{common, project};

/// The Nano button Block laid out as in the repo, so `blocks/nano-button-led/scene.jsonc` names
/// the same files from the folder's root, with `lines` as `tests/live.test`.
fn first_target(lines: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let block = dir.path().join("blocks/nano-button-led");
    common::copy_dir(&common::repo().join("blocks/nano-button-led"), &block);
    std::os::unix::fs::symlink(common::repo().join("parts"), dir.path().join("parts")).unwrap();
    fs::write(block.join("tests/live.test"), lines).unwrap();
    dir
}

const FIRST_TARGET: [&str; 8] = [
    "open blocks/nano-button-led/scene.jsonc",
    "step 100ms",
    "expect @led1 off",
    "press @btn1",
    "step --until @led1 lit --timeout 50ms",
    "expect @nano.D13 high",
    "get @nano.D13",
    "close",
];

/// Each reply `orbe test -v` prints, de-indented, as the live command prints it.
fn verbose_replies(stdout: &str) -> Vec<String> {
    let mut replies: Vec<String> = Vec::new();
    for line in stdout.lines() {
        if line.starts_with("blocks/nano-button-led/tests/live.test:") {
            replies.push(String::new());
        } else if let (Some(reply), Some(text)) = (replies.last_mut(), line.strip_prefix("  ")) {
            reply.push_str(text);
            reply.push('\n');
        }
    }
    replies
}

#[test]
fn the_first_target_runs_live_one_process_per_command_as_orbe_test_runs_it() {
    let lines: String = FIRST_TARGET
        .iter()
        .map(|l| format!("{}\n", l.replace("blocks/nano-button-led/", "")))
        .collect();
    let dir = first_target(&lines);
    let root = dir.path();
    let runtime = Runtime::new();
    let tested = runtime
        .orbe(
            root,
            &["test", "-v", "blocks/nano-button-led/tests/live.test"],
        )
        .ok();
    let expected = verbose_replies(&tested.stdout);
    assert_eq!(expected.len(), FIRST_TARGET.len(), "{}", tested.stdout);

    let mut transcript = String::new();
    for (line, expected) in FIRST_TARGET.iter().zip(&expected) {
        let args: Vec<&str> = line.split(' ').collect();
        let run = runtime.orbe(root, &args).ok();
        assert_eq!(&run.stdout, expected, "`orbe {line}`");
        transcript.push_str(&format!("$ orbe {line}\n{}", run.stdout));
    }
    assert!(
        transcript.contains("$ orbe get @nano.D13\nnano.D13 high"),
        "{transcript}"
    );
    wait_until("close ends the process", || runtime.daemons().is_empty());
    assert!(runtime.sockets().is_empty(), "close removes the socket");

    let tested = runtime
        .orbe(
            root,
            &[
                "test",
                "--json",
                "-v",
                "blocks/nano-button-led/tests/live.test",
            ],
        )
        .ok()
        .json();
    let replies = tested["data"]["tests"][0]["replies"].as_array().unwrap();
    for (line, entry) in FIRST_TARGET.iter().zip(replies) {
        let mut args: Vec<&str> = line.split(' ').collect();
        args.push("--json");
        let mut envelope = runtime.orbe(root, &args).ok().json();
        let mut expected = entry["reply"].clone();
        assert!(
            envelope["session"].as_str().unwrap().starts_with("tmp"),
            "{envelope}"
        );
        envelope["session"] = Value::Null;
        expected["session"] = Value::Null;
        assert_eq!(envelope, expected, "`orbe {line} --json`");
    }
}

#[test]
fn a_live_read_gives_the_engine_value_pressed_at_50ms_and_read_at_100ms() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    for line in ["open", "step 50ms", "press @btn1", "step 50ms"] {
        let args: Vec<&str> = line.split(' ').collect();
        runtime.orbe(dir.path(), &args).ok();
    }
    let run = runtime.orbe(dir.path(), &["get", "@nano.D13"]).ok();
    assert_eq!(run.stdout, "nano.D13 high 4.400713547712648 V\n");
    runtime.orbe(dir.path(), &["close"]).ok();
}

#[test]
fn a_command_with_no_session_says_which_open_to_run() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let run = runtime.orbe(dir.path(), &["get", "@led1", "--session", "bench"]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "error[no_session]: no session `bench` is running\n  hint: run `orbe open --session bench` to start it\n"
    );

    let tests = dir.path().join("tests");
    let run = runtime.orbe(&tests, &["step", "1ms", "--json"]);
    assert_eq!(run.code, 1);
    let envelope = run.json();
    assert_eq!(envelope["error"]["code"], "no_session");
    assert_eq!(
        envelope["error"]["hint"],
        "run `orbe open ../scene.jsonc` to start it"
    );
    assert_eq!(envelope["state"], "closed");
    assert!(runtime.daemons().is_empty(), "only `open` starts a process");
    assert!(runtime.sockets().is_empty());
}

#[test]
fn open_replaces_the_socket_a_killed_process_left() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "stale"];
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    runtime
        .orbe(dir.path(), &[&["step", "20ms"][..], &session].concat())
        .ok();
    kill_daemon(&runtime);
    assert_eq!(
        runtime.sockets(),
        ["stale.sock"],
        "kill -9 leaves the socket"
    );

    let run = runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    assert!(
        run.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
    let run = runtime
        .orbe(dir.path(), &[&["step", "10ms"][..], &session].concat())
        .ok();
    assert_eq!(
        run.stdout,
        "\
t=10ms (from 0s)
  brightness led1 off 3.4882796973335236e-10 A over 10ms
  brightness nano.led_l off 3.4809606749501575e-10 A over 10ms
  brightness nano.led_pwr lit 3.0136985456123377 mA over 10ms
"
    );
    runtime
        .orbe(dir.path(), &[&["close"][..], &session].concat())
        .ok();
    assert!(runtime.sockets().is_empty());
}

#[test]
fn a_process_that_dies_is_session_crashed_until_it_is_opened_again() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime
        .orbe(dir.path(), &["open", "--session", "crash"])
        .ok();
    let stepping = runtime.spawn(dir.path(), &["step", "600s", "--session", "crash"]);
    std::thread::sleep(Duration::from_millis(500));
    kill_daemon(&runtime);
    let run = Run::new(stepping.wait_with_output().unwrap());
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        "error[session_crashed]: the process of session `crash` ended before it answered\n  hint: run `orbe open --session crash` to start it again\n"
    );

    let run = runtime.orbe(dir.path(), &["snapshot", "--session", "crash", "--json"]);
    assert_eq!(run.code, 1);
    let envelope = run.json();
    assert_eq!(envelope["error"]["code"], "session_crashed");
    assert_eq!(
        envelope["error"]["message"],
        "the process of session `crash` ended without closing"
    );

    runtime
        .orbe(dir.path(), &["open", "--session", "crash"])
        .ok();
    runtime
        .orbe(dir.path(), &["close", "--session", "crash"])
        .ok();
}

/// The path a crash message names, `<name>.log.crashed`.
fn crashed_log(text: &str) -> &str {
    text.lines()
        .find_map(|line| {
            line.rsplit_once("its last output is in ")
                .map(|(_, path)| path.trim())
        })
        .unwrap_or_else(|| panic!("no crash log in:\n{text}"))
}

#[test]
fn a_crash_log_stays_after_it_is_read_and_the_next_open_names_it() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "kept"];
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    kill_daemon(&runtime);
    fs::write(runtime.sessions().join("kept.log"), "panic: boom\n").unwrap();

    let run = runtime.orbe(dir.path(), &["get", "@led1", "--session", "kept"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let path = crashed_log(&run.stderr);
    assert!(path.ends_with("kept.log.crashed"), "{path}");
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: boom\n");

    let run = runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    assert!(
        run.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(&format!(
            "the previous session crashed; its last output is in {path}"
        )),
        "{}",
        run.stdout
    );
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "panic: boom\n",
        "opening again must not empty the crash log"
    );
    let session_log = fs::read_to_string(runtime.sessions().join("kept.log")).unwrap_or_default();
    assert!(
        !session_log.contains("panic: boom"),
        "the new session's log is not the crash: {session_log}"
    );
    runtime
        .orbe(dir.path(), &[&["close"][..], &session].concat())
        .ok();
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: boom\n");
}

#[test]
fn close_keeps_the_latest_crash_log_and_close_all_does_too() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let crash_twice = |name: &str| {
        let session = ["--session", name];
        runtime
            .orbe(dir.path(), &[&["open"][..], &session].concat())
            .ok();
        kill_daemon(&runtime);
        fs::write(
            runtime.sessions().join(format!("{name}.log")),
            "crash one\n",
        )
        .unwrap();
        runtime
            .orbe(dir.path(), &[&["open"][..], &session].concat())
            .ok();
        let pid = runtime.daemon();
        let stderr = fs::read_link(format!("/proc/{pid}/fd/2")).unwrap();
        fs::write(&stderr, "crash two\n").unwrap();
        kill_daemon(&runtime);
    };

    crash_twice("c2");
    let run = runtime.orbe(dir.path(), &["close", "--session", "c2"]).ok();
    let path = crashed_log(&run.stdout);
    assert!(path.ends_with("c2.log.crashed"), "{path}\n{}", run.stdout);
    assert_eq!(fs::read_to_string(path).unwrap(), "crash two\n");

    crash_twice("c3");
    let run = runtime.orbe(dir.path(), &["close", "--all"]).ok();
    let path = crashed_log(&run.stdout);
    assert!(path.ends_with("c3.log.crashed"), "{path}\n{}", run.stdout);
    assert_eq!(fs::read_to_string(path).unwrap(), "crash two\n");
}

#[test]
fn a_refused_open_after_a_crash_still_names_the_crash_log() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "bad"];
    let scene = dir.path().join("scene.jsonc");
    let good = fs::read_to_string(&scene).unwrap();
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    kill_daemon(&runtime);
    fs::write(runtime.sessions().join("bad.log"), "panic: boom\n").unwrap();
    fs::write(&scene, "{\n").unwrap();

    let run = runtime.orbe(dir.path(), &[&["open"][..], &session].concat());
    assert_ne!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("error["), "{}", run.stdout);
    assert!(
        run.stdout.contains("the previous session crashed"),
        "{}",
        run.stdout
    );
    let path = crashed_log(&run.stdout);
    assert!(path.ends_with("bad.log.crashed"), "{path}");
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: boom\n");

    fs::write(&scene, good).unwrap();
    let run = runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    assert!(
        run.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: boom\n");
    runtime
        .orbe(dir.path(), &[&["close"][..], &session].concat())
        .ok();
}

/// A crash that writes nothing replaces the previous crash's log, so `step` and `close` do
/// not name that older panic.
#[test]
fn a_silent_crash_does_not_name_an_older_crash_log() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "s1"];
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    kill_daemon(&runtime);
    fs::write(runtime.sessions().join("s1.log"), "panic: crash one\n").unwrap();
    let opened = runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    let path = crashed_log(&opened.stdout);
    assert!(path.ends_with("s1.log.crashed"), "{path}");
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: crash one\n");
    kill_daemon(&runtime);

    let step = runtime.orbe(dir.path(), &["step", "1ms", "--session", "s1"]);
    assert_eq!(
        (step.code, step.stdout.as_str(), step.stderr.as_str()),
        (
            1,
            "",
            "error[session_crashed]: the process of session `s1` ended without closing\n  hint: run `orbe open --session s1` to start it again\n"
        )
    );
    assert!(
        !Path::new(path).exists(),
        "{path} still holds the older crash"
    );
    let close = runtime.orbe(dir.path(), &["close", "--session", "s1"]).ok();
    assert_eq!(
        close.stdout,
        "removed the socket of a process that ended without closing\n"
    );
    assert!(!close.stdout.contains("crash one"), "{}", close.stdout);
    assert!(!Path::new(path).exists(), "{path}");
}

/// An `open` whose process dies on every attempt names the crash log that holds the panic.
#[test]
fn an_open_whose_every_process_dies_names_the_crash_log() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let log = runtime.sessions().join("s2.log");
    let child = runtime
        .command(dir.path(), &["open", "--session", "s2"])
        .env("ORBE_TEST_READ_DELAY", "3s")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut kills = 0;
    let mut spins = 0;
    while kills < 5 && spins < 2_000 {
        spins += 1;
        if gone(child.id()) {
            break;
        }
        let Some(&pid) = runtime.daemons().first() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        if accepted_sockets(pid) < 2 {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        fs::write(&log, "panic: dies\n").unwrap();
        kill("-KILL", pid);
        wait_until("the killed session process is dead", || gone(pid));
        kills += 1;
    }
    let run = Run::new(child.wait_with_output().unwrap());
    assert_eq!(kills, 5, "{}{}", run.stdout, run.stderr);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(!run.stderr.contains("previous session"), "{}", run.stderr);
    let path = crashed_log(&run.stderr);
    assert!(path.ends_with("s2.log.crashed"), "{path}\n{}", run.stderr);
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: dies\n");
    assert!(
        run.stderr.contains(&format!(
            "error[session_crashed]: the process of session `s2` ended before it answered `open`, 5 times\n  hint: its last output is in {path}\n"
        )),
        "{}",
        run.stderr
    );
}

/// An earlier crash's output stays the file the note names when every retry of `open` also
/// writes. The retries are not a previous session.
#[test]
fn an_open_after_a_crash_names_that_crash_not_a_retry() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["open", "--session", "s2"]).ok();
    kill_daemon(&runtime);
    fs::write(runtime.sessions().join("s2.log"), "panic: earlier\n").unwrap();

    let log = runtime.sessions().join("s2.log");
    let child = runtime
        .command(dir.path(), &["open", "--session", "s2"])
        .env("ORBE_TEST_READ_DELAY", "3s")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut kills = 0;
    let mut spins = 0;
    while kills < 5 && spins < 2_000 {
        spins += 1;
        if gone(child.id()) {
            break;
        }
        let Some(&pid) = runtime.daemons().first() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        if accepted_sockets(pid) < 2 {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        fs::write(&log, "panic: retry\n").unwrap();
        kill("-KILL", pid);
        wait_until("the killed session process is dead", || gone(pid));
        kills += 1;
    }
    let run = Run::new(child.wait_with_output().unwrap());
    assert_eq!(kills, 5, "{}{}", run.stdout, run.stderr);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    let note = run
        .stderr
        .lines()
        .find(|line| line.contains("the previous session crashed"))
        .unwrap_or_else(|| panic!("no previous-session note in:\n{}", run.stderr));
    let path = note
        .rsplit_once("its last output is in ")
        .map(|(_, path)| path.trim())
        .unwrap_or_else(|| panic!("the note names no file:\n{note}"));
    assert!(path.ends_with("s2.log.crashed"), "{path}\n{}", run.stderr);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "panic: earlier\n",
        "{}",
        run.stderr
    );
    let hint = crashed_log(&run.stderr);
    assert_eq!(fs::read_to_string(hint).unwrap(), "panic: retry\n");
    assert_ne!(hint, path);
}

/// A socket nothing answers on, put in place of a live session's, is not that process's log.
/// `step` and `close` name the earlier crash and leave the live log in place.
#[test]
fn a_live_log_stays_when_a_dead_socket_stands_in_for_it() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "lk"];
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    kill_daemon(&runtime);
    fs::write(runtime.sessions().join("lk.log"), "panic: earlier\n").unwrap();
    let step = runtime.orbe(dir.path(), &["step", "1ms", "--session", "lk"]);
    assert_eq!(step.code, 1, "{}", step.stderr);
    let crash = crashed_log(&step.stderr);
    assert_eq!(fs::read_to_string(crash).unwrap(), "panic: earlier\n");

    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    let pid = runtime.daemon();
    let log = runtime.sessions().join("lk.log");
    assert_eq!(
        fs::read_link(format!("/proc/{pid}/fd/2")).unwrap(),
        log,
        "the live process writes the session log"
    );
    let sock = runtime.sessions().join("lk.sock");
    fs::remove_file(&sock).unwrap();
    drop(std::os::unix::net::UnixListener::bind(&sock).unwrap());

    let step = runtime.orbe(dir.path(), &["step", "1ms", "--session", "lk"]);
    assert_eq!(step.code, 1, "{}{}", step.stdout, step.stderr);
    assert_eq!(
        step.stderr.matches("session_crashed").count(),
        1,
        "{}",
        step.stderr
    );
    let named = crashed_log(&step.stderr);
    assert_eq!(named, crash);
    assert_eq!(fs::read_to_string(named).unwrap(), "panic: earlier\n");
    assert!(log.exists(), "the live log was removed");
    fs::write(format!("/proc/{pid}/fd/2"), "still live\n").unwrap();
    assert!(
        fs::read_to_string(&log).unwrap().contains("still live\n"),
        "the process no longer writes {}",
        log.display()
    );
    assert!(!gone(pid));

    let close = runtime.orbe(dir.path(), &["close", "--session", "lk"]).ok();
    let named = crashed_log(&close.stdout);
    assert_eq!(fs::read_to_string(named).unwrap(), "panic: earlier\n");
    assert!(log.exists(), "close removed the live log");
    assert!(
        fs::read_to_string(&log).unwrap().contains("still live\n"),
        "{}",
        log.display()
    );
    assert!(!gone(pid));
}

/// Session `foo.crashed` has its own log. It is not session `foo`'s crash log, and `close
/// --all` leaves that crash log in place.
#[test]
fn a_dotted_session_name_does_not_share_a_crash_log() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["open", "--session", "foo"]).ok();
    kill_daemon(&runtime);
    fs::write(runtime.sessions().join("foo.log"), "panic: from foo\n").unwrap();
    let step = runtime.orbe(dir.path(), &["step", "1ms", "--session", "foo"]);
    assert_eq!(step.code, 1, "{}", step.stderr);
    let path = crashed_log(&step.stderr);
    assert!(path.ends_with("foo.log.crashed"), "{path}\n{}", step.stderr);
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: from foo\n");

    runtime
        .orbe(dir.path(), &["open", "--session", "foo.crashed"])
        .ok();
    let live = runtime.sessions().join("foo.crashed.log");
    assert_ne!(Path::new(path), live.as_path());
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: from foo\n");
    let live_text = fs::read_to_string(&live).unwrap_or_default();
    assert!(
        !live_text.contains("panic: from foo"),
        "the other session's log is the crash: {live_text}"
    );
    runtime.orbe(dir.path(), &["close", "--all"]).ok();
    assert_eq!(fs::read_to_string(path).unwrap(), "panic: from foo\n");
    assert!(
        session_files(&runtime).contains(&"foo.log.crashed".to_string()),
        "{:?}",
        session_files(&runtime)
    );
}

/// A refusal the client makes after a crash carries `previous_crash`, as a refusal the
/// session process makes does, and both print that note on its own line.
#[test]
fn a_client_refused_open_after_a_crash_uses_previous_crash() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "s5"];
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    kill_daemon(&runtime);
    let log = runtime.sessions().join("s5.log");
    fs::remove_file(&log).unwrap();
    fs::create_dir(&log).unwrap();
    let crash = runtime.sessions().join("s5.log.crashed");
    fs::write(&crash, "panic: boom\n").unwrap();

    let text = runtime.orbe(dir.path(), &[&["open"][..], &session].concat());
    assert_eq!(text.code, 3, "{}{}", text.stdout, text.stderr);
    let note = format!(
        "the previous session crashed; its last output is in {}",
        crash.display()
    );
    assert!(
        text.stderr.ends_with(&format!("{note}\n")),
        "{}",
        text.stderr
    );
    assert!(
        text.stderr
            .lines()
            .any(|line| line == format!("  hint: remove it with `rm -r {}`", log.display())),
        "{}",
        text.stderr
    );
    assert!(
        !text
            .stderr
            .lines()
            .any(|line| { line.starts_with("  hint:") && line.contains("previous session") })
    );
    assert_eq!(fs::read_to_string(&crash).unwrap(), "panic: boom\n");

    let json = runtime.orbe(dir.path(), &[&["open", "--json"][..], &session].concat());
    assert_eq!(json.code, 3, "{}", json.stdout);
    let envelope = json.json();
    assert_eq!(envelope["data"]["previous_crash"], note);
    assert!(
        !envelope["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("previous session"),
        "{}",
        json.stdout
    );
    assert_eq!(envelope["error"]["code"], "io_error");

    fs::remove_dir(&log).unwrap();
    let scene = dir.path().join("scene.jsonc");
    let good = fs::read_to_string(&scene).unwrap();
    for (name, json) in [("s6", false), ("s7", true)] {
        runtime.orbe(dir.path(), &["open", "--session", name]).ok();
        kill_daemon(&runtime);
        fs::write(
            runtime.sessions().join(format!("{name}.log")),
            "panic: boom\n",
        )
        .unwrap();
        fs::write(&scene, "{\n").unwrap();
        let args: &[&str] = if json {
            &["open", "--json", "--session", name]
        } else {
            &["open", "--session", name]
        };
        let refused = runtime.orbe(dir.path(), args);
        fs::write(&scene, &good).unwrap();
        assert_ne!(
            refused.code, 0,
            "{name}: {}{}",
            refused.stdout, refused.stderr
        );
        if json {
            let process = refused.json();
            let process_note = process["data"]["previous_crash"].as_str().unwrap();
            assert!(
                process_note.starts_with("the previous session crashed; its last output is in "),
                "{process_note}"
            );
            assert!(
                process_note.ends_with(&format!("{name}.log.crashed")),
                "{process_note}"
            );
            assert!(
                !process["error"]["hint"]
                    .as_str()
                    .unwrap()
                    .contains("previous session"),
                "{}",
                refused.stdout
            );
        } else {
            let process_path = crashed_log(&refused.stdout);
            let process_note =
                format!("the previous session crashed; its last output is in {process_path}");
            assert!(
                refused.stdout.ends_with(&format!("{process_note}\n")),
                "{}",
                refused.stdout
            );
            assert!(
                process_path.ends_with(&format!("{name}.log.crashed")),
                "{process_path}"
            );
            assert!(
                !refused.stdout.lines().any(|line| {
                    line.starts_with("  hint:") && line.contains("previous session")
                }),
                "{}",
                refused.stdout
            );
        }
    }
}

/// Sockets the process holds. The listener is one; an accepted connection is the second.
fn accepted_sockets(pid: u32) -> usize {
    fs::read_dir(format!("/proc/{pid}/fd"))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    fs::read_link(entry.path())
                        .ok()
                        .is_some_and(|target| target.to_string_lossy().starts_with("socket:"))
                })
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn two_sessions_with_different_names_run_side_by_side() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let by_env = |args: &[&str]| {
        Run::new(
            runtime
                .command(dir.path(), args)
                .env("ORBE_SESSION", "b")
                .output()
                .unwrap(),
        )
    };
    runtime.orbe(dir.path(), &["open", "--session", "a"]).ok();
    by_env(&["open"]).ok();
    assert_eq!(runtime.daemons().len(), 2);
    runtime
        .orbe(dir.path(), &["step", "10ms", "--session", "a"])
        .ok();
    by_env(&["step", "20ms"]).ok();
    by_env(&["press", "@btn1"]).ok();
    by_env(&["step", "1ms"]).ok();

    let a = runtime
        .orbe(dir.path(), &["snapshot", "--json", "--session=a"])
        .ok()
        .json();
    let b = by_env(&["snapshot", "--json"]).ok().json();
    assert_eq!(
        (&a["session"], &a["t_ps"]),
        (&"a".into(), &10_000_000_000u64.into())
    );
    assert_eq!(
        (&b["session"], &b["t_ps"]),
        (&"b".into(), &21_000_000_000u64.into())
    );
    let pressed = |s: &Value| s["data"]["parts"][1]["value"]["i"].as_f64().unwrap().abs() > 1e-6;
    assert!(!pressed(&a) && pressed(&b), "{a}\n{b}");

    runtime.orbe(dir.path(), &["close", "--session", "a"]).ok();
    by_env(&["close"]).ok();
    wait_until("both processes end", || runtime.daemons().is_empty());
}

#[test]
fn ctrl_c_cancels_a_long_step_and_the_session_stays_where_it_stopped() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime
        .orbe(dir.path(), &["open", "--session", "sigint"])
        .ok();
    let stepping = runtime.spawn(
        dir.path(),
        &["step", "600s", "--json", "--session", "sigint"],
    );
    std::thread::sleep(Duration::from_millis(500));
    kill("-INT", stepping.id());
    let run = Run::new(stepping.wait_with_output().unwrap());
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    let envelope = run.json();
    assert_eq!(envelope["error"]["code"], "cancelled", "{envelope}");
    assert_eq!(envelope["state"], "paused");
    let reached = envelope["t_ps"].as_u64().unwrap();
    assert!(reached > 0 && reached < 600_000_000_000_000, "{envelope}");

    let snapshot = runtime
        .orbe(dir.path(), &["snapshot", "--json", "--session", "sigint"])
        .ok()
        .json();
    assert_eq!(snapshot["t_ps"].as_u64(), Some(reached));
    let stepped = runtime
        .orbe(
            dir.path(),
            &["step", "1ms", "--json", "--session", "sigint"],
        )
        .ok()
        .json();
    assert_eq!(stepped["data"]["from_ps"].as_u64(), Some(reached));
    assert_eq!(stepped["t_ps"].as_u64(), Some(reached + 1_000_000_000));
    runtime
        .orbe(dir.path(), &["close", "--session", "sigint"])
        .ok();
}

#[test]
fn an_idle_session_ends_after_its_timeout() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let run = runtime
        .command(dir.path(), &["open", "--session", "idle"])
        .env("ORBE_TEST_IDLE_TIMEOUT", "300ms")
        .output()
        .unwrap();
    assert_eq!(Run::new(run).code, 0);
    wait_until("the idle process ends", || runtime.daemons().is_empty());
    assert!(
        runtime.sockets().is_empty(),
        "an idle exit removes the socket"
    );
    let run = runtime.orbe(dir.path(), &["get", "@led1", "--session", "idle", "--json"]);
    assert_eq!(run.json()["error"]["code"], "no_session");
}

#[test]
fn a_playing_session_outlives_its_idle_timeout() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let run = runtime
        .command(dir.path(), &["open", "--session", "idle"])
        .env("ORBE_TEST_IDLE_TIMEOUT", "1s")
        .output()
        .unwrap();
    assert_eq!(Run::new(run).code, 0);
    runtime
        .orbe(dir.path(), &["play", "--session", "idle"])
        .ok();
    std::thread::sleep(Duration::from_millis(3000));
    let paused = runtime
        .orbe(dir.path(), &["pause", "--session", "idle", "--json"])
        .ok()
        .json();
    assert!(
        paused["state"] == "paused" && paused["t_ps"].as_u64().unwrap() > 0,
        "play ran past the idle timeout: {paused}"
    );
    wait_until("the paused process ends", || runtime.daemons().is_empty());
}

#[test]
fn a_session_folder_that_is_a_symlink_is_refused_wherever_it_points() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let temp = runtime.dir.path();
    let uid = fs::metadata(temp).unwrap().uid();
    let folder = temp.join(format!("orbe-{uid}"));
    let private = temp.join("private");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    for target in [Path::new("/"), &private] {
        let _ = fs::remove_file(&folder);
        std::os::unix::fs::symlink(target, &folder).unwrap();
        let run = Run::new(
            runtime
                .command(dir.path(), &["open"])
                .env_remove("XDG_RUNTIME_DIR")
                .env("TMPDIR", temp)
                .output()
                .unwrap(),
        );
        assert_eq!(run.code, 3, "{}", run.stderr);
        assert_eq!(
            run.stderr,
            format!(
                "error[io_error]: the session folder {shown} is a symlink, not a folder\n  hint: remove it with `rm {shown}`, or set XDG_RUNTIME_DIR to a folder you own\n",
                shown = folder.display()
            )
        );
    }
    assert_eq!(
        fs::read_dir(&private).unwrap().count(),
        0,
        "nothing is made in the target"
    );
    assert!(runtime.daemons().is_empty());
}

#[test]
fn a_session_socket_that_is_a_symlink_is_refused_and_the_session_it_names_is_untouched() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime
        .orbe(dir.path(), &["open", "--session", "real"])
        .ok();
    runtime
        .orbe(dir.path(), &["step", "20ms", "--session", "real"])
        .ok();
    let plant = runtime.sessions().join("plant.sock");
    std::os::unix::fs::symlink(runtime.sessions().join("real.sock"), &plant).unwrap();
    let refused = format!(
        "error[io_error]: {shown} is a symlink, not the socket of session `plant`\n  hint: remove it with `rm {shown}`\n",
        shown = plant.display()
    );
    for line in [&["open"][..], &["get", "@led1"], &["close"]] {
        let run = runtime.orbe(dir.path(), &[line, &["--session", "plant"]].concat());
        assert_eq!(
            (run.code, run.stderr.as_str()),
            (3, refused.as_str()),
            "{line:?}"
        );
    }
    let snapshot = runtime
        .orbe(dir.path(), &["snapshot", "--json", "--session", "real"])
        .ok()
        .json();
    assert_eq!(snapshot["t_ps"], 20_000_000_000u64);
    assert_eq!(runtime.daemons().len(), 1, "no process starts for `plant`");
}

#[test]
fn open_does_not_write_through_a_symlinked_log() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["close", "--all"]).ok();
    let secret = dir.path().join("secret.txt");
    fs::write(&secret, "keep me\n").unwrap();
    let log = runtime.sessions().join("plant.log");
    std::os::unix::fs::symlink(&secret, &log).unwrap();
    let run = runtime.orbe(dir.path(), &["open", "--session", "plant"]);
    assert_eq!(run.code, 3, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stderr,
        format!(
            "error[io_error]: {shown} is a symlink, not the log of session `plant`\n  hint: remove it with `rm {shown}`\n",
            shown = log.display()
        )
    );
    assert_eq!(fs::read_to_string(&secret).unwrap(), "keep me\n");
    assert!(runtime.daemons().is_empty());
}

#[test]
fn commands_queued_behind_close_get_no_session_and_a_queued_open_starts_again() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "q"];
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    let stepping = runtime.spawn(dir.path(), &[&["step", "600s"][..], &session].concat());
    std::thread::sleep(Duration::from_millis(300));
    let mut queued = Vec::new();
    for line in [&["close"][..], &["get", "@led1"], &["open"]] {
        queued.push(runtime.spawn(dir.path(), &[line, &session].concat()));
        std::thread::sleep(Duration::from_millis(150));
    }
    kill("-INT", stepping.id());
    Run::new(stepping.wait_with_output().unwrap());
    let mut runs = queued
        .into_iter()
        .map(|child| Run::new(child.wait_with_output().unwrap()));
    let (close, get, open) = (
        runs.next().unwrap(),
        runs.next().unwrap(),
        runs.next().unwrap(),
    );
    assert!(close.stdout.starts_with("closed at t="), "{}", close.stdout);
    assert_eq!((get.code, get.stdout.as_str()), (1, ""));
    assert_eq!(
        get.stderr,
        "error[no_session]: no session `q` is running\n  hint: run `orbe open --session q` to start it\n"
    );
    let open = open.ok();
    assert!(
        open.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        open.stdout
    );
    let snapshot = runtime
        .orbe(
            dir.path(),
            &[&["snapshot", "--json"][..], &session].concat(),
        )
        .ok()
        .json();
    assert_eq!(
        (&snapshot["state"], &snapshot["t_ps"]),
        (&"paused".into(), &0.into())
    );
    assert_eq!(runtime.daemons().len(), 1, "the open started a new process");
    runtime
        .orbe(dir.path(), &[&["close"][..], &session].concat())
        .ok();
}

#[test]
fn a_read_before_the_first_open_gets_the_shell_no_session() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let mut starting = runtime.listening(dir.path(), "st", None);
    let run = runtime.orbe(dir.path(), &["get", "@led1", "--session", "st"]);
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert_eq!(
        run.stderr,
        "error[no_session]: no session `st` is running\n  hint: run `orbe open --session st` to start it\n"
    );
    starting.kill().unwrap();
    starting.wait().unwrap();
}

#[test]
fn a_queued_command_whose_client_was_killed_does_not_run() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "k"];
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    let stepping = runtime.spawn(
        dir.path(),
        &[&["step", "600s", "--json"][..], &session].concat(),
    );
    std::thread::sleep(Duration::from_millis(300));
    let mut dead = [
        runtime.spawn(dir.path(), &[&["step", "5s"][..], &session].concat()),
        runtime.spawn(dir.path(), &[&["press", "@btn1"][..], &session].concat()),
    ];
    std::thread::sleep(Duration::from_millis(300));
    for child in &mut dead {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    std::thread::sleep(Duration::from_millis(100));
    kill("-INT", stepping.id());
    let run = Run::new(stepping.wait_with_output().unwrap());
    let reached = run.json()["t_ps"].as_u64().unwrap();

    let snapshot = runtime
        .orbe(
            dir.path(),
            &[&["snapshot", "--json"][..], &session].concat(),
        )
        .ok()
        .json();
    assert_eq!(
        snapshot["t_ps"].as_u64(),
        Some(reached),
        "the dead step moved no time"
    );
    runtime
        .orbe(dir.path(), &[&["press", "@btn1"][..], &session].concat())
        .ok();
    runtime
        .orbe(dir.path(), &[&["close"][..], &session].concat())
        .ok();
}

#[test]
fn a_working_folder_that_is_not_utf8_is_refused_before_anything_starts() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let odd = dir.path().join(OsStr::from_bytes(b"odd\xff"));
    fs::create_dir(&odd).unwrap();
    fs::copy(dir.path().join("scene.jsonc"), odd.join("scene.jsonc")).unwrap();
    let message = format!(
        "the working folder {} is not UTF-8, and the session process needs a UTF-8 path",
        odd.display()
    );
    let run = runtime.orbe(&odd, &["open", "--session", "u"]);
    assert_eq!(
        (run.code, run.stderr),
        (
            3,
            format!(
                "error[io_error]: {message}\n  hint: run `orbe` from a folder whose path is UTF-8\n"
            )
        )
    );
    let run = runtime.orbe(&odd, &["get", "@led1", "--session", "u", "--json"]);
    assert_eq!(run.code, 3);
    let envelope = run.json();
    assert_eq!(envelope["error"]["message"], message.as_str());
    assert!(runtime.daemons().is_empty(), "nothing started");
    assert!(runtime.sockets().is_empty());
}

#[test]
fn snapshot_shows_the_scene_from_the_folder_it_runs_in() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let root = dir.path();
    fs::create_dir_all(root.join("a/b")).unwrap();
    let run = runtime
        .orbe(
            &root.join("tests"),
            &["open", "../scene.jsonc", "--session", "v"],
        )
        .ok();
    assert!(
        run.stdout.starts_with("opened ../scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
    for (cwd, shown) in [
        (root.to_path_buf(), "scene.jsonc"),
        (root.join("tests"), "../scene.jsonc"),
        (root.join("a/b"), "../../scene.jsonc"),
    ] {
        let run = runtime.orbe(&cwd, &["snapshot", "--session", "v"]).ok();
        let first = run.stdout.lines().next().unwrap().to_string();
        assert_eq!(
            first,
            format!("t=0s  scene={shown}"),
            "from {}",
            cwd.display()
        );
        let json = runtime
            .orbe(&cwd, &["snapshot", "--json", "--session", "v"])
            .ok()
            .json();
        assert_eq!(json["data"]["scene"], shown);
    }
    runtime.orbe(root, &["close", "--session", "v"]).ok();
}

#[test]
fn a_refusal_the_client_makes_shows_the_state_the_process_reports() {
    let lines = "open\nstep 5ms\nget\n";
    let dir = project(lines);
    let runtime = Runtime::new();
    let tested = live::replies_in(&dir);
    for line in lines.lines().take(2) {
        let args: Vec<&str> = line.split(' ').collect();
        runtime
            .orbe(dir.path(), &[&args[..], &["--session", "n"]].concat())
            .ok();
    }
    let run = runtime.orbe(dir.path(), &["get", "--session", "n", "--json"]);
    assert_eq!(run.code, 2);
    let mut envelope = run.json();
    assert_eq!(envelope["state"], "paused", "{envelope}");
    let mut expected = tested[2].clone();
    envelope["session"] = Value::Null;
    expected["session"] = Value::Null;
    assert_eq!(envelope, expected);

    let run = runtime.orbe(dir.path(), &["get", "--session", "n"]);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (
            2,
            "",
            "error[usage]: takes one ref, such as `@btn1`, or a world quantity, such as `light`; usage: get <ref> | get <quantity>\n"
        )
    );

    let unknown = runtime
        .orbe(dir.path(), &["get", "@led1", "--session", "-x", "--json"])
        .json();
    assert_eq!(unknown["error"]["code"], "usage");
    assert!(
        unknown.get("state").is_none() && unknown.get("t_ps").is_none(),
        "no process told it the state: {unknown}"
    );
    runtime.orbe(dir.path(), &["close", "--session", "n"]).ok();
}

#[test]
fn a_session_another_version_started_names_that_version_and_asks_for_close() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "up"];
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    let version = runtime.sessions().join("up.version");
    assert_eq!(
        fs::read_to_string(&version).unwrap(),
        format!("{}\n", env!("CARGO_PKG_VERSION"))
    );
    fs::write(&version, "9.9.9\n").unwrap();
    let running = runtime.daemons();
    assert_eq!(running.len(), 1);

    let run = runtime.orbe(dir.path(), &["get", "@led1", "--session", "up"]);
    assert_eq!(
        run.stderr,
        "error[version_mismatch]: session `up` runs orbe 9.9.9\n  hint: run `orbe close --session up`\n"
    );
    assert_eq!(runtime.daemons(), running, "no second session starts");

    let opened = runtime.orbe(dir.path(), &[&["open"][..], &session].concat());
    assert_eq!(opened.code, 1, "{}{}", opened.stdout, opened.stderr);
    assert!(
        opened.stderr.contains("runs orbe 9.9.9"),
        "{}",
        opened.stderr
    );
    assert_eq!(runtime.daemons(), running);

    runtime
        .orbe(dir.path(), &[&["close"][..], &session].concat())
        .ok();
    assert!(runtime.daemons().is_empty());
    runtime
        .orbe(dir.path(), &[&["open"][..], &session].concat())
        .ok();
    assert_eq!(
        fs::read_to_string(&version).unwrap(),
        format!("{}\n", env!("CARGO_PKG_VERSION"))
    );
    runtime
        .orbe(dir.path(), &["get", "@led1", "--session", "up"])
        .ok();
    runtime
        .orbe(dir.path(), &[&["close"][..], &session].concat())
        .ok();
    assert!(!version.exists());
}

#[test]
fn a_socket_path_past_the_system_limit_is_refused_before_anything_starts() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let long = runtime.dir.path().join("d".repeat(100));
    fs::create_dir(&long).unwrap();
    let socket = long.join("orbe").join("s.sock");
    let expected = format!(
        "error[io_error]: the socket path {} is {} bytes, and the system takes at most 107\n  hint: use a shorter `--session` name, or set XDG_RUNTIME_DIR to a shorter folder\n",
        socket.display(),
        socket.as_os_str().len()
    );
    for line in [&["open"][..], &["get", "@led1"]] {
        let run = Run::new(
            runtime
                .command(dir.path(), &[line, &["--session", "s"]].concat())
                .env("XDG_RUNTIME_DIR", &long)
                .output()
                .unwrap(),
        );
        assert_eq!(
            (run.code, run.stderr.as_str()),
            (3, expected.as_str()),
            "{line:?}"
        );
    }
    assert!(runtime.daemons().is_empty());
    let left = fs::read_dir(socket.parent().unwrap()).unwrap().count();
    assert_eq!(left, 0, "a refused start leaves no log");
}

#[test]
fn close_on_a_crashed_session_removes_its_socket() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["open", "--session", "j"]).ok();
    kill_daemon(&runtime);
    let run = runtime.orbe(dir.path(), &["close", "--session", "j"]).ok();
    assert_eq!(
        (run.stdout.as_str(), run.stderr.as_str()),
        (
            "removed the socket of a process that ended without closing\n",
            ""
        )
    );
    assert_eq!(
        fs::read_dir(runtime.sessions()).unwrap().count(),
        0,
        "socket and empty log gone"
    );
    let run = runtime.orbe(dir.path(), &["close", "--session", "j"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.starts_with("error[no_session]: "),
        "{}",
        run.stderr
    );
}

#[test]
fn close_all_reports_an_entry_it_cannot_remove_as_what_it_is() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime
        .orbe(dir.path(), &["open", "--session", "dead"])
        .ok();
    kill_daemon(&runtime);
    let folder = runtime.sessions().join("d.sock");
    fs::create_dir(&folder).unwrap();
    let refused = format!(
        "d: error[io_error]: {shown} is a folder, not the socket of session `d`\n  hint: remove it with `rm -r {shown}`\n",
        shown = folder.display()
    );
    let run = runtime.orbe(dir.path(), &["close", "--all"]);
    assert_eq!(run.code, 3);
    assert_eq!(
        run.stdout,
        format!("{refused}dead: removed the socket of a process that ended without closing\n")
    );
    let run = runtime.orbe(dir.path(), &["close", "--all"]);
    assert_eq!((run.code, run.stdout), (3, refused));
    fs::remove_dir(&folder).unwrap();
    let run = runtime.orbe(dir.path(), &["close", "--all"]).ok();
    assert_eq!(run.stdout, "no sessions are running\n");
}

#[test]
fn with_no_scene_above_the_no_session_hint_asks_for_the_scene_path() {
    let empty = tempfile::tempdir().unwrap();
    let runtime = Runtime::new();
    let run = runtime.orbe(empty.path(), &["get", "@led1", "--session", "x"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        "error[no_session]: no session `x` is running\n  hint: run `orbe open path/to/scene.jsonc --session x` to start it; no scene.jsonc is in this folder or above\n"
    );
}

#[test]
fn a_session_at_its_stop_answers_reads_and_refuses_inputs_with_ended() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let session = ["--session", "end"];
    let run = runtime
        .command(dir.path(), &[&["open"][..], &session].concat())
        .env("ORBE_TEST_STOP", "50ms")
        .output()
        .unwrap();
    Run::new(run).ok();
    let stepped = runtime
        .orbe(
            dir.path(),
            &[&["step", "50ms", "--json"][..], &session].concat(),
        )
        .ok()
        .json();
    assert_eq!(stepped["state"], "stopped");
    for read in [
        &["get", "@led1"][..],
        &["expect", "@led1", "off"],
        &["snapshot"],
    ] {
        let run = runtime.orbe(dir.path(), &[read, &session].concat()).ok();
        assert!(!run.stdout.is_empty(), "{read:?}");
    }
    let state = runtime
        .orbe(
            dir.path(),
            &[&["get", "@led1", "--json"][..], &session].concat(),
        )
        .ok()
        .json();
    assert_eq!(
        (&state["state"], &state["t_ps"]),
        (&"stopped".into(), &50_000_000_000u64.into())
    );
    let hint = "  hint: run `orbe open --session end` to start again\n";
    for (line, message) in [
        (&["press", "@btn1"][..], "the session has ended"),
        (&["step", "1ms"], "the session reached its 50ms stop"),
    ] {
        let run = runtime.orbe(dir.path(), &[line, &session].concat());
        assert_eq!(
            (run.code, run.stdout),
            (1, format!("error[ended]: {message}\n{hint}")),
            "{line:?}"
        );
    }
    assert_eq!(runtime.daemons().len(), 1, "the process stays at its stop");
    let run = runtime
        .orbe(dir.path(), &[&["close"][..], &session].concat())
        .ok();
    assert_eq!(run.stdout, "closed at t=50ms\n");
}

#[test]
fn close_all_ends_every_session_and_removes_stale_sockets() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime
        .orbe(dir.path(), &["open", "--session", "dead"])
        .ok();
    kill_daemon(&runtime);
    runtime.orbe(dir.path(), &["open", "--session", "a"]).ok();
    runtime.orbe(dir.path(), &["open"]).ok();
    let sockets = runtime.sockets();
    assert_eq!(sockets.len(), 3, "{sockets:?}");
    let default = sockets
        .iter()
        .find(|s| !["a.sock", "dead.sock"].contains(&s.as_str()))
        .unwrap();
    let folder = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .trim_start_matches('.')
        .to_string();
    let hash = default
        .strip_prefix(&format!("{folder}-"))
        .and_then(|rest| rest.strip_suffix(".sock"))
        .unwrap_or_else(|| panic!("{default} is not the project folder's name and a hash"));
    assert!(
        hash.len() == 8 && hash.chars().all(|c| c.is_ascii_hexdigit()),
        "{default}"
    );

    let run = runtime.orbe(dir.path(), &["close", "--all"]).ok();
    let name = default.strip_suffix(".sock").unwrap();
    let mut lines: Vec<String> = vec![
        "a: closed at t=0s".into(),
        "dead: removed the socket of a process that ended without closing".into(),
        format!("{name}: closed at t=0s"),
    ];
    lines.sort();
    assert_eq!(run.stdout, format!("{}\n", lines.join("\n")));
    wait_until("every closed process ends", || runtime.daemons().is_empty());
    assert!(runtime.sockets().is_empty());
    let run = runtime.orbe(dir.path(), &["close", "--all"]).ok();
    assert_eq!(run.stdout, "no sessions are running\n");
}

#[test]
fn two_projects_in_one_git_repo_get_their_own_sessions() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    let a = repo.path().join("a");
    let b = repo.path().join("b");
    for dir in [&a, &b] {
        common::copy_dir(project("open\n").path(), dir);
    }
    let runtime = Runtime::new();
    runtime.orbe(&a, &["open"]).ok();
    runtime.orbe(&a, &["step", "10ms"]).ok();
    runtime.orbe(&b, &["open"]).ok();
    let mut sockets = runtime.sockets();
    sockets.sort();
    assert_eq!(sockets.len(), 2, "{sockets:?}");
    assert!(
        sockets[0].starts_with("a-") && sockets[1].starts_with("b-"),
        "{sockets:?}"
    );

    let run = runtime.orbe(&a.join("tests"), &["snapshot"]).ok();
    assert!(run.stdout.starts_with("t=10ms "), "{}", run.stdout);
    let run = runtime.orbe(&b, &["snapshot"]).ok();
    assert!(run.stdout.starts_with("t=0s "), "{}", run.stdout);

    let outside = repo.path().join("notes");
    fs::create_dir(&outside).unwrap();
    let run = runtime.orbe(&outside, &["open", "../a/scene.jsonc"]).ok();
    assert!(run.stdout.starts_with("opened "), "{}", run.stdout);
    let folder = repo.path().file_name().unwrap().to_str().unwrap();
    let folder = folder.trim_start_matches('.');
    assert!(
        runtime
            .sockets()
            .iter()
            .any(|s| s.starts_with(&format!("{folder}-"))),
        "outside a project the git root names the session: {:?}",
        runtime.sockets()
    );
    runtime.orbe(&a, &["close", "--all"]).ok();
}

fn no_session(name: &str) -> String {
    format!(
        "error[no_session]: no session `{name}` is running\n  hint: run `orbe open --session {name}` to start it\n"
    )
}

#[test]
fn a_failed_open_leaves_no_socket_for_the_next_command() {
    let dir = project("open\n");
    fs::remove_dir_all(dir.path().join("parts/nano-board")).unwrap();
    let runtime = Runtime::new();
    let run = runtime.orbe(dir.path(), &["open", "--session", "miss"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("error[part_missing]"),
        "{}{}",
        run.stdout,
        run.stderr
    );
    assert!(
        runtime.sockets().is_empty(),
        "a failed open left a socket: {:?}",
        runtime.sockets()
    );
    let again = runtime.orbe(dir.path(), &["get", "@led1", "--session", "miss"]);
    assert_eq!(again.code, 1, "{}{}", again.stdout, again.stderr);
    assert_eq!(again.stderr, no_session("miss"), "{}", again.stdout);
    let retry = runtime.orbe(dir.path(), &["open", "--session", "miss"]);
    assert_eq!(retry.code, 1, "{}{}", retry.stdout, retry.stderr);
    assert!(
        retry.stdout.contains("error[part_missing]"),
        "{}{}",
        retry.stdout,
        retry.stderr
    );
    assert!(
        !format!("{}{}", retry.stdout, retry.stderr).contains("session_crashed"),
        "{}{}",
        retry.stdout,
        retry.stderr
    );
    assert!(
        runtime.sockets().is_empty(),
        "the retry left a socket: {:?}",
        runtime.sockets()
    );
}

#[test]
fn close_on_a_process_with_no_scene_changes_nothing_and_open_reaches_it() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let mut starting = runtime.listening(dir.path(), "ns", None);
    let pid = runtime.daemon();
    let run = runtime.orbe(dir.path(), &["close", "--all"]).ok();
    assert_eq!(
        run.stdout,
        "ns: no scene is open; its process is starting or ending\n"
    );
    assert_eq!(runtime.daemons(), [pid], "close --all leaves the process");
    for line in [&["close"][..], &["get", "@led1"]] {
        let run = runtime.orbe(dir.path(), &[line, &["--session", "ns"]].concat());
        assert_eq!(run.code, 1, "{line:?}");
        assert!(
            [run.stderr.as_str(), run.stdout.as_str()]
                .iter()
                .any(|out| out.ends_with(&no_session("ns"))),
            "{line:?}: {}{}",
            run.stdout,
            run.stderr
        );
        assert_eq!(runtime.daemons(), [pid], "{line:?} leaves the process");
        assert_eq!(runtime.sockets(), ["ns.sock"], "{line:?} leaves the socket");
    }
    let run = runtime.orbe(dir.path(), &["open", "--session", "ns"]).ok();
    assert!(
        run.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
    assert_eq!(runtime.daemons(), [pid], "the open reached that process");
    runtime.orbe(dir.path(), &["close", "--session", "ns"]).ok();
    assert!(starting.wait().unwrap().success(), "close ended it");
}

#[test]
fn a_process_with_no_scene_ends_on_its_own_while_commands_reach_it() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let mut starting = runtime.listening(dir.path(), "orphan", Some("500ms"));
    for _ in 0..30 {
        if starting.try_wait().unwrap().is_some() {
            break;
        }
        let run = runtime.orbe(dir.path(), &["get", "@led1", "--session", "orphan"]);
        assert_eq!((run.code, run.stderr), (1, no_session("orphan")));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(starting.try_wait().unwrap().is_some(), "the process ended");
    assert!(runtime.sockets().is_empty(), "and removed its socket");
}

#[test]
fn open_reaches_the_process_it_started_while_another_shell_closes_in_a_loop() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let done = AtomicBool::new(false);
    let open = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                runtime.orbe(dir.path(), &["close", "--session", "oc"]);
            }
        });
        std::thread::sleep(Duration::from_millis(200));
        let open = runtime.orbe(dir.path(), &["open", "--session", "oc"]);
        done.store(true, Ordering::Relaxed);
        open
    });
    assert_eq!(open.code, 0, "{}{}", open.stdout, open.stderr);
    assert!(
        open.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        open.stdout
    );
}

#[test]
fn opens_at_once_leave_only_the_listening_process_in_the_log() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let opens: Vec<_> = (0..4)
        .map(|_| runtime.spawn(dir.path(), &["open", "--session", "lg"]))
        .collect();
    for open in opens {
        let run = Run::new(open.wait_with_output().unwrap()).ok();
        assert!(run.stdout.starts_with("opened scene.jsonc at t=0: "));
    }
    runtime.daemon();
    let files = || {
        let mut names: Vec<String> = fs::read_dir(runtime.sessions())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    };
    wait_until("only the session's files remain", || {
        files() == ["lg.log", "lg.sock", "lg.version"]
    });
    let log = fs::read_to_string(runtime.sessions().join("lg.log")).unwrap();
    assert_eq!(log, "", "the log holds only the listening process's output");

    kill_daemon(&runtime);
    let run = runtime.orbe(dir.path(), &["get", "@led1", "--session", "lg"]);
    assert_eq!(
        run.stderr,
        "error[session_crashed]: the process of session `lg` ended without closing\n  hint: run `orbe open --session lg` to start it again\n"
    );
}

#[test]
fn a_command_whose_process_ends_without_answering_is_told_so_and_open_starts_again() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["open", "--session", "dr"]).ok();
    runtime
        .orbe(dir.path(), &["step", "5ms", "--session", "dr"])
        .ok();
    let pid = runtime.daemon();
    kill("-STOP", pid);
    let get = runtime.spawn(dir.path(), &["get", "@led1", "--session", "dr"]);
    let open = runtime.spawn(dir.path(), &["open", "--session", "dr"]);
    std::thread::sleep(Duration::from_millis(500));
    kill("-STOP", open.id());
    fs::remove_file(runtime.sessions().join("dr.sock")).unwrap();
    kill("-KILL", pid);

    let get = Run::new(get.wait_with_output().unwrap());
    kill("-CONT", open.id());
    assert_eq!(
        (get.code, get.stderr.as_str()),
        (
            1,
            "error[session_crashed]: the process of session `dr` ended before it answered\n  hint: run `orbe open --session dr` to start it again\n"
        )
    );
    let open = Run::new(open.wait_with_output().unwrap()).ok();
    assert!(
        open.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        open.stdout
    );
    runtime.orbe(dir.path(), &["close", "--session", "dr"]).ok();
}

#[test]
fn the_test_stop_only_lowers_the_stop_and_a_refused_one_names_its_value() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let open = |stop: &str| {
        Run::new(
            runtime
                .command(dir.path(), &["open", "--session", "sm"])
                .env("ORBE_TEST_STOP", stop)
                .output()
                .unwrap(),
        )
    };
    let longer =
        "is longer than the 2-hour stop\n  hint: set ORBE_TEST_STOP to at most 7200s, or unset it";
    let shorter = "is shorter than 10ms, the shortest stop the engine runs for this scene\n  hint: set ORBE_TEST_STOP to at least 10ms, or unset it";
    for (stop, shown, refusal) in [
        ("8000s", "8000s", longer),
        ("7201s", "7201s", longer),
        ("0s", "0s", shorter),
        ("1ps", "1ps", shorter),
        ("9999us", "9.999ms", shorter),
        (
            "soon",
            "soon",
            "is not a duration: `soon` is not a time unit; use ps, ns, us, ms or s\n  hint: set ORBE_TEST_STOP to a duration such as 50ms, or unset it",
        ),
    ] {
        let run = open(stop);
        assert_eq!(
            (run.code, run.stdout),
            (
                2,
                format!("error[usage]: ORBE_TEST_STOP={shown} {refusal}\n")
            ),
            "{stop}"
        );
    }

    let run = runtime.orbe(dir.path(), &["open", "--session", "sm"]).ok();
    assert!(
        run.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
    let run = runtime.orbe(dir.path(), &["step", "7201s", "--session", "sm"]);
    assert_eq!(
        (run.code, run.stdout.as_str()),
        (
            1,
            "error[past_stop]: the step would end past the session's 2-hour stop\n  hint: the session has 7200s left; step at most that\n"
        )
    );

    open("10ms").ok();
    let run = runtime.orbe(dir.path(), &["step", "20ms", "--session", "sm"]);
    assert_eq!(
        (run.code, run.stdout.as_str()),
        (
            1,
            "error[past_stop]: the step would end past the session's 10ms stop\n  hint: the session has 10ms left; step at most that\n"
        )
    );
    runtime.orbe(dir.path(), &["close", "--session", "sm"]).ok();
}

fn session_files(runtime: &Runtime) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(runtime.sessions())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn opens_at_once_over_a_killed_process_leave_no_start_log() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    for round in 0..10 {
        runtime.orbe(dir.path(), &["open", "--session", "sl"]).ok();
        let pids = runtime.daemons();
        for &pid in &pids {
            kill("-KILL", pid);
        }
        wait_until("every session process is dead", || {
            pids.iter().all(|&pid| harness::gone(pid))
        });
        let opens: Vec<_> = (0..8)
            .map(|_| runtime.spawn(dir.path(), &["open", "--session", "sl"]))
            .collect();
        for open in opens {
            let run = Run::new(open.wait_with_output().unwrap()).ok();
            assert!(run.stdout.starts_with("opened scene.jsonc at t=0: "));
        }
        wait_until(
            &format!("round {round} leaves only the session's files"),
            || session_files(&runtime) == ["sl.log", "sl.sock", "sl.version"],
        );
        runtime.orbe(dir.path(), &["close", "--session", "sl"]).ok();
    }
}

#[test]
fn a_start_log_left_at_the_id_a_start_would_take_is_passed_over() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["close", "--all"]).ok();
    let run = std::process::Command::new("sh")
        .args([
            "-c",
            r#"printf 'old\n' > "$2/pr.log.$(( $$ * 5 + 1 ))"; echo "$2/pr.log.$(( $$ * 5 + 1 ))" >&2; exec "$1" open --session pr"#,
            "sh",
            env!("CARGO_BIN_EXE_orbe"),
            runtime.sessions().to_str().unwrap(),
        ])
        .current_dir(dir.path())
        .env("XDG_RUNTIME_DIR", runtime.dir.path())
        .env_remove("ORBE_SESSION")
        .env_remove("ORBE_TEST_IDLE_TIMEOUT")
        .env_remove("ORBE_TEST_STOP")
        .output()
        .unwrap();
    let run = Run::new(run);
    let planted = run.stderr.trim_end().to_string();
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
    assert_eq!(fs::read_to_string(&planted).unwrap(), "old\n");
    runtime.orbe(dir.path(), &["close", "--session", "pr"]).ok();
    let run = runtime.orbe(dir.path(), &["close", "--all"]).ok();
    assert_eq!(
        run.stdout,
        format!("pr: removed {planted}, the log of a start that ended\n")
    );
    assert!(session_files(&runtime).is_empty());
}

#[test]
fn close_removes_the_start_logs_no_process_holds_and_keeps_a_held_one() {
    use rustix::fs::{FlockOperation, flock};
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["open", "--session", "x"]).ok();
    kill_daemon(&runtime);
    let sessions = runtime.sessions();
    let (x, y, z) = (
        sessions.join("x.log.7"),
        sessions.join("y.log.8"),
        sessions.join("z.log.9"),
    );
    for start in [&x, &y, &z] {
        fs::write(start, "died\n").unwrap();
    }
    let held = fs::File::open(&z).unwrap();
    flock(&held, FlockOperation::LockExclusive).unwrap();

    let run = runtime.orbe(dir.path(), &["close", "--session", "x"]).ok();
    assert_eq!(
        run.stdout,
        format!(
            "removed the socket of a process that ended without closing; removed {}, the log of a start that ended\n",
            x.display()
        )
    );
    let run = runtime.orbe(dir.path(), &["close", "--all"]).ok();
    assert_eq!(
        run.stdout,
        format!(
            "y: removed {}, the log of a start that ended\n",
            y.display()
        )
    );
    assert_eq!(session_files(&runtime), ["z.log.9"], "the held log stays");
    drop(held);
    let run = runtime.orbe(dir.path(), &["close", "--all", "--json"]).ok();
    assert_eq!(
        run.json()["data"]["sessions"],
        serde_json::json!([{ "ok": true, "session": "z", "removed": "start_log", "log": z.display().to_string() }])
    );
    assert!(session_files(&runtime).is_empty());
}

#[test]
fn anything_but_a_file_at_the_log_is_refused_before_anything_starts() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["close", "--all"]).ok();
    let log = runtime.sessions().join("nf.log");
    for (what, remove, make) in [
        ("a folder", "rm -r", "mkdir"),
        ("a special file", "rm", "mkfifo"),
    ] {
        assert!(
            std::process::Command::new(make)
                .arg(&log)
                .status()
                .unwrap()
                .success()
        );
        let run = runtime.orbe(dir.path(), &["open", "--session", "nf"]);
        assert_eq!(
            (run.code, run.stdout.as_str(), run.stderr),
            (
                3,
                "",
                format!(
                    "error[io_error]: {shown} is {what}, not the log of session `nf`\n  hint: remove it with `{remove} {shown}`\n",
                    shown = log.display()
                )
            ),
            "{what}"
        );
        assert!(runtime.daemons().is_empty(), "{what}");
        assert_eq!(session_files(&runtime), ["nf.log"], "{what}");
        let _ = fs::remove_dir(&log);
        let _ = fs::remove_file(&log);
    }
}

#[test]
fn a_command_whose_process_crashed_while_another_shell_opened_again_says_so() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    runtime.orbe(dir.path(), &["open", "--session", "cr"]).ok();
    let stepping = runtime.spawn(dir.path(), &["step", "600s", "--session", "cr"]);
    std::thread::sleep(Duration::from_millis(300));
    kill("-STOP", stepping.id());
    kill_daemon(&runtime);
    runtime.orbe(dir.path(), &["open", "--session", "cr"]).ok();
    runtime
        .orbe(dir.path(), &["step", "1ms", "--session", "cr"])
        .ok();
    kill("-CONT", stepping.id());
    let run = Run::new(stepping.wait_with_output().unwrap());
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (
            1,
            "",
            "error[session_crashed]: the process of session `cr` ended before it answered; a new session `cr` is running\n  hint: run `orbe snapshot --session cr` to see it\n"
        )
    );
    let run = runtime
        .orbe(dir.path(), &["get", "@led1", "--json", "--session", "cr"])
        .ok()
        .json();
    assert_eq!(
        run["t_ps"], 1_000_000_000u64,
        "the new session is untouched"
    );
}

#[test]
fn close_all_after_a_refused_open_finds_nothing_and_exits_0() {
    let dir = project("open\n");
    fs::write(dir.path().join("bad.jsonc"), "{").unwrap();
    let runtime = Runtime::new();
    let run = runtime.orbe(dir.path(), &["open", "bad.jsonc", "--session", "ca"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        runtime.sockets().is_empty(),
        "a refused open left a socket: {:?}",
        runtime.sockets()
    );
    let run = runtime.orbe(dir.path(), &["close", "--all"]);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "no sessions are running\n", "")
    );
    assert!(runtime.daemons().is_empty(), "{:?}", runtime.daemons());
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the clock shows the request was read late"
)]
fn an_open_read_after_the_no_scene_wait_still_opens_on_the_process_it_started() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let started = std::time::Instant::now();
    let run = runtime
        .command(dir.path(), &["open", "--session", "late"])
        .env("ORBE_TEST_READ_DELAY", "5500ms")
        .output()
        .unwrap();
    let elapsed = started.elapsed();
    let run = Run::new(run).ok();
    assert!(
        run.stdout.starts_with("opened scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
    assert!(
        elapsed >= Duration::from_millis(5500),
        "the request was read late: {elapsed:?}"
    );
    assert_eq!(runtime.daemons().len(), 1);
    assert_eq!(
        session_files(&runtime),
        ["late.log", "late.sock", "late.version"]
    );
    runtime
        .orbe(dir.path(), &["close", "--session", "late"])
        .ok();
}

#[test]
fn an_open_whose_every_process_ends_before_reading_it_says_how_many_times() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let run = runtime
        .command(dir.path(), &["open", "--session", "rd"])
        .env("ORBE_TEST_IDLE_TIMEOUT", "300ms")
        .env("ORBE_TEST_READ_DELAY", "3s")
        .output()
        .unwrap();
    let run = Run::new(run);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (
            1,
            "",
            "error[session_crashed]: the process of session `rd` ended before it answered `open`, 5 times\n"
        )
    );
    wait_until("every process ended", || runtime.daemons().is_empty());
    assert!(session_files(&runtime).is_empty());
}

/// A scene whose name starts with `-` reads as a flag, live and in a test file, and the error
/// names the `./` form. `--` before it opens it too.
#[test]
fn a_scene_named_like_a_flag_opens_as_dot_slash_or_after_dashes() {
    let dir = project("open -dash.jsonc\n");
    let root = dir.path();
    fs::copy(root.join("scene.jsonc"), root.join("-dash.jsonc")).unwrap();
    let runtime = Runtime::new();
    let refused = "`-dash.jsonc` is a flag, not a scene; write a scene named so as `./-dash.jsonc`; usage: open [<scene>]";
    let run = runtime.orbe(root, &["open", "-dash.jsonc"]);
    assert_eq!(
        (run.code, run.stderr.as_str()),
        (2, format!("error[usage]: {refused}\n").as_str())
    );
    let tested = runtime.orbe(root, &["test"]);
    assert_eq!(tested.code, 2, "{}", tested.stdout);
    assert!(tested.stdout.contains(refused), "{}", tested.stdout);
    for scene in [&["./-dash.jsonc"][..], &["--", "-dash.jsonc"]] {
        let run = runtime.orbe(root, &[&["open"][..], scene].concat()).ok();
        assert!(run.stdout.contains("-dash.jsonc at t=0"), "{}", run.stdout);
        runtime.orbe(root, &["close"]).ok();
        fs::write(
            root.join("tests/a.test"),
            format!("open {}\n", scene.join(" ")),
        )
        .unwrap();
        runtime.orbe(root, &["test"]).ok();
    }
}

#[test]
fn check_on_a_missing_scene_uses_the_same_hint_as_open() {
    let runtime = Runtime::new();
    let dir = project("open\n");
    let text = "error[io_error]: no scene file at missing.jsonc\n  hint: write the scene, or name its path\n";
    let check = runtime.orbe(dir.path(), &["check", "missing.jsonc"]);
    assert_eq!(
        (check.code, check.stdout.as_str(), check.stderr.as_str()),
        (3, "", text)
    );
    let open = runtime.orbe(dir.path(), &["open", "missing.jsonc"]);
    assert_eq!(
        (open.code, open.stdout.as_str(), open.stderr.as_str()),
        (3, text, "")
    );
    let check_json = runtime.orbe(dir.path(), &["check", "missing.jsonc", "--json"]);
    assert_eq!(check_json.code, 3, "{}", check_json.stdout);
    let check_error = &check_json.json()["error"];
    let open_json = runtime.orbe(dir.path(), &["open", "missing.jsonc", "--json"]);
    assert_eq!(open_json.code, 3, "{}", open_json.stdout);
    let open_error = &open_json.json()["error"];
    assert_eq!(check_error, open_error);
    assert_eq!(check_error["code"], "io_error");
    assert_eq!(check_error["message"], "no scene file at missing.jsonc");
    assert_eq!(check_error["hint"], "write the scene, or name its path");
}
