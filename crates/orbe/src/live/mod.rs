//! The live commands from a shell. Each `orbe <command>` sends one request to its session's
//! process and prints the reply as `orbe test -v` prints that line.
//!
//! The wire is JSON lines on a Unix socket: the client writes one `Request` line, the process
//! writes one `Reply` line and closes. A client that hangs up first, as Ctrl-C does, cancels a
//! running `step`, whose reply still comes back.

mod daemon;
mod place;

use std::fs::{self, File};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
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

/// One request: a command, or the usage error its words gave, and the client's working
/// folder. `open` reads its scene from that folder and `snapshot` shows the scene's path from
/// it, so paths read as they would in `orbe test`. A usage error goes to the process only so
/// the refusal can carry the session's real time and state. `stop` is `ORBE_TEST_STOP` from
/// the shell that ran `open`.
#[derive(Debug, Serialize, Deserialize)]
struct Request {
    cwd: String,
    command: Result<Command, Error>,
    stop: Option<String>,
}

/// The longest line either side reads.
const LINE: u64 = 1 << 20;

/// `open` waits up to 2 s for a process it started to listen, polling every 10 ms.
const POLLS: u32 = 200;
const POLL: Duration = Duration::from_millis(10);

/// How many times `open` reaches a process, when the one it reached ended before it answered.
const TRIES: u32 = 5;

fn read_line(reader: &mut impl BufRead) -> Option<String> {
    let mut line = String::new();
    match reader.take(LINE).read_line(&mut line) {
        Ok(n) if n > 0 && line.ends_with('\n') => Some(line),
        _ => None,
    }
}

