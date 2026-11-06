//! The live commands from a shell. Each `orbe <command>` sends one request to its session's
//! process and prints the reply as `orbe test -v` prints that line.
//!
//! The wire is JSON lines on a Unix socket: the client writes one `Request` line, the process
//! writes one `Reply` line and closes. A client that hangs up first, as Ctrl-C does, cancels a
//! running `step`, whose reply still comes back.

mod daemon;
mod place;

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use orbe_session::{Code, Command, Data, Error, Reply, Spec, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use place::{Name, Place};

/// One request: a command, and the client's working folder, which `open` reads its scene
/// from, so paths in the reply read as they would in `orbe test`.
#[derive(Debug, Serialize, Deserialize)]
struct Request {
    cwd: PathBuf,
    command: Command,
}

/// The longest line either side reads.
const LINE: u64 = 1 << 20;

/// `open` waits up to 2 s for a process it started to listen, polling every 10 ms.
const POLLS: u32 = 200;
const POLL: Duration = Duration::from_millis(10);

fn read_line(reader: &mut impl BufRead) -> Option<String> {
    let mut line = String::new();
    match reader.take(LINE).read_line(&mut line) {
        Ok(n) if n > 0 && line.ends_with('\n') => Some(line),
        _ => None,
    }
}

/// `orbe daemon --session <name>`.
pub fn daemon(session: &str) -> ExitCode {
    match Name::parse(session) {
        Ok(name) => daemon::run(&name),
        Err(message) => {
            eprintln!("error[usage]: {message}");
            ExitCode::from(2)
        }
    }
}

/// The words of one live command, without the flags every live command shares.
struct Invocation {
    words: Vec<String>,
    session: Option<String>,
    all: bool,
}

impl Invocation {
    fn parse(spec: &Spec, args: &[String]) -> Result<Invocation, String> {
        let mut invocation = Invocation {
            words: Vec::new(),
            session: None,
            all: false,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--json" => {}
                "--session" => {
                    let name = args.next().ok_or("`--session` takes a name")?;
                    invocation.session = Some(name.clone());
                }
                "--all" if spec.name == "close" => invocation.all = true,
                _ => match arg.strip_prefix("--session=") {
                    Some(name) => invocation.session = Some(name.to_string()),
                    None => invocation.words.push(arg.clone()),
                },
            }
        }
        if invocation.all && !invocation.words.is_empty() {
            return Err("`close --all` takes no other arguments".into());
        }
        Ok(invocation)
    }
}

/// `orbe <command> [--session <name>] [--json]`.
pub fn main(spec: &Spec, args: &[String]) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    let answer = Invocation::parse(spec, args)
        .map_err(|message| {
            refusal(
                "",
                Code::Usage,
                message,
                format!("usage: orbe {}", spec.usage),
            )
        })
        .and_then(|invocation| {
            let client = Client::new(invocation.session.as_deref())?;
            if invocation.all {
                return client.close_all().map(Answer::All);
            }
            client.run(spec, &invocation.words).map(Answer::One)
        });
    match answer {
        Ok(Answer::One(reply)) => {
            if json {
                print_json(&reply.json());
            } else {
                println!("{}", reply.text());
            }
            ExitCode::from(reply.exit())
        }
        Ok(Answer::All(closed)) => print_all(&closed, json),
        Err(refused) => {
            if json {
                print_json(&refused.json());
            } else {
                eprintln!("{}", refused.text());
            }
            ExitCode::from(refused.exit())
        }
    }
}

/// One session's reply, or what `close --all` did to each session.
enum Answer {
    One(Box<Reply>),
    All(Vec<Closed>),
}

/// What `close --all` did to one session.
enum Closed {
    Answered(Box<Reply>),
    /// A socket that a crashed process left, removed.
    Stale(Name),
}

fn refusal(
    session: &str,
    code: Code,
    message: impl Into<String>,
    hint: impl Into<String>,
) -> Box<Reply> {
    Box::new(Reply {
        session: session.to_string(),
        t_ps: 0,
        state: State::Closed,
        data: Data::None,
        error: Some(Error::new(code, message, hint)),
    })
}

