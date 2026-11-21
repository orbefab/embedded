//! `orbe show`: the viewer page and the `/session` WebSocket a live session serves, with a
//! hand-built page folder in `ORBE_VIEWER_DIR` and never a browser.

#![cfg(target_os = "linux")]

#[allow(dead_code)]
#[path = "session/harness.rs"]
mod harness;
mod live;

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use harness::{Run, Runtime, gone, wait_until};
use live::project;

/// A page folder with only an `index.html`.
fn page() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("index.html"), "<p>viewer</p>").unwrap();
    dir
}

/// `orbe show --no-browser` with `page` as the page folder.
fn show(runtime: &Runtime, dir: &Path, page: &Path) -> Run {
    let output = runtime
        .command(dir, &["show", "--no-browser"])
        .env("ORBE_VIEWER_DIR", page)
        .output()
        .unwrap();
    Run::new(output)
}

/// The address, as `127.0.0.1:<port>`, and the token of a URL `show` printed.
fn split(url: &str) -> (String, String) {
    let url = url.trim();
    let (base, token) = url.split_once("/#t=").expect("a URL with a token");
    let host = base.strip_prefix("http://").unwrap().to_string();
    (host, token.to_string())
}

/// One request's response head, as the status line.
fn status(host: &str, request: &str) -> String {
    let mut stream = TcpStream::connect(host).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let response = String::from_utf8_lossy(&response);
    response.lines().next().unwrap_or_default().to_string()
}

/// A WebSocket upgrade request, with the key from RFC 6455's example.
fn upgrade(path: &str, host: &str, origin: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n",
    )
}

/// A viewer on the session, past the upgrade.
struct Viewer {
    socket: WebSocket<TcpStream>,
}

impl Viewer {
    fn connect(url: &str) -> Viewer {
        let (host, token) = split(url);
        let mut stream = TcpStream::connect(&host).unwrap();
        let request = upgrade(
            &format!("/session?t={token}"),
            &host,
            &format!("http://{host}"),
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut head = Vec::new();
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(
            head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
            "{head}"
        );
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        Viewer {
            socket: WebSocket::from_raw_socket(stream, Role::Client, None),
        }
    }

    /// The next message, `None` once the session closed the socket.
    fn next(&mut self) -> Option<Value> {
        for _ in 0..200 {
            match self.socket.read() {
                Ok(Message::Text(text)) => return Some(serde_json::from_str(&text).unwrap()),
                Ok(Message::Close(frame)) => {
                    let frame = frame.expect("a close frame with a reason");
                    assert_eq!(frame.reason.as_str(), "the session ended");
                    return None;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(tungstenite::Error::ConnectionClosed) => return None,
                Err(e) => panic!("{e}"),
            }
        }
        panic!("no message from the session in 200 reads of 50 ms");
    }

    /// Closes the socket, and waits until the session answered the close.
    fn leave(mut self) {
        let _ = self.socket.close(None);
        for _ in 0..200 {
            match self.socket.read() {
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return;
                }
                Err(tungstenite::Error::Io(e))
                    if !matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    return;
                }
                _ => {}
            }
        }
        panic!("the session did not answer the close in 200 reads of 50 ms");
    }

    /// Messages until one of `kind`, which it returns.
    fn until(&mut self, kind: &str, seen: &mut Vec<Value>) -> Value {
        loop {
            let message = self.next().expect("the session closed first");
            seen.push(message.clone());
            if message["type"] == kind {
                return message;
            }
        }
    }

    fn send(&mut self, message: Value) {
        self.socket
            .send(Message::text(message.to_string()))
            .unwrap();
    }

