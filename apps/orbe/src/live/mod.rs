//! The live commands from a shell. Each `orbe <command>` sends one request to its session's
//! process and prints the reply as `orbe test -v` prints that line.
//!
//! The wire is JSON lines on a Unix socket: the client writes one `Request` line, the process
//! writes one `Reply` line and closes. A client that hangs up first, as Ctrl-C does, cancels a
//! running `step`, whose reply still comes back.
//!
//! `orbe show` asks the process for a viewer's URL and opens it in the default browser.

mod daemon;
mod page;
mod place;
mod viewer;

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

/// One request: what it asks, and the client's working folder. `open` reads its scene from
/// that folder and `snapshot` shows the scene's path from it, so paths read as they would in
/// `orbe test`. `stop` is `ORBE_TEST_STOP` from the shell that ran `open`.
#[derive(Debug, Serialize, Deserialize)]
struct Request {
    cwd: String,
    ask: Ask,
    stop: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
enum Ask {
    /// A live command, or the usage error its words gave. A usage error goes to the process
    /// only so the refusal can carry the session's real time and state.
    Run(Result<Command, Error>),
    /// The viewer's URL. `page` is the folder `ORBE_VIEWER_DIR` names, from the client's.
    Show { page: Option<PathBuf> },
}

/// The longest line either side reads.
const LINE: u64 = 1 << 20;

/// `open` waits up to 2 s for a process it started to listen, polling every 10 ms.
const POLLS: u32 = 200;
const POLL: Duration = Duration::from_millis(10);

/// How many times `open` reaches a process, when the one it reached ended before it answered.
const TRIES: u32 = 5;

/// A killed process can close a connection just before its socket, so a connection that ended
/// with no reply looks at a socket that still answers this many more times, `POLL` apart.
const DYING: u32 = 10;

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
    /// `open --headed`: `show` after it.
    headed: bool,
    browser: bool,
}

