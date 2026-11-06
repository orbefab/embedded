//! The live commands between separate shell commands, each run against the real session
//! process. Every test gets its own runtime folder, and kills the processes it started.

#![cfg(target_os = "linux")]

#[path = "session/harness.rs"]
mod harness;
mod live;

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

use harness::{Run, Runtime, kill, kill_daemon, wait_until};
use live::{common, project};

/// The first target laid out as in the repo, so `examples/first-target/scene.jsonc` names the
/// same files from the folder's root, with `lines` as `tests/live.test`.
fn first_target(lines: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let example = dir.path().join("examples/first-target");
    common::copy_dir(&common::repo().join("examples/first-target"), &example);
    let sketch = dir.path().join("engine/firmware/sketch");
    fs::create_dir_all(&sketch).unwrap();
    fs::copy(common::firmware(), sketch.join("button.hex")).unwrap();
    fs::write(example.join("tests/live.test"), lines).unwrap();
    dir
}

const FIRST_TARGET: [&str; 8] = [
    "open examples/first-target/scene.jsonc",
    "step 100ms",
    "expect @led1 off",
    "press @btn1",
    "step --until @led1 on --timeout 50ms",
    "expect @nano.D13 high",
    "get @nano.D13",
    "close",
];

/// Each reply `orbe test -v` prints, de-indented, as the live command prints it.
fn verbose_replies(stdout: &str) -> Vec<String> {
    let mut replies: Vec<String> = Vec::new();
    for line in stdout.lines() {
        if line.starts_with("examples/first-target/tests/live.test:") {
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
        .map(|l| format!("{}\n", l.replace("examples/first-target/", "")))
        .collect();
    let dir = first_target(&lines);
    let root = dir.path();
    let runtime = Runtime::new();
    let tested = runtime
        .orbe(
            root,
            &["test", "-v", "examples/first-target/tests/live.test"],
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
    assert!(runtime.daemons().is_empty(), "close ends the process");
    assert!(runtime.sockets().is_empty(), "close removes the socket");

    let tested = runtime
        .orbe(
            root,
            &[
                "test",
                "--json",
                "-v",
                "examples/first-target/tests/live.test",
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
    assert_eq!(run.stdout, "nano.D13 high 4.621848245658706 V\n");
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
    assert_eq!(run.stdout, "t=10ms (from 0s)\n");
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
    assert!(runtime.daemons().is_empty());
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
    let mut starting = runtime
        .command(dir.path(), &["daemon", "--session=st"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until("the process listens", || runtime.sockets() == ["st.sock"]);
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
            "error[usage]: takes one ref, such as `@btn1`; usage: get <ref>\n"
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
fn a_socket_path_past_the_system_limit_is_refused_before_anything_starts() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let long = runtime.dir.path().join("d".repeat(100));
    fs::create_dir(&long).unwrap();
    let socket = long
        .join("orbe")
        .join(env!("CARGO_PKG_VERSION"))
        .join("s.sock");
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
        (&["step", "1ms"], "the session reached its 2-hour stop"),
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
    assert!(runtime.daemons().is_empty());
    assert!(runtime.sockets().is_empty());
    let run = runtime.orbe(dir.path(), &["close", "--all"]).ok();
    assert_eq!(run.stdout, "no sessions are running\n");
}