    /// The reply to one command the viewer sends, keeping what came before it in `seen`.
    fn run(&mut self, id: u64, cmd: &str, r#ref: Option<&str>, seen: &mut Vec<Value>) -> Value {
        let mut message = json!({"id": id, "cmd": cmd});
        if let Some(r#ref) = r#ref {
            message["ref"] = json!(r#ref);
        }
        self.send(message);
        let reply = self.until("reply", seen);
        assert_eq!(reply["id"], id);
        reply["reply"].clone()
    }
}

/// `value` without its `session` field, which differs between two projects.
fn unnamed(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("session");
    value
}

fn opened(runtime: &Runtime) -> (TempDir, TempDir, String) {
    let dir = project("open\n");
    let page = page();
    runtime.orbe(dir.path(), &["open"]).ok();
    let url = show(runtime, dir.path(), page.path()).ok().stdout;
    (dir, page, url)
}

#[test]
fn show_with_no_session_says_which_open_to_run() {
    let dir = project("open\n");
    let page = page();
    let runtime = Runtime::new();
    let run = show(&runtime, dir.path(), page.path());
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr.starts_with("error[no_session]"),
        "{}",
        run.stderr
    );
    assert!(runtime.daemons().is_empty());
}

#[test]
fn show_names_a_page_folder_with_no_index() {
    let runtime = Runtime::new();
    let dir = project("open\n");
    let empty = tempfile::tempdir().unwrap();
    runtime.orbe(dir.path(), &["open"]).ok();
    let run = show(&runtime, dir.path(), empty.path());
    assert_eq!(run.code, 3, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "error[io_error]: ORBE_VIEWER_DIR={} holds no index.html\n  hint: build the page into that folder, or unset ORBE_VIEWER_DIR\n",
            empty.path().display()
        )
    );
    runtime.orbe(dir.path(), &["close"]).ok();
}

#[test]
fn open_headed_whose_show_fails_says_the_session_is_open() {
    let runtime = Runtime::new();
    let dir = project("open\n");
    let empty = tempfile::tempdir().unwrap();
    let headed = |json: bool| {
        let mut args = vec!["open", "--headed", "--no-browser"];
        args.extend(json.then_some("--json"));
        let output = runtime
            .command(dir.path(), &args)
            .env("ORBE_VIEWER_DIR", empty.path())
            .output()
            .unwrap();
        Run::new(output)
    };
    let hint = "build the page into that folder, or unset ORBE_VIEWER_DIR; the session stays open, so run `orbe show` to try again, or `orbe close` to end it";

    let text = headed(false);
    assert_eq!(text.code, 3, "{}{}", text.stdout, text.stderr);
    let lines: Vec<&str> = text.stdout.lines().collect();
    assert!(
        lines[0].starts_with("opened scene.jsonc at t=0: "),
        "{}",
        text.stdout
    );
    assert_eq!(
        lines[lines.len() - 2..],
        [
            format!(
                "error[io_error]: ORBE_VIEWER_DIR={} holds no index.html",
                empty.path().display()
            ),
            format!("  hint: {hint}"),
        ]
    );

    let json = headed(true);
    assert_eq!(json.code, 3, "{}{}", json.stdout, json.stderr);
    let envelope = json.json();
    assert_eq!(envelope["ok"], false, "{envelope}");
    assert_eq!(envelope["state"], "paused");
    assert_eq!(envelope["data"]["scene"], "scene.jsonc");
    assert_eq!(envelope["error"]["code"], "io_error");
    assert_eq!(envelope["error"]["hint"], hint);

    let get = runtime.orbe(dir.path(), &["get", "@led1", "--json"]).ok();
    assert_eq!(get.json()["state"], "paused", "the session is open");
    runtime.orbe(dir.path(), &["close"]).ok();
}

