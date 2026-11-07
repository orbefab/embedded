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
use std::path::Path;
use std::time::Duration;

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
    "step --until @led1 on --timeout 50ms",
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
fn play_then_pause_advances_time_at_wall_speed() {
    let runtime = Runtime::new();
    let (dir, _page, url) = opened(&runtime);
    let mut viewer = Viewer::connect(&url);
    let played = runtime.orbe(dir.path(), &["play"]).ok();
    assert_eq!(played.stdout, "playing from t=0s\n");
    std::thread::sleep(Duration::from_millis(300));
    let paused = runtime.orbe(dir.path(), &["pause", "--json"]).ok().json();
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["data"]["playing"], false);
    let t = paused["t_ps"].as_u64().unwrap();
    assert!(
        (100_000_000_000..=1_000_000_000_000).contains(&t),
        "300 ms of wall time plays about 300 ms: {t} ps"
    );

    let mut seen = Vec::new();
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