/// `orbe daemon --session <name>`.
pub fn daemon(session: &str, start_log: Option<u64>) -> ExitCode {
    match Name::parse(session) {
        Ok(name) => daemon::run(&name, start_log),
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
        Ok(Answer::One(done)) => {
            if json {
                print_json(&done.json());
            } else {
                println!("{}", done.text());
            }
            ExitCode::from(done.exit())
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

enum Answer {
    One(Done),
    All(Vec<Result<Done, Refusal>>),
}

/// Printed on stdout.
enum Done {
    /// The process's reply, success or failure, as `orbe test -v` prints it.
    Reply(Box<Reply>),
    /// `close` found a socket a crashed process left, and removed it.
    Stale(Stale),
}

struct Stale {
    session: Name,
    /// The process's log, kept when it holds output such as a panic.
    log: Option<PathBuf>,
}

impl Done {
    fn session(&self) -> &str {
        match self {
            Done::Reply(reply) => &reply.session,
            Done::Stale(stale) => stale.session.as_str(),
        }
    }

    fn text(&self) -> String {
        match self {
            Done::Reply(reply) => reply.text(),
            Done::Stale(Stale { log, .. }) => {
                let mut text =
                    String::from("removed the socket of a process that ended without closing");
                if let Some(log) = log {
                    text.push_str(&format!("; its last output is in {}", log.display()));
                }
                text
            }
        }
    }

    fn json(&self) -> Value {
        match self {
            Done::Reply(reply) => reply.json(),
            Done::Stale(Stale { session, log }) => {
                let mut entry =
                    json!({ "ok": true, "session": session.as_str(), "removed": "stale_socket" });
                if let Some(log) = log {
                    entry["log"] = log.display().to_string().into();
                }
                entry
            }
        }
    }

    fn exit(&self) -> u8 {
        match self {
            Done::Reply(reply) => reply.exit(),
            Done::Stale(_) => 0,
        }
    }
}

/// A refusal the client prints itself, on stderr or as one `--json` envelope. `at` is the
/// session's time and state when its process, or a missing socket, told the client. Without
/// it the envelope leaves them out rather than guess.
struct Refusal {
    session: String,
    error: Error,
    at: Option<(u64, State)>,
}

impl Refusal {
    fn reply(&self) -> Reply {
        let (t_ps, state) = self.at.unwrap_or((0, State::Closed));
        Reply {
            session: self.session.clone(),
            t_ps,
            state,
            data: Data::None,
            error: Some(self.error.clone()),
        }
    }

    fn text(&self) -> String {
        self.reply().text()
    }

    fn json(&self) -> Value {
        let mut envelope = self.reply().json();
        if self.at.is_none()
            && let Some(fields) = envelope.as_object_mut()
        {
            fields.remove("t_ps");
            fields.remove("state");
        }
        envelope
    }

    fn exit(&self) -> u8 {
        self.reply().exit()
    }
}

fn refusal(
    session: &str,
    code: Code,
    message: impl Into<String>,
    hint: impl Into<String>,
) -> Refusal {
    Refusal {
        session: session.to_string(),
        error: Error::new(code, message, hint),
        at: None,
    }
}

enum Found {
    Missing,
    Listening(UnixStream),
    /// A socket nothing answers on, which a crashed process left.
    Stale,
}

/// One session, as this shell names it.
struct Client {
    name: Name,
    /// Named with `--session`, so a hint repeats it. `ORBE_SESSION` stays set by itself.
    flagged: bool,
    place: Place,
    socket: PathBuf,
    cwd: String,
}

impl Client {
    fn new(flag: Option<&str>) -> Result<Client, Refusal> {
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
            })?
            .into_os_string()
            .into_string()
            .map_err(|cwd| {
                refusal(
                    shown,
                    Code::IoError,
                    format!(
                        "the working folder {} is not UTF-8, and the session process needs a UTF-8 path",
                        Path::new(&cwd).display()
                    ),
                    "run `orbe` from a folder whose path is UTF-8",
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
            None => Name::default_for(Path::new(&cwd)),
        };
        let place = Place::find()
            .map_err(|(message, hint)| refusal(name.as_str(), Code::IoError, message, hint))?;
        Client::named(name, flag.is_some(), place, cwd)
    }

    fn named(name: Name, flagged: bool, place: Place, cwd: String) -> Result<Client, Refusal> {
        let socket = place
            .socket(&name)
            .map_err(|(message, hint)| refusal(name.as_str(), Code::IoError, message, hint))?;
        Ok(Client {
            name,
            flagged,
            place,
            socket,
            cwd,
        })
    }

    fn refuse(&self, code: Code, message: impl Into<String>, hint: impl Into<String>) -> Refusal {
        refusal(self.name.as_str(), code, message, hint)
    }

    fn run(&self, spec: &Spec, words: &[String]) -> Result<Done, Refusal> {
        let command = match spec.command(words, Path::new("")) {
            Ok(command) => command,
            Err(message) => return Err(self.usage(Error::new(Code::Usage, message, ""))),
        };
        match command {
            Command::Close => self.close(),
            Command::Open(_) => self.open(command).map(Done::Reply),
            _ => match self.find()? {
                Found::Listening(stream) => self.send(stream, Ok(command)).map(Done::Reply),
                Found::Missing => Err(self.no_session()),
                Found::Stale => Err(self.crashed("ended without closing")),
            },
        }
    }

    fn find(&self) -> Result<Found, Refusal> {
        let shown = self.socket.display();
        let meta = match fs::symlink_metadata(&self.socket) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Found::Missing);
            }
            Err(error) => {
                return Err(self.refuse(
                    Code::IoError,
                    format!("cannot read {shown}: {error}"),
                    "",
                ));
            }
        };
        if !meta.file_type().is_socket() {
            let (what, remove) = if meta.is_symlink() {
                ("a symlink", "rm")
            } else if meta.is_dir() {
                ("a folder", "rm -r")
            } else if meta.is_file() {
                ("a file", "rm")
            } else {
                ("a special file", "rm")
            };
            return Err(self.refuse(
                Code::IoError,
                format!(
                    "{shown} is {what}, not the socket of session `{}`",
                    self.name
                ),
                format!("remove it with `{remove} {shown}`"),
            ));
        }
        match UnixStream::connect(&self.socket) {
            Ok(stream) => Ok(Found::Listening(stream)),
            Err(error) => match error.kind() {
                std::io::ErrorKind::ConnectionRefused => Ok(Found::Stale),
                std::io::ErrorKind::NotFound => Ok(Found::Missing),
                _ => Err(self.refuse(
                    Code::IoError,
                    format!("cannot reach session `{}` at {shown}: {error}", self.name),
                    "",
                )),
            },
        }
    }

    /// `open` to the session's process, starting one when none listens. When the process it
    /// reached ends before it answers, it starts again.
    fn open(&self, command: Command) -> Result<Box<Reply>, Refusal> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let answered = match self.find()? {
                Found::Listening(stream) => self.send(stream, Ok(command.clone())),
                Found::Missing | Found::Stale => self
                    .start(attempt)
                    .and_then(|stream| self.send(stream, Ok(command.clone()))),
            };
            match answered {
                Err(refused) if refused.error.code == Code::NoSession && attempt < TRIES => {}
                answered => return answered,
            }
        }
    }

    /// Ends the session: `close` to its process, or, for a socket a crashed process left, the
    /// socket removed and the log kept when it holds output.
    fn close(&self) -> Result<Done, Refusal> {
        match self.find()? {
            Found::Listening(stream) => self.send(stream, Ok(Command::Close)).map(Done::Reply),
            Found::Missing => Err(self.no_session()),
            Found::Stale => {
                fs::remove_file(&self.socket).map_err(|error| {
                    self.refuse(
                        Code::IoError,
                        format!("cannot remove {}: {error}", self.socket.display()),
                        "",
                    )
                })?;
                let log = self.place.log(&self.name);
                let kept = fs::symlink_metadata(&log).is_ok_and(|m| m.is_file() && m.len() > 0);
                if !kept {
                    let _ = fs::remove_file(&log);
                }
                Ok(Done::Stale(Stale {
                    session: self.name.clone(),
                    log: kept.then_some(log),
                }))
            }
        }
    }

    /// A usage error, with the session's time and state from its process when one answers
    /// within a second.
    fn usage(&self, error: Error) -> Refusal {
        let at = match self.find() {
            Ok(Found::Listening(stream)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                match self.send(stream, Err(error.clone())) {
                    Ok(reply) => Some((reply.t_ps, reply.state)),
                    Err(refused) if refused.error.code == Code::NoSession => refused.at,
                    Err(_) => None,
                }
            }
            Ok(Found::Missing) => Some((0, State::Closed)),
            _ => None,
        };
        Refusal {
            session: self.name.to_string(),
            error,
            at,
        }
    }

    /// Starts the session's process, detached in its own process group so a terminal's Ctrl-C
    /// reaches only the client, and waits for it to listen. Its log is its own until it holds
    /// the socket, named by this client's pid and its attempt.
    fn start(&self, attempt: u32) -> Result<UnixStream, Refusal> {
        let id = u64::from(std::process::id()) * u64::from(TRIES) + u64::from(attempt);
        let log = self.place.start_log(&self.name, id);
        let stderr = self.create_log(&log)?;
        let spawned = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .args([
                    "daemon",
                    &format!("--session={}", self.name),
                    &format!("--start-log={id}"),
                ])
                .current_dir("/")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(stderr)
                .process_group(0)
                .spawn()
        });
        let mut child = spawned.map_err(|error| {
            let _ = fs::remove_file(&log);
            self.refuse(
                Code::IoError,
                format!("cannot start the session process: {error}"),
                "",
            )
        })?;
        for _ in 0..POLLS {
            if let Ok(Found::Listening(stream)) = self.find() {
                return Ok(stream);
            }
            if let Ok(Some(status)) = child.try_wait() {
                if status.success() {
                    return Err(self.no_session());
                }
                return Err(self.failed_start(
                    &log,
                    format!("the session process exited ({status}) before it listened"),
                ));
            }
            std::thread::sleep(POLL);
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(self.failed_start(&log, "the session process did not listen within 2 s"))
    }

    /// A symlink at the session's log is refused, since the process that wins the socket
    /// renames its log over it.
    fn create_log(&self, start: &Path) -> Result<File, Refusal> {
        use rustix::fs::{Mode, OFlags};
        let log = self.place.log(&self.name);
        if fs::symlink_metadata(&log).is_ok_and(|meta| meta.is_symlink()) {
            let shown = log.display();
            return Err(self.refuse(
                Code::IoError,
                format!(
                    "{shown} is a symlink, not the log of session `{}`",
                    self.name
                ),
                format!("remove it with `rm {shown}`"),
            ));
        }
        let flags =
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        rustix::fs::open(start, flags, Mode::RUSR | Mode::WUSR)
            .map(File::from)
            .map_err(|errno| {
                self.refuse(
                    Code::IoError,
                    format!("cannot write the log {}: {errno}", start.display()),
                    "",
                )
            })
    }

    /// A start that failed. Its log stays, named in the hint, only when it holds output.
    fn failed_start(&self, log: &Path, message: impl Into<String>) -> Refusal {
        let hint = if fs::symlink_metadata(log).is_ok_and(|m| m.len() > 0) {
            format!("its output is in {}", log.display())
        } else {
            let _ = fs::remove_file(log);
            String::new()
        };
        self.refuse(Code::IoError, message, hint)
    }

    /// A `no_session` reply, from a process with no scene yet or one that is ending, becomes
    /// the client's own, and a hint to `open` again names the shell command.
    fn send(
        &self,
        stream: UnixStream,
        command: Result<Command, Error>,
    ) -> Result<Box<Reply>, Refusal> {
        if let Ok(hangup) = stream.try_clone() {
            let pressed = AtomicBool::new(false);
            let _ = ctrlc::set_handler(move || {
                if pressed.swap(true, Ordering::Relaxed) {
                    std::process::exit(130);
                }
                let _ = hangup.shutdown(Shutdown::Write);
            });
        }
        let stop = matches!(command, Ok(Command::Open(_)))
            .then(|| std::env::var("ORBE_TEST_STOP").ok())
            .flatten();
        let request = Request {
            cwd: self.cwd.clone(),
            command,
            stop,
        };
        let mut wire = serde_json::to_string(&request).map_err(|error| {
            self.refuse(
                Code::IoError,
                format!("cannot write the request: {error}"),
                "",
            )
        })?;
        wire.push('\n');
        (&stream)
            .write_all(wire.as_bytes())
            .map_err(|_| self.unanswered())?;
        let mut line = String::new();
        match BufReader::new(&stream).take(LINE).read_line(&mut line) {
            Ok(_) if line.ends_with('\n') => {}
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(self.refuse(
                    Code::IoError,
                    format!("session `{}` did not answer in time", self.name),
                    "",
                ));
            }
            _ => return Err(self.unanswered()),
        }
        let mut reply = serde_json::from_str::<Box<Reply>>(&line).map_err(|error| {
            self.refuse(
                Code::IoError,
                format!("the session process sent a reply this orbe cannot read: {error}"),
                "run `orbe close --all`, then open the session again",
            )
        })?;
        if let Some(error) = &mut reply.error {
            match error.code {
                Code::NoSession => return Err(self.no_session()),
                Code::Ended => error.hint = self.open_hint("to start again"),
                Code::Faulted => {
                    error.hint = format!(
                        "run `orbe snapshot{}` to see the diagnostics, fix the scene or firmware, then {}",
                        self.session_flag(),
                        self.open_hint("again")
                    );
                }
                _ => {}
            }
        }
        Ok(reply)
    }

    fn no_session(&self) -> Refusal {
        Refusal {
            at: Some((0, State::Closed)),
            ..self.refuse(
                Code::NoSession,
                format!("no session `{}` is running", self.name),
                self.open_hint("to start it"),
            )
        }
    }

    /// A connection that ended with no reply. A process that ends on its own removes its
    /// socket before it drops a connection, so only a socket still there that nothing answers
    /// on was left by a crash.
    fn unanswered(&self) -> Refusal {
        match self.find() {
            Ok(Found::Stale) => self.crashed("ended before it answered"),
            Ok(Found::Missing | Found::Listening(_)) => self.no_session(),
            Err(refused) => refused,
        }
    }

    fn crashed(&self, how: &str) -> Refusal {
        let log = self.place.log(&self.name);
        let mut hint = self.open_hint("to start it again");
        if fs::symlink_metadata(&log).is_ok_and(|m| m.len() > 0) {
            hint.push_str(&format!("; its last output is in {}", log.display()));
        }
        self.refuse(
            Code::SessionCrashed,
            format!("the process of session `{}` {how}", self.name),
            hint,
        )
    }

    fn session_flag(&self) -> String {
        if self.flagged {
            format!(" --session {}", self.name)
        } else {
            String::new()
        }
    }

    /// The `orbe open` that starts this session from this folder, or, with no scene.jsonc here
    /// or above, one with a path to fill in.
    fn open_hint(&self, purpose: &str) -> String {
        let session = self.session_flag();
        let found = Path::new(&self.cwd)
            .ancestors()
            .position(|dir| dir.join("scene.jsonc").is_file());
        match found {
            Some(0) => format!("run `orbe open{session}` {purpose}"),
            Some(up) => format!(
                "run `orbe open {}scene.jsonc{session}` {purpose}",
                "../".repeat(up)
            ),
            None => format!(
                "run `orbe open path/to/scene.jsonc{session}` {purpose}; no scene.jsonc is in this folder or above"
            ),
        }
    }

    /// Closes every session of this build for this user, and removes sockets that crashed
    /// processes left.
    fn close_all(&self) -> Result<Vec<Result<Done, Refusal>>, Refusal> {
        let names = self.place.names().map_err(|error| {
            self.refuse(
                Code::IoError,
                format!("cannot list the sessions: {error}"),
                "",
            )
        })?;
        let closed = names
            .into_iter()
            .map(|name| Client::named(name, true, self.place.clone(), self.cwd.clone())?.close());
        Ok(closed.collect())
    }
}

fn print_all(closed: &[Result<Done, Refusal>], json: bool) -> ExitCode {
    let exit = closed
        .iter()
        .map(|c| c.as_ref().map_or_else(Refusal::exit, Done::exit))
        .max()
        .unwrap_or(0);
    if json {
        let sessions: Vec<Value> = closed
            .iter()
            .map(|c| c.as_ref().map_or_else(Refusal::json, Done::json))
            .collect();
        print_json(&json!({ "ok": exit == 0, "data": { "sessions": sessions } }));
    } else if closed.is_empty() {
        println!("no sessions are running");
    } else {
        for c in closed {
            match c {
                Ok(done) => println!("{}: {}", done.session(), done.text()),
                Err(refused) => println!("{}: {}", refused.session, refused.text()),
            }
        }
    }
    ExitCode::from(exit)
}

fn print_json(value: &Value) {
    println!("{}", serde_json::to_string(value).expect("a JSON value"));
}
