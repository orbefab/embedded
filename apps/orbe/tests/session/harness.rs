//! The real `orbe` binary and the session processes it starts, each test in its own runtime
//! folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn new(output: Output) -> Run {
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}: {}", self.stdout))
    }

    pub fn ok(self) -> Run {
        assert_eq!(self.code, 0, "{}{}", self.stdout, self.stderr);
        self
    }
}

/// A private `XDG_RUNTIME_DIR`. Dropping it kills every session process that used it.
pub struct Runtime {
    pub dir: TempDir,
}

impl Runtime {
    pub fn new() -> Runtime {
        Runtime {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orbe"));
        command
            .args(args)
            .current_dir(cwd)
            .env("XDG_RUNTIME_DIR", self.dir.path())
            .env_remove("ORBE_SESSION")
            .env_remove("ORBE_TEST_IDLE_TIMEOUT")
            .env_remove("ORBE_TEST_STOP");
        command
    }

    pub fn orbe(&self, cwd: &Path, args: &[&str]) -> Run {
        Run::new(self.command(cwd, args).output().unwrap())
    }

    pub fn spawn(&self, cwd: &Path, args: &[&str]) -> Child {
        self.command(cwd, args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    pub fn sessions(&self) -> PathBuf {
        self.dir.path().join("orbe")
    }

    pub fn sockets(&self) -> Vec<String> {
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

    /// The pids of live session processes started with this runtime folder, or a folder in it.
    pub fn daemons(&self) -> Vec<u32> {
        let wanted = format!("XDG_RUNTIME_DIR={}", self.dir.path().display());
        let mine = |v: &[u8]| {
            v.strip_prefix(wanted.as_bytes())
                .is_some_and(|rest| rest.is_empty() || rest[0] == b'/')
        };
        fs::read_dir("/proc")
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| {
                let read = |file: &str| fs::read(format!("/proc/{pid}/{file}")).unwrap_or_default();
                let cmdline = read("cmdline");
                let args: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
                args.get(1) == Some(&&b"daemon"[..])
                    && read("environ").split(|&b| b == 0).any(mine)
                    && !gone(*pid)
            })
            .collect()
    }

    /// `orbe daemon --session=<name>` started by hand, once it listens, with no scene open.
    pub fn listening(&self, cwd: &Path, name: &str, idle: Option<&str>) -> Child {
        let mut command = self.command(cwd, &["daemon", &format!("--session={name}")]);
        if let Some(idle) = idle {
            command.env("ORBE_TEST_IDLE_TIMEOUT", idle);
        }
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let socket = format!("{name}.sock");
        wait_until("the process listens", || self.sockets().contains(&socket));
        child
    }

    pub fn daemon(&self) -> u32 {
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

pub fn kill(signal: &str, pid: u32) {
    Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .unwrap();
}

/// Dead, or a zombie its parent has not reaped. A killed process's first thread can be a
/// zombie while its other threads still hold its sockets open, so those must be gone too.
pub fn gone(pid: u32) -> bool {
    let zombie = fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
        stat.rsplit(") ").next().is_some_and(|s| s.starts_with('Z'))
    });
    let threads = fs::read_dir(format!("/proc/{pid}/task")).map_or(0, Iterator::count);
    zombie && threads <= 1
}

pub fn wait_until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting until {what}");
}

pub fn kill_daemon(runtime: &Runtime) {
    let pid = runtime.daemon();
    kill("-KILL", pid);
    wait_until("the session process is dead", || gone(pid));
}
