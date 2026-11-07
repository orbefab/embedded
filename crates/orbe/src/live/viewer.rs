//! The session process's viewer listener, which the first `show` starts on 127.0.0.1 at a
//! random port. It serves the page and the scene's part models over HTTP, and each viewer's
//! WebSocket at `/session`. The page holds no secret, so a `GET` checks only Host. The upgrade
//! checks the token `show` made, Host and Origin, once, and is refused when any differs.
//! Nothing here logs a request line, so the token stays out of the session's log.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use orbe_session::{Code, Command, Error};
use serde_json::Value;
use sha1::{Digest, Sha1};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role};
use tungstenite::{Message, WebSocket};

use super::daemon::{Event, wall};
use super::page::Page;

/// The longest request head the listener reads.
const HEAD: usize = 16 * 1024;
/// How long the listener waits for a whole request head, and for a viewer to take what it is
/// sent.
const WAIT: Duration = Duration::from_secs(5);
/// The most connections the listener serves at once, page requests and viewers together. It
/// closes any past these at once.
const CONNECTIONS: usize = 32;
/// `accept` fails again at once while the process is out of files, so the listener waits this
/// long instead of spinning.
const BACKOFF: Duration = Duration::from_millis(100);
/// How long a viewer's thread waits for a command before it sends what the session queued.
const POLL: Duration = Duration::from_millis(5);
/// RFC 6455's key suffix for `Sec-WebSocket-Accept`.
const ACCEPT: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// What the session sends one viewer.
pub(super) enum Out {
    /// A scene message or a command's reply, sent in order.
    Text(Arc<str>),
    /// A frame. Of the frames waiting, only the latest is sent.
    Frame(Arc<str>),
    /// The last message, then the close. `done` hears once both are sent.
    Close(Arc<str>, Sender<()>),
}

/// What a viewer's thread tells the session.
pub(super) enum Viewer {
    Joined(u64, Sender<Out>),
    /// A command, with the id its reply carries back.
    Command(u64, Value, Result<Command, Error>),
    Left(u64),
}

/// The part models the page may load, by URL path, such as `/parts/led-5mm/led-5mm.glb`.
pub(super) type Models = Arc<Mutex<BTreeMap<String, PathBuf>>>;

struct Site {
    page: Page,
    models: Models,
    host: String,
    origin: String,
    token: String,
}

/// Listens, and returns the page's URL with the token in its fragment.
pub(super) fn start(page: Page, models: Models, events: Sender<Event>) -> io::Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let token = token()?;
    let url = format!("http://127.0.0.1:{port}/#t={token}");
    let site = Arc::new(Site {
        page,
        models,
        host: format!("127.0.0.1:{port}"),
        origin: format!("http://127.0.0.1:{port}"),
        token,
    });
    thread::spawn(move || {
        let serving = Arc::new(AtomicUsize::new(0));
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                thread::sleep(BACKOFF);
                continue;
            };
            if serving.load(Ordering::SeqCst) >= CONNECTIONS {
                continue;
            }
            serving.fetch_add(1, Ordering::SeqCst);
            let (site, events, serving) = (Arc::clone(&site), events.clone(), Arc::clone(&serving));
            thread::spawn(move || {
                serve(stream, &site, &events);
                serving.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(url)
}

fn token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

struct Head {
    method: String,
    path: String,
    query: String,
    /// By lowercase name.
    headers: BTreeMap<String, String>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Reads up to the blank line that ends the head, a byte at a time, so nothing past it is
/// taken from a stream that becomes a WebSocket.
fn read_head(stream: &mut TcpStream) -> Option<Head> {
    let until = wall() + WAIT;
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        let left = until.saturating_duration_since(wall());
        if left.is_zero() || bytes.len() >= HEAD {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        bytes.push(byte[0]);
    }
    let text = String::from_utf8(bytes).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?, first.next()?);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    Some(Head {
        method: method.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        headers,
    })
}

fn serve(mut stream: TcpStream, site: &Site, events: &Sender<Event>) {
    let Some(head) = read_head(&mut stream) else {
        return;
    };
    if stream.set_write_timeout(Some(WAIT)).is_err() {
        return;
    }
    if head.header("host") != Some(site.host.as_str()) {
        return respond(&mut stream, 403, "text/plain", b"forbidden", false);
    }
    if head.path == "/session" {
        let Some(key) = upgrade_key(&head, site) else {
            return respond(&mut stream, 403, "text/plain", b"forbidden", false);
        };
        let accepted = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            accept(key)
        );
        if stream.write_all(accepted.as_bytes()).is_ok() {
            watch(stream, events);
        }
        return;
    }
    let only_head = head.method == "HEAD";
    if head.method != "GET" && !only_head {
        return respond(&mut stream, 405, "text/plain", b"GET only", false);
    }
    let body = if head.path.starts_with("/parts/") {
        let file = site.models.lock().unwrap().get(&head.path).cloned();
        file.and_then(|f| std::fs::read(f).ok()).map(Cow::Owned)
    } else {
        site.page.get(&head.path)
    };
    match body {
        Some(body) => respond(&mut stream, 200, content_type(&head.path), &body, only_head),
        None => respond(&mut stream, 404, "text/plain", b"not found", only_head),
    }
}

/// The client's `Sec-WebSocket-Key` when the upgrade carries the token, this listener's Host
/// and the page's Origin.
fn upgrade_key<'h>(head: &'h Head, site: &Site) -> Option<&'h str> {
    let token = head
        .query
        .split('&')
        .find_map(|pair| pair.strip_prefix("t="))?;
    let allowed = head.method == "GET"
        && same(token.as_bytes(), site.token.as_bytes())
        && head.header("origin") == Some(site.origin.as_str())
        && head
            .header("upgrade")
            .is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
        && head.header("sec-websocket-version") == Some("13");
    allowed.then(|| head.header("sec-websocket-key")).flatten()
}