#[test]
fn the_server_refuses_a_wrong_token_host_or_origin() {
    let runtime = Runtime::new();
    let (dir, _page, url) = opened(&runtime);
    let (host, token) = split(&url);
    let origin = format!("http://{host}");
    let get = |path: &str, header_host: &str| {
        status(
            &host,
            &format!("GET {path} HTTP/1.1\r\nHost: {header_host}\r\n\r\n"),
        )
    };
    assert_eq!(get("/", &host), "HTTP/1.1 200 OK");
    assert_eq!(get("/", "evil.example"), "HTTP/1.1 403 Forbidden");
    assert_eq!(get("/", "localhost"), "HTTP/1.1 403 Forbidden");
    assert_eq!(
        get("/parts/../scene.jsonc", &host),
        "HTTP/1.1 404 Not Found"
    );
    assert_eq!(get("/parts/led-5mm/led-5mm.glb", &host), "HTTP/1.1 200 OK");

    let last = if token.ends_with('0') { '1' } else { '0' };
    let wrong = format!("{}{last}", &token[..token.len() - 1]);
    for (path, header_host, from) in [
        ("/session".to_string(), host.as_str(), origin.as_str()),
        (format!("/session?t={wrong}"), &host, &origin),
        (format!("/session?t={token}"), "evil.example", &origin),
        (format!("/session?t={token}"), &host, "http://evil.example"),
        (format!("/session?t={token}"), &host, "null"),
    ] {
        assert_eq!(
            status(&host, &upgrade(&path, header_host, from)),
            "HTTP/1.1 403 Forbidden",
            "{path} {header_host} {from}"
        );
    }
    Viewer::connect(&url);
    runtime.orbe(dir.path(), &["close"]).ok();
}

/// Whether the server closed `stream` within `within`, without a response. The extra
/// connection is closed from `accept`, which a busy machine can reach late. `within` is how
/// long this test watches for that close. It is not a deadline the server has.
fn dropped(mut stream: &TcpStream, within: Duration) -> bool {
    stream.set_read_timeout(Some(within)).unwrap();
    matches!(stream.read(&mut [0; 64]), Ok(0))
}

#[test]
fn the_server_serves_32_connections_at_once_and_drops_the_rest() {
    let runtime = Runtime::new();
    let (dir, _page, url) = opened(&runtime);
    let (host, _) = split(&url);
    let held: Vec<TcpStream> = (0..32)
        .map(|_| TcpStream::connect(&host).unwrap())
        .collect();
    let over = TcpStream::connect(&host).unwrap();
    assert!(
        dropped(&over, Duration::from_secs(5)),
        "a 33rd connection is closed at once"
    );
    assert!(
        !dropped(&held[31], Duration::from_secs(1)),
        "the 32nd still waits for its request"
    );
    drop(held);
    let get = format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n");
    wait_until("a connection is served again", || {
        status(&host, &get) == "HTTP/1.1 200 OK"
    });
    runtime.orbe(dir.path(), &["close"]).ok();
}

/// Whether `stream` closed within `within`.
fn shut(stream: &mut TcpStream, within: Duration) -> bool {
    stream.set_read_timeout(Some(within)).unwrap();
    let read = stream.read(&mut [0; 64]);
    matches!(&read, Ok(0))
        || read
            .as_ref()
            .is_err_and(|e| e.kind() == ErrorKind::ConnectionReset)
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the clock bounds how long a request head may take"
)]
fn a_request_head_has_5_s_in_all() {
    let runtime = Runtime::new();
    let (dir, _page, url) = opened(&runtime);
    let (host, _) = split(&url);
    let mut slow = TcpStream::connect(&host).unwrap();
    let started = Instant::now();
    for (i, byte) in b"GET ".iter().enumerate() {
        let _ = slow.write_all(&[*byte]);
        // The listener's 5 s starts when it accepts, which can be after this clock. A probe
        // well before 5 s still has to see an open connection. One near the deadline would
        // fail when accept itself was late.
        if started.elapsed() < Duration::from_secs(3) {
            slow.set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let early = slow.read(&mut [0; 64]);
            assert!(
                early.as_ref().is_err_and(|e| {
                    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                }),
                "still open before 5 s, after byte {i}: {early:?}"
            );
        }
        std::thread::sleep(Duration::from_millis(1750));
    }
    // The bytes already span more than 5 s. The further wait is only so a late accept can
    // still show the close. A head the listener never cuts off stays open through it.
    let left = Duration::from_secs(10).saturating_sub(started.elapsed());
    assert!(
        shut(&mut slow, left),
        "a byte every 1.75 s is cut off 5 s after the first"
    );
    assert!(
        started.elapsed() >= Duration::from_secs(5),
        "the close arrived before 5 s: {:?}",
        started.elapsed()
    );
    runtime.orbe(dir.path(), &["close"]).ok();
}

