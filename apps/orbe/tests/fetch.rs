//! `orbe add` against a registry built from this commit. Nothing here calls orbefab.com.

#[path = "registry_env.rs"]
mod registry_env;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
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
    run_env(dir, args, registry, &[])
}

fn run_env(
    dir: &Path,
    args: &[&str],
    registry: &str,
    env: &[(&str, &str)],
) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orbe"));
    command
        .args(args)
        .current_dir(dir)
        .env("ORBE_REGISTRY", registry);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().unwrap();
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
    cert: PathBuf,
    count: PathBuf,
    child: Child,
    _dir: tempfile::TempDir,
}

impl Server {
    fn bind(root: PathBuf, mode: Mode) -> Server {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        let made = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-keyout",
                key.to_str().unwrap(),
                "-out",
                cert.to_str().unwrap(),
                "-days",
                "1",
                "-nodes",
                "-subj",
                "/CN=127.0.0.1",
                "-addext",
                "subjectAltName=IP:127.0.0.1",
                "-addext",
                "basicConstraints=CA:TRUE",
            ])
            .output()
            .unwrap();
        assert!(
            made.status.success(),
            "openssl: {}",
            String::from_utf8_lossy(&made.stderr)
        );
        let script = dir.path().join("serve.py");
        fs::write(&script, SERVE_PY).unwrap();
        let port_file = dir.path().join("port");
        let count = dir.path().join("redirects");
        let log = File::create(dir.path().join("log")).unwrap();
        let mode_arg = match mode {
            Mode::Status(code) => code.to_string(),
            Mode::RedirectAt => "redirect".to_string(),
            Mode::Observe => "observe".to_string(),
        };
        let child = Command::new("python3")
            .arg(&script)
            .arg(&port_file)
            .arg(&cert)
            .arg(&key)
            .arg(&root)
            .arg(&mode_arg)
            .arg(&count)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let mut server = Server {
            port: 0,
            cert,
            count,
            child,
            _dir: dir,
        };
        server.port = wait_port(&port_file, server._dir.path().join("log"));
        server
    }

    fn template(&self) -> String {
        format!("https://127.0.0.1:{}/{{name}}.json", self.port)
    }

    fn trust(&self) -> Vec<(&str, String)> {
        let cert = self.cert.display().to_string();
        vec![("SSL_CERT_FILE", cert.clone()), ("CURL_CA_BUNDLE", cert)]
    }

    fn redirect_count(&self) -> usize {
        fs::read_to_string(&self.count)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .count()
    }

    fn paths(&self) -> Vec<String> {
        fs::read_to_string(&self.count)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn clear_paths(&self) {
        fs::write(&self.count, "").unwrap();
    }

    fn max_inflight(&self) -> usize {
        fs::read_to_string(self.count.with_extension("max"))
            .unwrap_or_default()
            .trim()
            .parse()
            .unwrap_or(0)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_port(port_file: &Path, log: PathBuf) -> u16 {
    for _ in 0..100 {
        if let Ok(text) = fs::read_to_string(port_file)
            && let Ok(port) = text.trim().parse()
        {
            return port;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "https server wrote no port: {}",
        fs::read_to_string(log).unwrap_or_default()
    );
}

enum Mode {
    /// Every request is this status, with an empty body.
    Status(u16),
    /// The first request for a path that contains `@` is a 307 to the same path with `@`
    /// written `%40`. The next request for it is the file.
    RedirectAt,
    /// Serve the registry. Each path is logged, and overlapping requests are counted.
    Observe,
}

const SERVE_PY: &str = r#"
import os
import socketserver
import ssl
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import unquote

port_file, cert, key, root, mode, count_file = sys.argv[1:7]
seen = set()
lock = threading.Lock()
inflight = 0
max_inflight = 0

class ThreadingServer(socketserver.ThreadingMixIn, HTTPServer):
    daemon_threads = True

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if mode == "observe":
            self.observe(path)
            return
        if mode != "redirect":
            self.send_response(int(mode))
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        decoded = unquote(path)
        if "@" in decoded and decoded not in seen:
            seen.add(decoded)
            location = decoded.lstrip("/").replace("@", "%40")
            with open(count_file, "a", encoding="utf-8") as handle:
                handle.write("1\n")
            self.send_response(307)
            self.send_header("Location", location)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        rel = decoded.lstrip("/")
        if not rel or any(part == ".." for part in rel.split("/")):
            body = b"no"
            self.send_response(404)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        try:
            with open(os.path.join(root, rel), "rb") as handle:
                body = handle.read()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except OSError:
            body = b"no"
            self.send_response(404)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    def observe(self, path):
        global inflight, max_inflight
        with lock:
            inflight += 1
            if inflight > max_inflight:
                max_inflight = inflight
            with open(count_file, "a", encoding="utf-8") as handle:
                handle.write(path + "\n")
            with open(count_file + ".max", "w", encoding="utf-8") as handle:
                handle.write(str(max_inflight) + "\n")
        try:
            time.sleep(0.03)
            self.serve_file(path)
        finally:
            with lock:
                inflight -= 1

    def serve_file(self, path):
        decoded = unquote(path)
        rel = decoded.lstrip("/")
        if not rel or any(part == ".." for part in rel.split("/")):
            body = b"no"
            code = 404
        else:
            try:
                with open(os.path.join(root, rel), "rb") as handle:
                    body = handle.read()
                code = 200
            except OSError:
                body = b"no"
                code = 404
        self.send_response(code)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        return

context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(cert, key)
if mode == "observe":
    server = ThreadingServer(("127.0.0.1", 0), Handler)
else:
    server = HTTPServer(("127.0.0.1", 0), Handler)
server.socket = context.wrap_socket(server.socket, server_side=True)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
"#;

#[test]
fn a_part_install_follows_an_at_redirect_and_writes_the_copy() {
    let server = Server::bind(registry_env::dir().to_path_buf(), Mode::RedirectAt);
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let cache_home = cache.path().display().to_string();
    let trust = server.trust();
    let mut env: Vec<(&str, &str)> = trust
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    env.push(("XDG_CACHE_HOME", cache_home.as_str()));
    let (code, stdout, stderr) = run_env(dir.path(), &["add", "led-5mm"], &server.template(), &env);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert_eq!(stdout, "wrote parts/led-5mm 0.3.0\n");
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
    let trust = server.trust();
    let env: Vec<(&str, &str)> = trust
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    let (code, stdout, stderr) = run_env(dir.path(), &["add", "led-5mm"], &server.template(), &env);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!(
            "error[registry]: https://127.0.0.1:{}/index.json returned HTTP 404\n",
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
        "https://127.0.0.1:1/{name}.json",
    );
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains(
            "error[registry]: https://127.0.0.1:1/index.json could not be reached\n  hint: check the network and run the command again\n"
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

#[test]
fn a_missing_curl_says_orbe_add_needs_it() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-bin");
    fs::create_dir(&missing).unwrap();
    let (code, stdout, stderr) = run_env(
        dir.path(),
        &["add", "led-5mm"],
        "https://127.0.0.1:1/{name}.json",
        &[("PATH", missing.to_str().unwrap())],
    );
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains("could not be fetched, because curl is not on PATH"),
        "{stderr}"
    );
    assert!(stderr.contains("hint: `orbe add` needs curl\n"), "{stderr}");
}

#[test]
fn a_named_part_keeps_its_latest_when_a_board_uses_an_older_one() {
    let (_tmp, reg) = cloned_registry();
    let old = led_version_used_by_nano(&reg);
    publish_led_0603(&reg, &old, "9.0.0");
    let template = format!("file://{}/{{name}}.json", reg.display());
    for order in [["nano-board", "led-0603"], ["led-0603", "nano-board"]] {
        let project = tempfile::tempdir().unwrap();
        let (code, stdout, stderr) = run(project.path(), &["add", order[0], order[1]], &template);
        assert_eq!(code, 0, "{order:?}\n{stdout}{stderr}");
        assert!(
            stdout.contains("wrote parts/led-0603 9.0.0\n"),
            "{order:?}\n{stdout}"
        );
        assert!(
            stdout.contains("wrote parts/nano-board "),
            "{order:?}\n{stdout}"
        );
        assert_eq!(
            declared_version(&project.path().join("parts/led-0603/part.jsonc")),
            "9.0.0",
            "{order:?}"
        );
        assert_eq!(
            declared_version(
                &project
                    .path()
                    .join("parts/nano-board/parts/led-0603/part.jsonc")
            ),
            old,
            "{order:?}"
        );
    }
}

#[test]
fn a_part_can_hold_two_versions_of_one_grandchild() {
    let (_tmp, reg) = cloned_registry();
    let index: Value = serde_json::from_slice(&fs::read(reg.join("index.json")).unwrap()).unwrap();
    let names: Vec<&str> = index["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"led-holder"), "{names:?}");
    assert!(!names.contains(&"two-leds"), "{names:?}");
    let old = led_version_used_by_nano(&reg);
    publish_led_0603(&reg, &old, "9.0.0");
    let nano = highest_version(&reg, "nano-board");
    add_part_from_led(&reg, "led-holder", "0.1.0", &["led-0603@9.0.0".to_string()]);
    add_part_from_led(
        &reg,
        "two-leds",
        "0.1.0",
        &[format!("nano-board@{nano}"), "led-holder@0.1.0".to_string()],
    );
    let template = format!("file://{}/{{name}}.json", reg.display());
    let project = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(project.path(), &["add", "two-leds"], &template);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("wrote parts/two-leds 0.1.0\n"), "{stdout}");
    assert_eq!(
        declared_version(
            &project
                .path()
                .join("parts/two-leds/parts/nano-board/parts/led-0603/part.jsonc")
        ),
        old
    );
    assert_eq!(
        declared_version(
            &project
                .path()
                .join("parts/two-leds/parts/led-holder/parts/led-0603/part.jsonc")
        ),
        "9.0.0"
    );
}

#[test]
fn fetched_files_match_the_registry_and_a_cached_copy_is_reused() {
    let registry = registry_env::dir();
    let server = Server::bind(registry.to_path_buf(), Mode::Observe);
    let cache = tempfile::tempdir().unwrap();
    let cache_home = cache.path().display().to_string();
    let trust = server.trust();
    let mut env: Vec<(&str, &str)> = trust
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    env.push(("XDG_CACHE_HOME", cache_home.as_str()));

    let first = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run_env(
        first.path(),
        &["add", "nano-board"],
        &server.template(),
        &env,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        server.max_inflight() >= 2,
        "in flight {}",
        server.max_inflight()
    );
    let paths = server.paths();
    let mut once = BTreeSet::new();
    for path in &paths {
        assert!(once.insert(path.clone()), "fetched twice: {path}");
    }
    assert!(paths.iter().any(|path| path.ends_with("/index.json")));
    let files: Vec<&String> = paths
        .iter()
        .filter(|path| path.contains("/files/"))
        .collect();
    assert!(files.len() >= 2, "{paths:?}");
    assert_parts_match(first.path(), registry);

    server.clear_paths();
    let second = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run_env(
        second.path(),
        &["add", "nano-board"],
        &server.template(),
        &env,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let again = server.paths();
    assert_eq!(again.len(), 1, "{again:?}");
    assert!(again[0].ends_with("/index.json"), "{again:?}");
    assert_parts_match(second.path(), registry);

    let part_sha = file_sha(registry, "nano-board", "part.jsonc");
    server.clear_paths();
    let (code, stdout, stderr) = run_env(
        second.path(),
        &["part", "nano-board"],
        &server.template(),
        &env,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let part_paths = server.paths();
    assert!(
        part_paths.iter().any(|path| path.ends_with("/index.json")),
        "{part_paths:?}"
    );
    assert!(
        part_paths.iter().all(|path| !path.contains(&part_sha)),
        "{part_paths:?}"
    );

    let sha = files[0].rsplit('/').next().unwrap();
    let kept = files[1].rsplit('/').next().unwrap();
    let cached = cache.path().join("orbe").join("files").join(sha);
    assert!(cached.is_file(), "{}", cached.display());
    fs::write(&cached, b"not the file").unwrap();
    server.clear_paths();
    let third = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run_env(
        third.path(),
        &["add", "nano-board"],
        &server.template(),
        &env,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let refetched = server.paths();
    assert!(
        refetched.iter().any(|path| path.contains(sha)),
        "{refetched:?}"
    );
    assert!(
        refetched.iter().all(|path| !path.contains(kept)),
        "{refetched:?}"
    );
    assert_parts_match(third.path(), registry);
}

#[test]
fn a_warm_add_and_part_fetch_only_the_index_and_a_bad_record_is_fetched_again() {
    let registry = registry_env::dir();
    let server = Server::bind(registry.to_path_buf(), Mode::Observe);
    let cache = tempfile::tempdir().unwrap();
    let cache_home = cache.path().display().to_string();
    let trust = server.trust();
    let mut env: Vec<(&str, &str)> = trust
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    env.push(("XDG_CACHE_HOME", cache_home.as_str()));

    let first = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) =
        run_env(first.path(), &["add", "led-5mm"], &server.template(), &env);
    assert_eq!(code, 0, "{stdout}{stderr}");
    server.clear_paths();
    let second = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) =
        run_env(second.path(), &["add", "led-5mm"], &server.template(), &env);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let warm_add = server.paths();
    assert_eq!(warm_add.len(), 1, "{warm_add:?}");
    assert!(warm_add[0].ends_with("/index.json"), "{warm_add:?}");

    let sha = record_sha(registry, "led-5mm");
    let cached = cache.path().join("orbe").join("files").join(&sha);
    assert!(cached.is_file(), "{}", cached.display());
    fs::write(&cached, b"not the record").unwrap();
    server.clear_paths();
    let third = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) =
        run_env(third.path(), &["add", "led-5mm"], &server.template(), &env);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let refetched = server.paths();
    assert!(
        refetched.iter().any(|path| path.contains("led-5mm@")),
        "{refetched:?}"
    );
    assert!(
        refetched.iter().all(|path| !path.contains("/files/")),
        "{refetched:?}"
    );
    assert_parts_match(third.path(), registry);

    let part_cache = tempfile::tempdir().unwrap();
    let part_home = part_cache.path().display().to_string();
    let mut part_env: Vec<(&str, &str)> = trust
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    part_env.push(("XDG_CACHE_HOME", part_home.as_str()));
    let project = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run_env(
        project.path(),
        &["part", "led-5mm"],
        &server.template(),
        &part_env,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    server.clear_paths();
    let (code, stdout, stderr) = run_env(
        project.path(),
        &["part", "led-5mm"],
        &server.template(),
        &part_env,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let warm_part = server.paths();
    assert_eq!(warm_part.len(), 1, "{warm_part:?}");
    assert!(warm_part[0].ends_with("/index.json"), "{warm_part:?}");
}

#[test]
fn a_cache_that_cannot_be_created_still_installs() {
    let project = tempfile::tempdir().unwrap();
    let blocked = project.path().join("not-a-directory");
    fs::write(&blocked, b"x").unwrap();
    let home = blocked.display().to_string();
    let (code, stdout, stderr) = run_env(
        project.path(),
        &["add", "led-5mm"],
        &registry_env::template(),
        &[("XDG_CACHE_HOME", home.as_str())],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(project.path().join("parts/led-5mm/part.jsonc").is_file());
    assert_parts_match(project.path(), registry_env::dir());
}

fn assert_parts_match(project: &Path, registry: &Path) {
    let mut dirs = Vec::new();
    collect_part_dirs(&project.join("parts"), &mut dirs);
    assert!(!dirs.is_empty(), "{}", project.display());
    for dir in dirs {
        let id = dir.file_name().unwrap().to_str().unwrap();
        let version = declared_version(&dir.join("part.jsonc"));
        let record: Value = serde_json::from_slice(
            &fs::read(registry.join(format!("{id}@{version}.json")))
                .unwrap_or_else(|error| panic!("{id}@{version}: {error}")),
        )
        .unwrap();
        for file in record["files"].as_array().unwrap() {
            let rel = file["path"].as_str().unwrap();
            let sha = file["sha256"].as_str().unwrap();
            let got = fs::read(dir.join(rel)).unwrap_or_else(|error| panic!("{id} {rel}: {error}"));
            let want = fs::read(registry.join("files").join(sha)).unwrap();
            assert_eq!(got, want, "{id} {rel}");
        }
    }
}

fn collect_part_dirs(root: &Path, dirs: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("part.jsonc").is_file() {
            dirs.push(path.clone());
        }
        collect_part_dirs(&path, dirs);
    }
}

fn record_sha(registry: &Path, name: &str) -> String {
    let version = highest_version(registry, name);
    let index: Value =
        serde_json::from_slice(&fs::read(registry.join("index.json")).unwrap()).unwrap();
    index["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name && row["version"] == version)
        .unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_string()
}

fn file_sha(registry: &Path, id: &str, path: &str) -> String {
    let version = highest_version(registry, id);
    let record: Value =
        serde_json::from_slice(&fs::read(registry.join(format!("{id}@{version}.json"))).unwrap())
            .unwrap();
    record["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == path)
        .unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_string()
}

fn cloned_registry() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("reg");
    let status = Command::new("cp")
        .args(["-a", "--link"])
        .arg(registry_env::dir())
        .arg(&dest)
        .status()
        .unwrap();
    assert!(status.success());
    (tmp, dest)
}

fn led_version_used_by_nano(reg: &Path) -> String {
    let version = highest_version(reg, "nano-board");
    let record: Value =
        serde_json::from_slice(&fs::read(reg.join(format!("nano-board@{version}.json"))).unwrap())
            .unwrap();
    record["registryDependencies"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|dep| dep.as_str().unwrap().strip_prefix("led-0603@"))
        .expect("nano-board uses led-0603")
        .to_string()
}

fn highest_version(reg: &Path, name: &str) -> String {
    let index: Value = serde_json::from_slice(&fs::read(reg.join("index.json")).unwrap()).unwrap();
    index["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["name"] == name)
        .max_by_key(|row| version_tuple(row["version"].as_str().unwrap()))
        .unwrap()["version"]
        .as_str()
        .unwrap()
        .to_string()
}

fn version_tuple(text: &str) -> (u64, u64, u64) {
    let mut parts = text.split('.');
    (
        parts.next().unwrap().parse().unwrap(),
        parts.next().unwrap().parse().unwrap(),
        parts.next().unwrap().parse().unwrap(),
    )
}

/// Publishes `led-0603` at `new`, with that version written into its `part.jsonc`.
fn publish_led_0603(reg: &Path, old: &str, new: &str) {
    let path = reg.join(format!("led-0603@{old}.json"));
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let files = record["files"].as_array_mut().unwrap();
    let part = files
        .iter_mut()
        .find(|file| file["path"] == "part.jsonc")
        .unwrap();
    let old_sha = part["sha256"].as_str().unwrap().to_string();
    let bytes = fs::read(reg.join("files").join(&old_sha)).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let from = format!("\"version\": \"{old}\"");
    assert_eq!(text.matches(&from).count(), 1, "{text}");
    let text = text.replacen(&from, &format!("\"version\": \"{new}\""), 1);
    let bytes = text.into_bytes();
    let sha = sha256_hex(&bytes);
    fs::write(reg.join("files").join(&sha), &bytes).unwrap();
    part["sha256"] = json!(sha);
    part["bytes"] = json!(bytes.len());
    part["url"] = json!(format!("files/{sha}"));
    let mut folder = BTreeMap::new();
    for file in record["files"].as_array().unwrap() {
        let path = file["path"].as_str().unwrap().to_string();
        let sha = file["sha256"].as_str().unwrap();
        folder.insert(path, fs::read(reg.join("files").join(sha)).unwrap());
    }
    record["hash"] = json!(folder_hash(&folder));
    record["version"] = json!(new);
    write_record(reg, "led-0603", new, &record);
}

/// A part whose files are `led-0603`'s, so the folder hash stays, and whose record depends on `deps`.
fn add_part_from_led(reg: &Path, name: &str, version: &str, deps: &[String]) {
    let source = highest_version(reg, "led-0603");
    let mut record: Value =
        serde_json::from_slice(&fs::read(reg.join(format!("led-0603@{source}.json"))).unwrap())
            .unwrap();
    record["name"] = json!(name);
    record["version"] = json!(version);
    record["registryDependencies"] = json!(deps);
    write_record(reg, name, version, &record);
}

fn write_record(reg: &Path, name: &str, version: &str, record: &Value) {
    let body = serde_json::to_vec(record).unwrap();
    let sha = sha256_hex(&body);
    let url = format!("{name}@{version}.json");
    write_fresh(&reg.join(&url), &body);
    let mut index: Value =
        serde_json::from_slice(&fs::read(reg.join("index.json")).unwrap()).unwrap();
    let mut row = index["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "led-0603")
        .unwrap()
        .clone();
    row["name"] = json!(name);
    row["version"] = json!(version);
    row["url"] = json!(url);
    row["sha256"] = json!(sha);
    row["bytes"] = json!(body.len());
    index["items"].as_array_mut().unwrap().push(row);
    write_fresh(
        &reg.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );
}

fn write_fresh(path: &Path, bytes: &[u8]) {
    if path.exists() {
        fs::remove_file(path).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn folder_hash(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut lines = String::new();
    for (path, bytes) in files {
        lines.push_str(&sha256_hex(bytes));
        lines.push_str("  ");
        lines.push_str(path);
        lines.push('\n');
    }
    format!("sha256:{}", sha256_hex(lines.as_bytes()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn declared_version(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap();
    let key = "\"version\": \"";
    let at = text
        .find(key)
        .unwrap_or_else(|| panic!("{}", path.display()));
    let rest = &text[at + key.len()..];
    rest[..rest.find('"').unwrap()].to_string()
}
