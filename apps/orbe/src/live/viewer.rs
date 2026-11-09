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

use orbe_scene::Files;
use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role};
use tungstenite::{Message, WebSocket};

use super::daemon::{Event, wall};
use super::page::Page;

/// The longest request head the listener reads.
const HEAD: usize = 16 * 1024;
/// Headers in one request. A head past `HEAD`, or with more headers than this, is not a request.
const HEADERS: usize = 128;
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
    /// A command's text, as the viewer sent it.
    Command(u64, String),
    Left(u64),
}

/// The part models the page may load, by URL path, such as `/parts/led-5mm/led-5mm.glb`, each
/// with the project folder that reads it.
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
            let slot = Slot::take(&serving);
            let (site, events) = (Arc::clone(&site), events.clone());
            thread::spawn(move || {
                serve(stream, &site, &events);
                drop(slot);
            });
        }
    });
    Ok(url)
}

/// One of the listener's connections, given back when dropped: when its thread ends, panics,
/// or never starts.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(serving: &Arc<AtomicUsize>) -> Self {
        serving.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(serving))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
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

enum HeadEnd {
    /// The head is not complete.
    More,
    /// The bytes are not a request head.
    Bad,
    /// The head, and the index where the bytes after it start.
    Ready(Head, usize),
}

/// The request head in `bytes`, when they hold one. Bytes after the head stay unparsed.
fn take_head(bytes: &[u8]) -> HeadEnd {
    let mut slots = [httparse::EMPTY_HEADER; HEADERS];
    let mut request = httparse::Request::new(&mut slots);
    let end = match request.parse(bytes) {
        Ok(httparse::Status::Complete(end)) => end,
        Ok(httparse::Status::Partial) => return HeadEnd::More,
        Err(_) => return HeadEnd::Bad,
    };
    let (Some(method), Some(target)) = (request.method, request.path) else {
        return HeadEnd::Bad;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut headers = BTreeMap::new();
    for header in request.headers.iter() {
        let Ok(value) = str::from_utf8(header.value) else {
            return HeadEnd::Bad;
        };
        headers.insert(header.name.to_ascii_lowercase(), value.trim().to_string());
    }
    HeadEnd::Ready(
        Head {
            method: method.to_string(),
            path: path.to_string(),
            query: query.to_string(),
            headers,
        },
        end,
    )
}

/// Reads a request head. Bytes after it that arrived in the same read are returned with it,
/// so a WebSocket frame there is still read as a frame.
fn read_head(stream: &mut TcpStream) -> Option<(Head, Vec<u8>)> {
    let until = wall() + WAIT;
    let mut bytes = Vec::new();
    loop {
        match take_head(&bytes) {
            HeadEnd::Ready(head, end) => return Some((head, bytes.split_off(end))),
            HeadEnd::Bad => return None,
            HeadEnd::More => {}
        }
        let left = until.saturating_duration_since(wall());
        if left.is_zero() || bytes.len() >= HEAD {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        let start = bytes.len();
        bytes.resize(HEAD, 0);
        let read = stream.read(&mut bytes[start..]).ok()?;
        bytes.truncate(start + read);
        if read == 0 {
            return None;
        }
    }
}

fn serve(mut stream: TcpStream, site: &Site, events: &Sender<Event>) {
    let Some((head, rest)) = read_head(&mut stream) else {
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
            derive_accept_key(key.as_bytes())
        );
        if stream.write_all(accepted.as_bytes()).is_ok() {
            watch(stream, rest, events);
        }
        return;
    }
    let only_head = head.method == "HEAD";
    if head.method != "GET" && !only_head {
        return respond(&mut stream, 405, "text/plain", b"GET only", false);
    }
    let body = if head.path.starts_with("/parts/") {
        let root = site.models.lock().unwrap().get(&head.path).cloned();
        let (id, rel) = head.path["/parts/".len()..]
            .split_once('/')
            .unwrap_or_default();
        root.and_then(|root| orbe_scene::part_file(&Files::disk(), &root, id, rel))
            .map(Cow::Owned)
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
fn watch(stream: TcpStream, rest: Vec<u8>, events: &Sender<Event>) {
    if stream.set_read_timeout(Some(POLL)).is_err() {
        return;
    }
    let mut socket = WebSocket::from_partially_read(stream, rest, Role::Server, None);
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
                let event = Event::Viewer(Viewer::Command(id, text.to_string()));
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

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    use super::*;

    /// A masked client text frame. The server refuses an unmasked one.
    fn client_text(text: &str) -> Vec<u8> {
        let mask = [0x11, 0x22, 0x33, 0x44];
        let mut frame = vec![0x81, 0x80 | u8::try_from(text.len()).unwrap()];
        frame.extend(mask);
        for (i, byte) in text.bytes().enumerate() {
            frame.push(byte ^ mask[i % 4]);
        }
        frame
    }

    #[test]
    fn a_head_keeps_the_frame_that_shares_its_buffer() {
        let frame = client_text("hi");
        let mut bytes = b"GET /session?t=abc HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n".to_vec();
        bytes.extend_from_slice(&frame);
        let HeadEnd::Ready(head, end) = take_head(&bytes) else {
            panic!("a complete head");
        };
        assert_eq!(head.path, "/session");
        assert_eq!(head.query, "t=abc");
        assert_eq!(head.header("host"), Some("127.0.0.1:9"));
        assert_eq!(&bytes[end..], frame.as_slice());
    }

    #[test]
    fn a_request_that_is_not_http_gets_no_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let host = format!("127.0.0.1:{port}");
        let mut client = TcpStream::connect(&host).unwrap();
        client.write_all(b"nope\r\n\r\n").unwrap();
        let (server, _) = listener.accept().unwrap();
        let (events, _) = mpsc::channel();
        let site = Arc::new(Site {
            page: Page::Built(&[]),
            models: Arc::new(Mutex::new(BTreeMap::new())),
            host,
            origin: "http://127.0.0.1".to_string(),
            token: "abc".to_string(),
        });
        let served = thread::spawn(move || serve(server, &site, &events));
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut buf = [0; 8];
        assert_eq!(client.read(&mut buf).unwrap(), 0);
        served.join().unwrap();
    }

    /// The upgrade and the first frame share one write. The session still receives the text.
    #[test]
    fn a_frame_in_the_same_write_as_the_head_is_a_command() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let host = format!("127.0.0.1:{port}");
        let token = "abc";
        let mut bytes = format!(
            "GET /session?t={token} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\n\
             Upgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .into_bytes();
        bytes.extend(client_text("hi"));

        let mut client = TcpStream::connect(&host).unwrap();
        client.write_all(&bytes).unwrap();
        let (server, _) = listener.accept().unwrap();
        let (events, inbox) = mpsc::channel();
        let site = Arc::new(Site {
            page: Page::Built(&[]),
            models: Arc::new(Mutex::new(BTreeMap::new())),
            host: host.clone(),
            origin: format!("http://{host}"),
            token: token.to_string(),
        });
        let served = thread::spawn(move || serve(server, &site, &events));

        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut response = Vec::new();
        let mut byte = [0];
        while !response.ends_with(b"\r\n\r\n") {
            client.read_exact(&mut byte).unwrap();
            response.push(byte[0]);
        }
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 101"), "{response}");
        assert!(
            response.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
            "{response}"
        );

        let mut command = None;
        let until = wall() + Duration::from_secs(2);
        while wall() < until {
            if let Ok(Event::Viewer(Viewer::Command(_, text))) =
                inbox.recv_timeout(Duration::from_millis(50))
            {
                command = Some(text);
                break;
            }
        }
        drop(client);
        served.join().unwrap();
        assert_eq!(command.as_deref(), Some("hi"));
    }
}