/// The CPU time the process has used, in clock ticks: utime plus stime, fields 14 and 15 of
/// `/proc/<pid>/stat`, counted after the `) ` that ends the command name.
fn cpu_ticks(pid: u32) -> u64 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<&str> = stat.rsplit(") ").next().unwrap().split(' ').collect();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

#[test]
fn a_listener_out_of_files_waits_instead_of_spinning() {
    let runtime = Runtime::new();
    let (dir, _page, url) = opened(&runtime);
    let (host, _) = split(&url);
    let pid = runtime.daemon();
    let files = fs::read_dir(format!("/proc/{pid}/fd")).unwrap().count();
    let limits = fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
    let soft = limits
        .lines()
        .find_map(|line| line.strip_prefix("Max open files"))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .to_string();
    let limit = |soft: &str| {
        let status = std::process::Command::new("prlimit")
            .args(["--pid", &pid.to_string(), &format!("--nofile={soft}:")])
            .status()
            .unwrap();
        assert!(status.success(), "prlimit");
    };
    limit(&files.to_string());
    let waiting: Vec<TcpStream> = (0..3).map(|_| TcpStream::connect(&host).unwrap()).collect();
    std::thread::sleep(Duration::from_millis(200));
    let before = cpu_ticks(pid);
    std::thread::sleep(Duration::from_secs(1));
    let used = cpu_ticks(pid) - before;
    limit(&soft);
    drop(waiting);
    assert!(used < 20, "{used} ticks of CPU in 1 s of failing accepts");
    runtime.orbe(dir.path(), &["close"]).ok();
}

#[test]
fn a_viewer_press_is_a_cli_press() {
    let runtime = Runtime::new();
    let (watched, _page, url) = opened(&runtime);
    let mut viewer = Viewer::connect(&url);
    let mut seen = Vec::new();
    let scene = viewer.until("scene", &mut seen);
    assert_eq!(scene["channels"]["switches"][0], "@btn1");
    let plain = project("open\n");
    runtime.orbe(plain.path(), &["open"]).ok();

    runtime.orbe(watched.path(), &["step", "50ms"]).ok();
    runtime.orbe(plain.path(), &["step", "50ms"]).ok();
    let by_viewer = viewer.run(1, "press", Some("@btn1"), &mut seen);
    let by_cli = runtime
        .orbe(plain.path(), &["press", "@btn1", "--json"])
        .ok()
        .json();
    assert_eq!(unnamed(by_viewer), unnamed(by_cli));

    for args in [
        &["step", "50ms", "--json"][..],
        &["snapshot", "--json"],
        &["get", "@led1", "--json"],
    ] {
        let a = runtime.orbe(watched.path(), args).ok().json();
        let b = runtime.orbe(plain.path(), args).ok().json();
        assert_eq!(unnamed(a), unnamed(b), "{args:?}");
    }

    let refused = viewer.run(2, "step", None, &mut seen);
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"]["code"], "usage");
    let no_ref = viewer.run(3, "press", None, &mut seen);
    assert_eq!(no_ref["ok"], false);

    let last = seen.iter().rev().find(|m| m["type"] == "frame").unwrap();
    assert_eq!(
        last["switches"][0], 1,
        "the contact closed after the press landed"
    );
    runtime.orbe(watched.path(), &["close"]).ok();
    runtime.orbe(plain.path(), &["close"]).ok();
}

const LINES: [&str; 7] = [
    "step 100ms",
    "expect @led1 off",
    "press @btn1",
    "step --until @led1 lit --timeout 50ms",
    "expect @nano.D13 high",
    "release @btn1",
    "step 300ms",
];

