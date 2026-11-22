//! `orbe add` against a registry built from this commit. Nothing here calls orbefab.com.

#[path = "registry_env.rs"]
mod registry_env;

use std::collections::BTreeSet;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};

struct Close<'a>(&'a Path);

impl Drop for Close<'_> {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_orbe"))
            .arg("close")
            .current_dir(self.0)
            .output();
    }
}

fn run(dir: &Path, args: &[&str], registry: &str) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(args)
        .current_dir(dir)
        .env("ORBE_REGISTRY", registry)
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(1),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn file_template(dir: &Path) -> String {
    format!("file://{}/{{name}}.json", dir.display())
}

/// A copy of the built index, with `led-5mm`'s row changed, and that part's record beside it.
fn led_index(edit: impl Fn(&mut Value)) -> (tempfile::TempDir, String) {
    let src = registry_env::dir();
    let dir = tempfile::tempdir().unwrap();
    let index: Value = serde_json::from_slice(&fs::read(src.join("index.json")).unwrap()).unwrap();
    let row = index["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "led-5mm")
        .expect("led-5mm");
    let url = row["url"].as_str().unwrap().to_string();
    fs::copy(src.join(&url), dir.path().join(&url)).unwrap();
    let mut index = index;
    let row = index["items"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["name"] == "led-5mm")
        .unwrap();
    edit(row);
    fs::write(
        dir.path().join("index.json"),
        serde_json::to_vec_pretty(&index).unwrap(),
    )
    .unwrap();
    let record = format!("file://{}/{url}", dir.path().display());
    (dir, record)
}

struct Server {
    port: u16,
    stop: Arc<Mutex<bool>>,
    redirects: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn bind(root: PathBuf, mode: Mode) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&stop);
        let (tx, redirects) = mpsc::channel();
        let thread = thread::spawn(move || serve(listener, root, mode, flag, tx));
        Server {
            port,
            stop,
            redirects,
            thread: Some(thread),
        }
    }

    fn template(&self) -> String {
        format!("http://127.0.0.1:{}/{{name}}.json", self.port)
    }

    fn redirect_count(&self) -> usize {
        let mut count = 0;
        while self.redirects.try_recv().is_ok() {
            count += 1;
        }
        count
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        *self.stop.lock().unwrap() = true;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Mode {
    /// Every request is this status, with an empty body.
    Status(u16),
    /// The first request for a path that contains `@` is a 307 to the same path with `@`
    /// written `%40`. The next request for it is the file.
    RedirectAt,
}

fn serve(
    listener: TcpListener,
    root: PathBuf,
    mode: Mode,
    stop: Arc<Mutex<bool>>,
    redirects: Sender<()>,
) {
    let mut once = BTreeSet::new();
    while !*stop.lock().unwrap() {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let _ = handle(&mut stream, &root, &mode, &mut once, &redirects);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
}

fn handle(
    stream: &mut TcpStream,
    root: &Path,
    mode: &Mode,
    once: &mut BTreeSet<String>,
    redirects: &Sender<()>,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while buf.len() < 16 * 1024 {
        if stream.read(&mut byte)? == 0 {
            break;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let path = text
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/");
    if let Mode::Status(code) = mode {
        return write_response(stream, &format!("{code} no"), "", b"");
    }
    let decoded = percent_decode(path);
    if matches!(mode, Mode::RedirectAt) && decoded.contains('@') && once.insert(decoded.clone()) {
        let location = decoded.trim_start_matches('/').replace('@', "%40");
        let _ = redirects.send(());
        return write_response(
            stream,
            "307 Temporary Redirect",
            &format!("Location: {location}\r\n"),
            b"",
        );
    }
    let rel = decoded.trim_start_matches('/');
    if rel.is_empty() || rel.split('/').any(|segment| segment == "..") {
        return write_response(stream, "404 Not Found", "", b"no");
    }
    match fs::read(root.join(rel)) {
        Ok(bytes) => write_response(stream, "200 OK", "", &bytes),
        Err(_) => write_response(stream, "404 Not Found", "", b"no"),
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    extra: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn a_part_install_follows_an_at_redirect_and_writes_the_copy() {
    let server = Server::bind(registry_env::dir().to_path_buf(), Mode::RedirectAt);
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(dir.path(), &["add", "led-5mm"], &server.template());
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert_eq!(stdout, "wrote parts/led-5mm 0.1.1\n");
    assert!(dir.path().join("parts/led-5mm/part.jsonc").is_file());
    assert_eq!(server.redirect_count(), 1, "the record url holds @");
}

#[test]
fn a_board_nests_its_chip_and_can_be_added_again_beside_that_chip() {
    let registry = registry_env::template();
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(dir.path(), &["add", "atmega328p-tqfp32"], &registry);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        dir.path()
            .join("parts/atmega328p-tqfp32/part.jsonc")
            .is_file()
    );
    let (code, stdout, stderr) = run(dir.path(), &["add", "nano-board"], &registry);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("wrote parts/nano-board "), "{stdout}");
    assert!(
        dir.path()
            .join("parts/nano-board/parts/atmega328p-tqfp32/part.jsonc")
            .is_file()
    );
    fs::remove_dir_all(dir.path().join("parts/nano-board")).unwrap();
    let (code, stdout, stderr) = run(dir.path(), &["add", "nano-board"], &registry);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        dir.path()
            .join("parts/nano-board/parts/atmega328p-tqfp32/part.jsonc")
            .is_file()
    );
    assert!(
        dir.path()
            .join("parts/atmega328p-tqfp32/part.jsonc")
            .is_file()
    );
}

#[test]
fn an_interrupted_block_names_the_block_and_the_folders_to_remove() {
    let dir = tempfile::tempdir().unwrap();
    for id in [
        "breadboard-400",
        "led-5mm",
        "nano-board",
        "resistor-axial",
        "tactile-switch-6mm",
    ] {
        fs::create_dir_all(dir.path().join("parts").join(id)).unwrap();
    }
    let (code, stdout, stderr) = run(
        dir.path(),
        &["add", "nano-button-led"],
        &registry_env::template(),
    );
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("error[part_exists]:"), "{stderr}");
    for id in [
        "led-5mm",
        "nano-board",
        "resistor-axial",
        "tactile-switch-6mm",
    ] {
        assert!(
            stderr.contains(&format!("parts/{id} is already there\n")),
            "{stderr}"
        );
    }
    assert!(
        stderr.contains("hint: remove them and run `orbe add nano-button-led`\n"),
        "{stderr}"
    );
    assert!(!stderr.contains("orbe add led-5mm"), "{stderr}");
    assert!(!dir.path().join("scene.jsonc").exists());
}

#[test]
fn a_record_whose_size_or_hash_disagrees_names_the_url() {
    let (dir, url) = led_index(|row| {
        let bytes = row["bytes"].as_u64().unwrap();
        row["bytes"] = json!(bytes + 10);
    });
    let (code, stdout, stderr) = run(dir.path(), &["add", "led-5mm"], &file_template(dir.path()));
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!("{url} is ")) && stderr.contains("bytes, and its index row"),
        "{stderr}"
    );
    assert!(
        stderr.contains("hint: the file does not match its record\n"),
        "{stderr}"
    );
    assert!(!dir.path().join("parts").exists());

    let (dir, url) = led_index(|row| {
        let sha = row["sha256"].as_str().unwrap();
        let flipped = if sha.ends_with('0') { "1" } else { "0" };
        row["sha256"] = json!(format!("{}{flipped}", &sha[..sha.len() - 1]));
    });
    let (code, stdout, stderr) = run(dir.path(), &["add", "led-5mm"], &file_template(dir.path()));
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!("{url} SHA-256 is ")) && stderr.contains("and its index row"),
        "{stderr}"
    );
    assert!(
        stderr.contains("hint: the file does not match its record\n"),
        "{stderr}"
    );
}