/// Compares in time that does not depend on where the bytes differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn accept(key: &str) -> String {
    base64(&Sha1::digest(format!("{key}{ACCEPT}")))
}

fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(LETTERS[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn content_type(path: &str) -> &'static str {
    let name = Path::new(path);
    match name.extension().and_then(|e| e.to_str()) {
        None => "text/html; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("glb") => "model/gltf-binary",
        Some("wasm") => "application/wasm",
        Some(_) => "application/octet-stream",
    }
}

fn respond(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8], only_head: bool) {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    if !only_head {
        let _ = stream.write_all(body);
    }
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// One viewer's WebSocket: what the session queues goes out, and each command goes to the
/// session's queue, until either side closes.
fn watch(stream: TcpStream, events: &Sender<Event>) {
    if stream.set_read_timeout(Some(POLL)).is_err() {
        return;
    }
    let mut socket = WebSocket::from_raw_socket(stream, Role::Server, None);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (out, queue) = mpsc::channel();
    if events.send(Event::Viewer(Viewer::Joined(id, out))).is_ok() {
        let _ = relay(&mut socket, id, &queue, events);
    }
    let _ = events.send(Event::Viewer(Viewer::Left(id)));
}

fn relay(
    socket: &mut WebSocket<TcpStream>,
    id: u64,
    queue: &Receiver<Out>,
    events: &Sender<Event>,
) -> tungstenite::Result<()> {
    loop {
        let mut frame = None;
        for out in queue.try_iter() {
            match out {
                Out::Text(text) => socket.write(Message::text(&*text))?,
                Out::Frame(text) => frame = Some(text),
                Out::Close(last, done) => {
                    if let Some(frame) = frame.take() {
                        socket.write(Message::text(&*frame))?;
                    }
                    socket.write(Message::text(&*last))?;
                    socket.close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "the session ended".into(),
                    }))?;
                    let flushed = socket.flush();
                    let _ = done.send(());
                    return flushed;
                }
            }
        }
        if let Some(frame) = frame {
            socket.write(Message::text(&*frame))?;
        }
        socket.flush()?;
        match socket.read() {
            Ok(Message::Text(text)) => {
                let (request, command) = command(&text);
                let event = Event::Viewer(Viewer::Command(id, request, command));
                if events.send(event).is_err() {
                    return Ok(());
                }
            }
            Ok(Message::Close(_)) => {
                while socket.flush().is_ok() {}
                return Ok(());
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(error),
        }
    }
}

/// A viewer's command, `{"id": 1, "cmd": "press", "ref": "@btn1"}`, parsed by the CLI's own
/// table, as `orbe press @btn1` would be. A viewer sends only `press`, `release`, `play` and
/// `pause`.
fn command(text: &str) -> (Value, Result<Command, Error>) {
    let usage = |message: String| {
        Error::new(
            Code::Usage,
            message,
            "a viewer sends press, release, play or pause, as {\"id\": 1, \"cmd\": \"press\", \"ref\": \"@btn1\"}",
        )
    };
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return (
            Value::Null,
            Err(usage("a viewer's command is not JSON".into())),
        );
    };
    let id = message["id"].clone();
    let name = message["cmd"].as_str().unwrap_or_default();
    let parsed = match orbe_session::spec(name) {
        Some(spec) if ["press", "release", "play", "pause"].contains(&name) => {
            let mut args: Vec<String> = Vec::new();
            args.extend(message["ref"].as_str().map(String::from));
            args.extend(message.get("speed").map(Value::to_string));
            spec.command(&args, Path::new(""))
                .map_err(|reason| Error::new(Code::Usage, reason, ""))
        }
        _ => Err(usage(format!("`{name}` is not a command a viewer sends"))),
    };
    (id, parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accept_key_is_rfc_6455_s() {
        assert_eq!(
            accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
    }

    #[test]
    fn a_viewer_s_command_parses_as_the_cli_s() {
        let (id, press) = command(r#"{"id": 7, "cmd": "press", "ref": "@btn1"}"#);
        assert_eq!(id, 7);
        assert_eq!(
            press.unwrap(),
            Command::parse("press @btn1", Path::new(""))
                .unwrap()
                .unwrap()
        );
        let (_, play) = command(r#"{"id": 8, "cmd": "play"}"#);
        assert_eq!(play.unwrap(), Command::Play);
        let (_, step) = command(r#"{"id": 9, "cmd": "step", "ref": "1ms"}"#);
        assert_eq!(
            step.unwrap_err().message,
            "`step` is not a command a viewer sends"
        );
        let (_, unnamed) = command(r#"{"id": 10, "cmd": "press", "ref": "btn1"}"#);
        assert_eq!(unnamed.unwrap_err().code, Code::Usage);
        let (_, fast) = command(r#"{"id": 11, "cmd": "play", "speed": 2}"#);
        assert_eq!(fast.unwrap_err().message, "takes no arguments; usage: play");
    }
}