#[test]
fn a_viewer_watching_a_step_changes_no_reply() {
    let runtime = Runtime::new();
    let (watched, _page, url) = opened(&runtime);
    let mut viewer = Viewer::connect(&url);
    let plain = project("open\n");
    runtime.orbe(plain.path(), &["open"]).ok();

    let mut end = 0;
    for line in LINES {
        let mut args: Vec<&str> = line.split(' ').collect();
        args.push("--json");
        let a = runtime.orbe(watched.path(), &args).ok().json();
        let b = runtime.orbe(plain.path(), &args).ok().json();
        end = a["t_ps"].as_u64().unwrap();
        assert_eq!(unnamed(a), unnamed(b), "{line}");
    }

    let mut frames = Vec::new();
    while frames.last().map(|f: &Value| f["t_ps"].as_u64().unwrap()) != Some(end) {
        let message = viewer.next().expect("frames until the last step's end");
        if message["type"] == "frame" {
            frames.push(message);
        }
    }
    assert!(
        frames
            .iter()
            .any(|f| f["led_a"][0].as_f64().unwrap() > 0.005),
        "a frame shows led1 lit"
    );
    let seqs: Vec<u64> = frames.iter().map(|f| f["seq"].as_u64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[1] > w[0]), "{seqs:?}");
    runtime.orbe(watched.path(), &["close"]).ok();
    runtime.orbe(plain.path(), &["close"]).ok();
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the clock is the wall the session plays against"
)]
fn play_then_pause_advances_time_at_wall_speed() {
    let runtime = Runtime::new();
    let (dir, _page, url) = opened(&runtime);
    let mut viewer = Viewer::connect(&url);
    // From before `play` answers until after `pause` returns. Simulated time starts inside
    // that interval, when the session takes `play`, and stops when it takes `pause`.
    let started = Instant::now();
    let played = runtime.orbe(dir.path(), &["play"]).ok();
    assert_eq!(played.stdout, "playing from t=0s\n");
    let mut seen = Vec::new();
    let ticked = loop {
        let frame = viewer.until("frame", &mut seen);
        if frame["state"] == "playing" && frame["t_ps"] != 0 {
            break frame["t_ps"].as_u64().unwrap();
        }
    };
    std::thread::sleep(Duration::from_millis(300));
    let paused = runtime.orbe(dir.path(), &["pause", "--json"]).ok().json();
    let elapsed = started.elapsed();
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["data"]["playing"], false);
    let t = paused["t_ps"].as_u64().unwrap();
    // Slow motion is what a tick costs when the engine takes longer than the slice it
    // simulates, so the floor is only that time moved after the first frame. The ceiling is
    // the measured interval plus one tick. The loop applies a due tick before it reads
    // `pause`, and that tick can have started before this clock. A second tick of slack
    // would let a session playing at twice wall speed pass.
    let margin = orbe_session::viewer::TICK;
    let allowed_ps = u64::try_from(elapsed.saturating_add(margin).as_nanos())
        .unwrap()
        .saturating_mul(1000);
    assert!(
        t > ticked && t <= allowed_ps,
        "play advanced from {ticked} to {t} ps during {elapsed:?}; the margin is one tick"
    );

    while !seen
        .iter()
        .any(|m: &Value| m["type"] == "frame" && m["state"] == "paused")
    {
        viewer.until("frame", &mut seen);
    }
    let playing: Vec<&Value> = seen
        .iter()
        .filter(|m| m["state"] == "playing" && m["t_ps"] != 0)
        .collect();
    assert!(!playing.is_empty(), "frames from play's ticks");
    assert!(playing.iter().all(|f| f["speed"] == 1.0), "{playing:?}");
    assert!(playing.iter().all(|f| f["ratio"].as_f64().unwrap() > 0.0));

    let again = viewer.run(1, "play", None, &mut seen);
    assert_eq!(again["state"], "playing");
    let stepped = runtime
        .orbe(dir.path(), &["step", "1ms", "--json"])
        .ok()
        .json();
    assert_eq!(stepped["state"], "paused", "a step stops play");
    runtime.orbe(dir.path(), &["close"]).ok();
}

/// Presses `@btn1` for 500 ms, then steps 50 ms past its release, and returns `get @led1`.
fn pulse(runtime: &Runtime, dir: &Path) -> Value {
    for args in [
        &["press", "@btn1"][..],
        &["step", "500ms"],
        &["release", "@btn1"],
        &["step", "50ms"],
    ] {
        runtime.orbe(dir, args).ok();
    }
    runtime.orbe(dir, &["get", "@led1", "--json"]).ok().json()
}