#[test]
fn a_registry_that_returns_an_error_names_the_url() {
    let server = Server::bind(PathBuf::new(), Mode::Status(404));
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(dir.path(), &["add", "led-5mm"], &server.template());
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!(
            "error[registry]: http://127.0.0.1:{}/index.json returned HTTP 404\n",
            server.port
        )),
        "{stderr}"
    );
    assert!(!stderr.contains("hint:"), "{stderr}");
}

#[test]
fn an_unreachable_registry_names_the_index_url() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(
        dir.path(),
        &["add", "led-5mm"],
        "http://127.0.0.1:1/{name}.json",
    );
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains(
            "error[registry]: http://127.0.0.1:1/index.json could not be reached\n  hint: check the network and run the command again\n"
        ),
        "{stderr}"
    );
}

#[test]
fn check_open_and_test_do_not_use_the_network() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(
        dir.path(),
        &["add", "nano-button-led"],
        &registry_env::template(),
    );
    assert_eq!(code, 0, "{stdout}{stderr}");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let registry = format!("http://127.0.0.1:{port}/{{name}}.json");
    let _close = Close(dir.path());
    let (code, stdout, stderr) = run(dir.path(), &["check", "scene.jsonc"], &registry);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let (code, stdout, stderr) = run(dir.path(), &["open"], &registry);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.starts_with("opened "), "{stdout}");
    let (code, stdout, stderr) = run(dir.path(), &["test"], &registry);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("1 passed, 0 failed"), "{stdout}");
    let (code, _, _) = run(dir.path(), &["close"], &registry);
    assert_eq!(code, 0);
    match listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        other => panic!("check, open or test connected: {other:?}"),
    }
}