impl Invocation {
    fn parse(spec: &Spec, args: &[String]) -> Result<Invocation, String> {
        let mut invocation = Invocation {
            words: Vec::new(),
            session: None,
            all: false,
            headed: false,
            browser: true,
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
                "--headed" if spec.name == "open" => invocation.headed = true,
                "--no-browser" if spec.name == "open" => invocation.browser = false,
                _ => match arg.strip_prefix("--session=") {
                    Some(name) => invocation.session = Some(name.to_string()),
                    None => invocation.words.push(arg.clone()),
                },
            }
        }
        if invocation.all && !invocation.words.is_empty() {
            return Err("`close --all` takes no other arguments".into());
        }
        if !invocation.browser && !invocation.headed {
            return Err("`--no-browser` goes with `--headed`".into());
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
            let done = client.run(spec, &invocation.words)?;
            match done {
                Done::Reply(opened) if invocation.headed && opened.ok() => Ok(Answer::Headed(
                    opened,
                    client.show_opened(),
                    invocation.browser,
                )),
                done => Ok(Answer::One(done)),
            }
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
        Ok(Answer::Headed(opened, shown, browser)) => print_headed(&opened, shown, json, browser),
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
    /// `open --headed`: `open`'s reply, `show`'s, and whether to open a browser.
    Headed(Box<Reply>, Result<Box<Reply>, Refusal>, bool),
    All(Vec<Result<Done, Refusal>>),
}

/// `orbe show [--session <name>] [--json] [--no-browser]`.
pub fn show(session: Option<&str>, json: bool, browser: bool) -> ExitCode {
    let shown = Client::new(session).and_then(|client| client.show());
    match shown {
        Ok(reply) => {
            if json {
                print_json(&reply.json());
            } else {
                println!("{}", reply.text());
            }
            if browser {
                open_browser(&reply);
            }
            ExitCode::from(reply.exit())
        }
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

/// `open`'s reply, then `show`'s URL or its error, with `show`'s exit code. With `--json`, one
/// envelope: `open`'s, with the URL in `data.url`, or `ok: false` and `show`'s error.
fn print_headed(
    opened: &Reply,
    shown: Result<Box<Reply>, Refusal>,
    json: bool,
    browser: bool,
) -> ExitCode {
    let mut envelope = opened.json();
    let (error, exit) = match &shown {
        Ok(reply) => (reply.json()["error"].clone(), reply.exit()),
        Err(refused) => (refused.json()["error"].clone(), refused.exit()),
    };
    if let Ok(Reply {
        data: Data::Viewer { url, .. },
        ..
    }) = shown.as_deref()
    {
        envelope["data"]["url"] = url.clone().into();
    }
    if !error.is_null() {
        envelope["ok"] = false.into();
        envelope["error"] = error;
    }
    if json {
        print_json(&envelope);
    } else {
        println!("{}", opened.text());
        match &shown {
            Ok(reply) => println!("{}", reply.text()),
            Err(refused) => eprintln!("{}", refused.text()),
        }
    }
    if browser
        && let Ok(reply) = &shown
        && let Data::Viewer { viewers: 0, .. } = &reply.data
    {
        open_browser(reply);
    }
    ExitCode::from(exit)
}

/// Opens `show`'s URL in the default browser. A browser that does not start leaves the URL to
/// open by hand.
fn open_browser(reply: &Reply) {
    let Data::Viewer { url, .. } = &reply.data else {
        return;
    };
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let started = std::process::Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(error) = started {
        eprintln!("could not run `{program}` to open a browser: {error}; open the URL above");
    }
}

/// Printed on stdout.
enum Done {
    /// The process's reply, success or failure, as `orbe test -v` prints it.
    Reply(Box<Reply>),
    /// `close` found a socket a crashed process left, and removed it.
    Stale(Stale),
    /// `close --all` found a process with no scene, which ends on its own.
    Unopened(Name),
    /// `close --all` removed a start log no process held.
    Left(Name, PathBuf),
}

struct Stale {
    session: Name,
    /// The process's log, kept when it holds output such as a panic.
    log: Option<PathBuf>,
    /// The session's start logs no process held, removed.
    starts: Vec<PathBuf>,
}

const ENDED_START: &str = "the log of a start that ended";

impl Done {
    fn session(&self) -> &str {
        match self {
            Done::Reply(reply) => &reply.session,
            Done::Stale(stale) => stale.session.as_str(),
            Done::Unopened(session) | Done::Left(session, _) => session.as_str(),
        }
    }

    fn text(&self) -> String {
        match self {
            Done::Reply(reply) => reply.text(),
            Done::Stale(Stale { log, starts, .. }) => {
                let mut text =
                    String::from("removed the socket of a process that ended without closing");
                if let Some(log) = log {
                    text.push_str(&format!("; its last output is in {}", log.display()));
                }
                for start in starts {
                    text.push_str(&format!("; removed {}, {ENDED_START}", start.display()));
                }
                text
            }
            Done::Unopened(_) => "no scene is open; its process is starting or ending".into(),
            Done::Left(_, start) => format!("removed {}, {ENDED_START}", start.display()),
        }
    }

    fn json(&self) -> Value {
        match self {
            Done::Reply(reply) => reply.json(),
            Done::Stale(Stale {
                session,
                log,
                starts,
            }) => {
                let mut entry =
                    json!({ "ok": true, "session": session.as_str(), "removed": "stale_socket" });
                if let Some(log) = log {
                    entry["log"] = log.display().to_string().into();
                }
                if !starts.is_empty() {
                    let starts: Vec<String> =
                        starts.iter().map(|s| s.display().to_string()).collect();
                    entry["start_logs"] = starts.into();
                }
                entry
            }
            Done::Unopened(session) => {
                json!({ "ok": true, "session": session.as_str(), "skipped": "no_scene" })
            }
            Done::Left(session, start) => json!({
                "ok": true,
                "session": session.as_str(),
                "removed": "start_log",
                "log": start.display().to_string(),
            }),
        }
    }

    fn exit(&self) -> u8 {
        match self {
            Done::Reply(reply) => reply.exit(),
            Done::Stale(_) | Done::Unopened(_) | Done::Left(..) => 0,
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
            previous_crash: None,
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
                Found::Listening(stream) => {
                    self.send(stream, Ask::Run(Ok(command))).map(Done::Reply)
                }
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
            let (what, remove) = what_is(&meta);
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
        let mut started = None;
        let mut recovered = None;
        for attempt in 1..=TRIES {
            let answered = match self.find()? {
                Found::Listening(stream) => self.send(stream, Ask::Run(Ok(command.clone()))),
                Found::Missing => self.start(attempt).and_then(|(stream, log)| {
                    started = Some(log);
                    self.send_started(stream, Ask::Run(Ok(command.clone())))
                }),
                Found::Stale => {
                    recovered = Some(self.crash_log());
                    self.start(attempt).and_then(|(stream, log)| {
                        started = Some(log);
                        self.send_started(stream, Ask::Run(Ok(command.clone())))
                    })
                }
            };
            match answered {
                Err(refused)
                    if matches!(refused.error.code, Code::NoSession | Code::SessionCrashed) => {}
                Ok(mut reply) => {
                    if let Some(path) = &recovered {
                        reply.previous_crash = Some(match path {
                            Some(path) => format!(
                                "the previous session crashed; its last output is in {path}"
                            ),
                            None => "the previous session crashed".into(),
                        });
                    }
                    return Ok(reply);
                }
                answered => return answered,
            }
        }
        let hint = [Some(self.place.log(&self.name)), started]
            .into_iter()
            .flatten()
            .find(|log| holds_output(log))
            .map(|log| format!("its last output is in {}", log.display()))
            .unwrap_or_default();
        Err(self.refuse(
            Code::SessionCrashed,
            format!(
                "the process of session `{}` ended before it answered `open`, {TRIES} times",
                self.name
            ),
            hint,
        ))
    }

    /// Ends the session: `close` to its process, or, for a socket a crashed process left, the
    /// socket and the session's start logs no process holds removed, and the log kept when it
    /// holds output.
    fn close(&self) -> Result<Done, Refusal> {
        match self.find()? {
            Found::Listening(stream) => self
                .send(stream, Ask::Run(Ok(Command::Close)))
                .map(Done::Reply),
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
                let kept = holds_output(&log);
                if !kept {
                    let _ = fs::remove_file(&log);
                }
                let starts = self.place.remove_left_logs(|name| name == &self.name);
                let _ = fs::remove_file(self.place.version(&self.name));
                Ok(Done::Stale(Stale {
                    session: self.name.clone(),
                    log: kept.then_some(log),
                    starts: starts.into_iter().map(|(_, start)| start).collect(),
                }))
            }
        }
    }

    /// A usage error, with the session's time and state from its process when one answers
    /// within a second.
    fn usage(&self, error: Error) -> Refusal {
        let at = match self.find() {
            Ok(Found::Listening(stream)) => {
                if let Err(refused) = self.refuse_other_version() {
                    return refused;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                match self.send(stream, Ask::Run(Err(error.clone()))) {
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
    /// reaches only the client, and waits for it to listen. It returns the connection and the
    /// process's start log, which is the process's own until it holds the socket.
    fn start(&self, attempt: u32) -> Result<(UnixStream, PathBuf), Refusal> {
        let (id, log, stderr) = self.create_log(attempt)?;
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
                return Ok((stream, log));
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

    /// This start's log, its id, and the file, locked for as long as the process it is given
    /// to runs. The id is this client's pid and attempt, and an id a start that ended left is
    /// passed over. Anything but a file at the session's log is refused, since the process that
    /// wins the socket renames its log over it.
    fn create_log(&self, attempt: u32) -> Result<(u64, PathBuf, File), Refusal> {
        use rustix::fs::{FlockOperation, Mode, OFlags};
        let log = self.place.log(&self.name);
        if let Ok(meta) = fs::symlink_metadata(&log)
            && !meta.is_file()
        {
            let shown = log.display();
            let (what, remove) = what_is(&meta);
            return Err(self.refuse(
                Code::IoError,
                format!("{shown} is {what}, not the log of session `{}`", self.name),
                format!("remove it with `{remove} {shown}`"),
            ));
        }
        let flags =
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut id = u64::from(std::process::id()) * u64::from(TRIES) + u64::from(attempt);
        loop {
            let start = self.place.start_log(&self.name, id);
            let cannot = |errno| {
                self.refuse(
                    Code::IoError,
                    format!("cannot write the log {}: {errno}", start.display()),
                    "",
                )
            };
            let fd = match rustix::fs::open(&start, flags, Mode::RUSR | Mode::WUSR) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::EXIST) => {
                    id += 1 << 40;
                    continue;
                }
                Err(errno) => return Err(cannot(errno)),
            };
            match rustix::fs::flock(&fd, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) if rustix::fs::fstat(&fd).is_ok_and(|stat| stat.st_nlink > 0) => {
                    return Ok((id, start, File::from(fd)));
                }
                Ok(()) | Err(rustix::io::Errno::WOULDBLOCK) => id += 1 << 40,
                Err(errno) => {
                    let _ = fs::remove_file(&start);
                    return Err(cannot(errno));
                }
            }
        }
    }

    /// A start that failed. Its log stays, named in the hint, only when it holds output.
    fn failed_start(&self, log: &Path, message: impl Into<String>) -> Refusal {
        let hint = if holds_output(log) {
            format!("its output is in {}", log.display())
        } else {
            let _ = fs::remove_file(log);
            String::new()
        };
        self.refuse(Code::IoError, message, hint)
    }

    /// `show` to the session's process. A relative `ORBE_VIEWER_DIR` starts from this folder.
    fn show(&self) -> Result<Box<Reply>, Refusal> {
        let page = std::env::var_os(page::FOLDER)
            .filter(|dir| !dir.is_empty())
            .map(|dir| Path::new(&self.cwd).join(dir));
        match self.find()? {
            Found::Listening(stream) => self.send(stream, Ask::Show { page }),
            Found::Missing => Err(self.no_session()),
            Found::Stale => Err(self.crashed("ended without closing")),
        }
    }

    /// `show` after `open --headed`. A `show` the session refused leaves it open, so the hint
    /// says so. A refusal from this side, such as `no_session`, means it may not be open.
    fn show_opened(&self) -> Result<Box<Reply>, Refusal> {
        let mut shown = self.show()?;
        if let Some(error) = &mut shown.error {
            let flag = self.session_flag();
            let open = format!(
                "the session stays open, so run `orbe show{flag}` to try again, or `orbe close{flag}` to end it"
            );
            error.hint = match error.hint.as_str() {
                "" => open,
                hint => format!("{hint}; {open}"),
            };
        }
        Ok(shown)
    }

    /// A `no_session` reply, from a process with no scene yet or one that is ending, becomes
    /// the client's own, and a hint to `open` again names the shell command.
    fn send(&self, stream: UnixStream, ask: Ask) -> Result<Box<Reply>, Refusal> {
        if !matches!(&ask, Ask::Run(Ok(Command::Close))) {
            self.refuse_other_version()?;
        }
        self.dispatch(stream, ask)
    }

    /// `send` for the process this client just started. Its version file may still name the
    /// session it replaced.
    fn send_started(&self, stream: UnixStream, ask: Ask) -> Result<Box<Reply>, Refusal> {
        self.dispatch(stream, ask)
    }

    fn dispatch(&self, stream: UnixStream, ask: Ask) -> Result<Box<Reply>, Refusal> {
        if let Ok(hangup) = stream.try_clone() {
            let pressed = AtomicBool::new(false);
            let _ = ctrlc::set_handler(move || {
                if pressed.swap(true, Ordering::Relaxed) {
                    std::process::exit(130);
                }
                let _ = hangup.shutdown(Shutdown::Write);
            });
        }
        #[cfg(debug_assertions)]
        let stop = matches!(ask, Ask::Run(Ok(Command::Open(_))))
            .then(|| std::env::var("ORBE_TEST_STOP").ok())
            .flatten();
        #[cfg(not(debug_assertions))]
        let stop: Option<String> = None;
        let request = Request {
            cwd: self.cwd.clone(),
            ask,
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

    /// A connection that ended with no reply: the process it reached ended before it answered.
    /// A socket that still answers after `DYING` looks is a new process's, which the command
    /// never reached.
    fn unanswered(&self) -> Refusal {
        for _ in 0..=DYING {
            match self.find() {
                Ok(Found::Listening(_)) => std::thread::sleep(POLL),
                Ok(Found::Missing | Found::Stale) => {
                    return self.crashed("ended before it answered");
                }
                Err(refused) => return refused,
            }
        }
        self.refuse(
            Code::SessionCrashed,
            format!(
                "the process of session `{name}` ended before it answered; a new session `{name}` is running",
                name = self.name
            ),
            format!("run `orbe snapshot{}` to see it", self.session_flag()),
        )
    }

    fn crashed(&self, how: &str) -> Refusal {
        let log = self.crash_log();
        let mut hint = self.open_hint("to start it again");
        if let Some(log) = log {
            hint.push_str(&format!("; its last output is in {log}"));
        }
        self.refuse(
            Code::SessionCrashed,
            format!("the process of session `{}` {how}", self.name),
            hint,
        )
    }

    /// A live session another version started. `close` is the way off it, so `close` does
    /// not ask. A missing file is this version: a process writes the file as it binds.
    fn refuse_other_version(&self) -> Result<(), Refusal> {
        let path = self.place.version(&self.name);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(self.refuse(
                    Code::IoError,
                    format!("cannot read {}: {error}", path.display()),
                    "",
                ));
            }
        };
        let version = text.trim();
        if version.is_empty() || version == env!("CARGO_PKG_VERSION") {
            return Ok(());
        }
        Err(self.refuse(
            Code::IoError,
            format!("session `{}` runs orbe {version}", self.name),
            format!("run `orbe close{}`", self.session_flag()),
        ))
    }

    /// The session log after a newer crashed start, if one was waiting, has replaced it.
    /// `None` when that log holds nothing.
    fn crash_log(&self) -> Option<String> {
        self.place.keep_latest_crash(&self.name);
        let log = self.place.log(&self.name);
        holds_output(&log).then(|| log.display().to_string())
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

    /// Closes every session for this user, whichever version started it, and removes the
    /// sockets and start logs that ended processes left. A process with no scene is left to
    /// end on its own.
    fn close_all(&self) -> Result<Vec<Result<Done, Refusal>>, Refusal> {
        let names = self.place.names().map_err(|error| {
            self.refuse(
                Code::IoError,
                format!("cannot list the sessions: {error}"),
                "",
            )
        })?;
        let mut closed: Vec<Result<Done, Refusal>> = names
            .into_iter()
            .map(|name| {
                let client = Client::named(name, true, self.place.clone(), self.cwd.clone())?;
                match client.close() {
                    Err(refused) if refused.error.code == Code::NoSession => {
                        Ok(Done::Unopened(client.name))
                    }
                    closed => closed,
                }
            })
            .collect();
        let left = self.place.remove_left_logs(|_| true);
        closed.extend(
            left.into_iter()
                .map(|(name, start)| Ok(Done::Left(name, start))),
        );
        Ok(closed)
    }
}

/// What is at a path that should be a socket or a log, and the `rm` that removes it.
fn what_is(meta: &fs::Metadata) -> (&'static str, &'static str) {
    if meta.is_symlink() {
        ("a symlink", "rm")
    } else if meta.is_dir() {
        ("a folder", "rm -r")
    } else if meta.is_file() {
        ("a file", "rm")
    } else {
        ("a special file", "rm")
    }
}

fn holds_output(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
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