fn assert_drawn_as_read(frame: &Value, got: &Value) {
    assert_eq!(frame["state"], "paused");
    assert_eq!(frame["t_ps"], got["t_ps"]);
    assert_eq!(
        frame["led_a"][0], got["data"]["value"]["i"],
        "the page draws @led1 at the current `get` reads: {frame}"
    );
}

#[test]
fn a_page_that_joins_late_draws_each_led_as_get_reads_it() {
    let runtime = Runtime::new();
    let (dir, _page, url) = opened(&runtime);
    let first = Viewer::connect(&url);
    first.leave();

    let got = pulse(&runtime, dir.path());
    assert_eq!(got["data"]["value"]["state"], "off");
    let mut late = Viewer::connect(&url);
    assert_drawn_as_read(&late.until("frame", &mut Vec::new()), &got);

    let got = pulse(&runtime, dir.path());
    let mut seen = Vec::new();
    while late.until("frame", &mut seen)["t_ps"] != got["t_ps"] {}
    let mut second = Viewer::connect(&url);
    assert_drawn_as_read(&second.until("frame", &mut Vec::new()), &got);
    runtime.orbe(dir.path(), &["close"]).ok();
}

#[test]
fn a_viewer_leaving_keeps_the_session_and_close_ends_the_rest() {
    let runtime = Runtime::new();
    let (dir, _page, url) = opened(&runtime);
    let first = Viewer::connect(&url);
    let mut second = Viewer::connect(&url);
    second.until("frame", &mut Vec::new());
    drop(first);
    runtime.orbe(dir.path(), &["step", "10ms"]).ok();
    let pid = runtime.daemon();

    runtime.orbe(dir.path(), &["close"]).ok();
    let mut seen = Vec::new();
    while let Some(message) = second.next() {
        seen.push(message);
    }
    assert_eq!(
        seen.last(),
        Some(&json!({"type": "closed", "reason": "`orbe close` ended the session"}))
    );
    wait_until("the session process ends", || gone(pid));
}

/// An `xdg-open` that records the URL it was given.
fn xdg_open(dir: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("bin");
    fs::create_dir(&bin).unwrap();
    let log = dir.join("xdg.log");
    let script = bin.join("xdg-open");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"$ORBE_XDG_LOG\"\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).unwrap();
    (bin, log)
}

#[test]
fn open_headed_opens_a_browser_only_when_no_viewer_is_connected() {
    let runtime = Runtime::new();
    let dir = project("open\n");
    let page = page();
    let tools = tempfile::tempdir().unwrap();
    let (bin, log) = xdg_open(tools.path());
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let headed = |args: &[&str]| {
        let output = runtime
            .command(dir.path(), args)
            .env("ORBE_VIEWER_DIR", page.path())
            .env("PATH", &path)
            .env("ORBE_XDG_LOG", &log)
            .output()
            .unwrap();
        Run::new(output)
    };

    let first = headed(&["open", "--headed"]);
    assert_eq!(first.code, 0, "{}{}", first.stdout, first.stderr);
    let url = first.stdout.lines().next_back().unwrap().to_string();
    assert!(url.starts_with("http://127.0.0.1:"), "{}", first.stdout);
    wait_until("xdg-open ran", || {
        fs::read_to_string(&log).is_ok_and(|text| text.trim() == url)
    });
    let opened = fs::read_to_string(&log).unwrap();

    let mut viewer = Viewer::connect(&url);
    viewer.until("scene", &mut Vec::new());
    let second = headed(&["open", "--headed"]);
    assert_eq!(second.code, 0, "{}{}", second.stdout, second.stderr);
    assert!(
        second.stdout.lines().any(|line| line == url),
        "{}",
        second.stdout
    );
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        opened,
        "a connected viewer opens no browser"
    );

    let shown = headed(&["show", "--no-browser"]).ok();
    assert!(shown.stdout.contains(&url), "{}", shown.stdout);
    assert_eq!(fs::read_to_string(&log).unwrap(), opened);
    viewer.leave();
    runtime.orbe(dir.path(), &["close"]).ok();
}
