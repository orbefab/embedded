//! The live commands between separate shell commands, each run against the real session
//! process. Every test gets its own runtime folder, and kills the processes it started.

#![cfg(target_os = "linux")]

mod live;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

use live::{common, project};

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn new(output: Output) -> Run {
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}: {}", self.stdout))
    }

    fn ok(self) -> Run {
        assert_eq!(self.code, 0, "{}{}", self.stdout, self.stderr);
        self
    }
}

/// A private `XDG_RUNTIME_DIR`. Dropping it kills every session process that used it.
struct Runtime {
    dir: TempDir,
}

impl Runtime {
    fn new() -> Runtime {
        Runtime {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orbe"));
        command
            .args(args)
            .current_dir(cwd)
            .env("XDG_RUNTIME_DIR", self.dir.path())
            .env_remove("ORBE_SESSION")
            .env_remove("ORBE_TEST_IDLE_TIMEOUT");
        command
    }

    fn orbe(&self, cwd: &Path, args: &[&str]) -> Run {
        Run::new(self.command(cwd, args).output().unwrap())
    }

    fn spawn(&self, cwd: &Path, args: &[&str]) -> Child {
        self.command(cwd, args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn sessions(&self) -> PathBuf {
        self.dir.path().join("orbe").join(env!("CARGO_PKG_VERSION"))
    }

    fn sockets(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.sessions())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok()?.file_name().into_string().ok())
                    .filter(|n| n.ends_with(".sock"))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The pids of live session processes started with this runtime folder.
    fn daemons(&self) -> Vec<u32> {
        let wanted = format!("XDG_RUNTIME_DIR={}", self.dir.path().display());
        fs::read_dir("/proc")
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| {
                let read = |file: &str| fs::read(format!("/proc/{pid}/{file}")).unwrap_or_default();
                let cmdline = read("cmdline");
                let args: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
                args.get(1) == Some(&&b"daemon"[..])
                    && read("environ")
                        .split(|&b| b == 0)
                        .any(|v| v == wanted.as_bytes())
                    && !gone(*pid)
            })
            .collect()
    }

    fn daemon(&self) -> u32 {
        match self.daemons().as_slice() {
            [pid] => *pid,
            pids => panic!("expected one session process, found {pids:?}"),
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        for pid in self.daemons() {
            kill("-KILL", pid);
        }
    }
}

fn kill(signal: &str, pid: u32) {
    Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .unwrap();
}

/// Dead, or a zombie its parent has not reaped.
fn gone(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
        stat.rsplit(") ").next().is_some_and(|s| s.starts_with('Z'))
    })
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting until {what}");
}

fn kill_daemon(runtime: &Runtime) {
    let pid = runtime.daemon();
    kill("-KILL", pid);
    wait_until("the session process is dead", || gone(pid));
}

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
fn a_session_folder_another_user_owns_is_refused() {
    let dir = project("open\n");
    let runtime = Runtime::new();
    let temp = runtime.dir.path();
    let uid = fs::metadata(temp).unwrap().uid();
    let folder = temp.join(format!("orbe-{uid}"));
    std::os::unix::fs::symlink("/", &folder).unwrap();
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
            "error[io_error]: the session folder {} is owned by another user (uid 0), not you (uid {uid})\n  hint: set XDG_RUNTIME_DIR to a folder you own\n",
            folder.display()
        )
    );
    assert!(runtime.daemons().is_empty());
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