/// One session, as this shell names it.
struct Client {
    name: Name,
    /// Named with `--session`, so a hint repeats it. `ORBE_SESSION` stays set by itself.
    flagged: bool,
    place: Place,
    cwd: PathBuf,
}

impl Client {
    fn new(flag: Option<&str>) -> Result<Client, Box<Reply>> {
        let shown = flag.unwrap_or_default();
        let cwd = std::env::current_dir()
            .and_then(|dir| dir.canonicalize())
            .map_err(|error| {
                refusal(
                    shown,
                    Code::IoError,
                    format!("cannot read the working folder: {error}"),
                    "run `orbe` from a folder that exists",
                )
            })?;
        let env = std::env::var("ORBE_SESSION").ok().filter(|n| !n.is_empty());
        let name = match flag.or(env.as_deref()) {
            Some(text) => Name::parse(text).map_err(|message| {
                refusal(
                    text,
                    Code::Usage,
                    message,
                    "name it with `--session <name>` or ORBE_SESSION",
                )
            })?,
            None => Name::default_for(&cwd),
        };
        let place = Place::find()
            .map_err(|(message, hint)| refusal(name.as_str(), Code::IoError, message, hint))?;
        Ok(Client {
            flagged: flag.is_some(),
            name,
            place,
            cwd,
        })
    }

    fn refuse(
        &self,
        code: Code,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Box<Reply> {
        refusal(self.name.as_str(), code, message, hint)
    }

    fn run(&self, spec: &Spec, words: &[String]) -> Result<Box<Reply>, Box<Reply>> {
        let command = spec
            .command(words, Path::new(""))
            .map_err(|message| self.refuse(Code::Usage, message, ""))?;
        let socket = self.place.socket(&self.name);
        let stream = match command {
            Command::Open(_) => match UnixStream::connect(&socket) {
                Ok(stream) => stream,
                Err(_) => self.start()?,
            },
            _ => self.connect()?,
        };
        self.send(stream, command)
    }

    fn connect(&self) -> Result<UnixStream, Box<Reply>> {
        let socket = self.place.socket(&self.name);
        UnixStream::connect(&socket).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => self.refuse(
                Code::NoSession,
                format!("no session `{}` is running", self.name),
                format!("run `{}` to start it", self.open_command()),
            ),
            std::io::ErrorKind::ConnectionRefused => self.crashed("ended without closing"),
            _ => self.refuse(
                Code::IoError,
                format!(
                    "cannot reach session `{}` at {}: {error}",
                    self.name,
                    socket.display()
                ),
                "run `orbe close --all`, then open the session again",
            ),
        })
    }

    /// Starts the session's process, detached in its own process group so a terminal's Ctrl-C
    /// reaches only the client, and waits for it to listen.
    fn start(&self) -> Result<UnixStream, Box<Reply>> {
        let io = |what: &str, error: std::io::Error| {
            self.refuse(
                Code::IoError,
                format!("cannot start the session process: {what}: {error}"),
                "",
            )
        };
        let log = self.place.log(&self.name);
        let stderr = File::create(&log).map_err(|e| io(&log.display().to_string(), e))?;
        let exe = std::env::current_exe().map_err(|e| io("cannot find orbe", e))?;
        let mut child = std::process::Command::new(exe)
            .args(["daemon", &format!("--session={}", self.name)])
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .process_group(0)
            .spawn()
            .map_err(|e| io("spawn", e))?;
        let socket = self.place.socket(&self.name);
        let output = format!("its output is in {}", log.display());
        for _ in 0..POLLS {
            if let Ok(stream) = UnixStream::connect(&socket) {
                return Ok(stream);
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(self.refuse(
                    Code::IoError,
                    format!("the session process exited ({status}) before it listened"),
                    output,
                ));
            }
            std::thread::sleep(POLL);
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(self.refuse(
            Code::IoError,
            "the session process did not listen within 2 s",
            output,
        ))
    }

    fn send(&self, stream: UnixStream, command: Command) -> Result<Box<Reply>, Box<Reply>> {
        if let Ok(hangup) = stream.try_clone() {
            let pressed = AtomicBool::new(false);
            let _ = ctrlc::set_handler(move || {
                if pressed.swap(true, Ordering::Relaxed) {
                    std::process::exit(130);
                }
                let _ = hangup.shutdown(Shutdown::Write);
            });
        }
        let request = Request {
            cwd: self.cwd.clone(),
            command,
        };
        let mut wire = serde_json::to_string(&request).expect("a request serializes");
        wire.push('\n');
        let line = (&stream)
            .write_all(wire.as_bytes())
            .ok()
            .and_then(|()| read_line(&mut BufReader::new(&stream)))
            .ok_or_else(|| self.crashed("ended before it answered"))?;
        serde_json::from_str::<Box<Reply>>(&line).map_err(|error| {
            self.refuse(
                Code::IoError,
                format!("the session process sent a reply this orbe cannot read: {error}"),
                "run `orbe close --all`, then open the session again",
            )
        })
    }

    fn crashed(&self, how: &str) -> Box<Reply> {
        let log = self.place.log(&self.name);
        let mut hint = format!("run `{}` to start it again", self.open_command());
        if std::fs::metadata(&log).is_ok_and(|m| m.len() > 0) {
            hint.push_str(&format!("; its last output is in {}", log.display()));
        }
        self.refuse(
            Code::SessionCrashed,
            format!("the process of session `{}` {how}", self.name),
            hint,
        )
    }

    /// The `orbe open` that starts this session from this folder.
    fn open_command(&self) -> String {
        let mut command = String::from("orbe open");
        if !self.cwd.join("scene.jsonc").is_file() {
            let up = self
                .cwd
                .ancestors()
                .skip(1)
                .position(|dir| dir.join("scene.jsonc").is_file());
            command.push(' ');
            command.push_str(&match up {
                Some(up) => format!("{}scene.jsonc", "../".repeat(up + 1)),
                None => "<scene>".into(),
            });
        }
        if self.flagged {
            command.push_str(&format!(" --session {}", self.name));
        }
        command
    }

    /// Closes every session of this build for this user, and removes sockets that crashed
    /// processes left.
    fn close_all(&self) -> Result<Vec<Closed>, Box<Reply>> {
        let names = self.place.names().map_err(|error| {
            self.refuse(
                Code::IoError,
                format!("cannot list the sessions: {error}"),
                "",
            )
        })?;
        let closed = names.into_iter().map(|name| {
            let client = Client {
                name,
                flagged: true,
                place: self.place.clone(),
                cwd: self.cwd.clone(),
            };
            let reply = client
                .connect()
                .and_then(|stream| client.send(stream, Command::Close));
            match reply {
                Err(refused)
                    if refused
                        .error
                        .as_ref()
                        .is_some_and(|e| e.code == Code::SessionCrashed) =>
                {
                    let _ = std::fs::remove_file(client.place.socket(&client.name));
                    let _ = std::fs::remove_file(client.place.log(&client.name));
                    Closed::Stale(client.name)
                }
                Ok(reply) | Err(reply) => Closed::Answered(reply),
            }
        });
        Ok(closed.collect())
    }
}

fn print_all(closed: &[Closed], json: bool) -> ExitCode {
    let exit = closed
        .iter()
        .map(|c| match c {
            Closed::Answered(reply) => reply.exit(),
            Closed::Stale(_) => 0,
        })
        .max()
        .unwrap_or(0);
    if json {
        let sessions: Vec<Value> = closed
            .iter()
            .map(|c| match c {
                Closed::Answered(reply) => reply.json(),
                Closed::Stale(name) => {
                    json!({ "ok": true, "session": name.as_str(), "removed": "stale_socket" })
                }
            })
            .collect();
        print_json(&json!({ "ok": exit == 0, "data": { "sessions": sessions } }));
    } else if closed.is_empty() {
        println!("no sessions are running");
    } else {
        for c in closed {
            match c {
                Closed::Answered(reply) => {
                    println!("{}: {}", reply.session, reply.text());
                }
                Closed::Stale(name) => {
                    println!("{name}: removed the socket of a process that ended without closing");
                }
            }
        }
    }
    ExitCode::from(exit)
}

fn print_json(value: &Value) {
    println!("{}", serde_json::to_string(value).expect("a JSON value"));
}
