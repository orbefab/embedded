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

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use orbe_session::{Code, Command, Data, Error, Reply, Spec, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use place::{Name, Place};

/// One request: what it asks, and the client's working folder. `open` reads its scene from
/// that folder and `snapshot` shows the scene's path from it, so paths read as they would in
/// `orbe test`. `stop` is `ORBE_TEST_STOP` from the shell that ran `open`, and `step_budget`
/// is `ORBE_TEST_STEP_BUDGET` from the shell that ran `step` (both debug-only test seams).
#[derive(Debug, Serialize, Deserialize)]
struct Request {
    cwd: String,
    ask: Ask,
    stop: Option<String>,
    #[serde(default)]
    step_budget: Option<String>,
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

/// A print to a pipe the reader closed. The process exits 0 instead of panicking with 101.
pub(crate) fn broken_pipe(info: &std::panic::PanicHookInfo<'_>) -> bool {
    if info
        .payload()
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == ErrorKind::BrokenPipe)
    {
        return true;
    }
    let text = info
        .payload()
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| info.payload().downcast_ref::<&str>().copied())
        .unwrap_or("");
    text.contains("Broken pipe") || text.contains("os error 32")
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
                    one_session(&mut invocation.session, name)?;
                }
                "--all" if spec.name == "close" => invocation.all = true,
                "--headed" if spec.name == "open" => invocation.headed = true,
                "--no-browser" if spec.name == "open" => invocation.browser = false,
                _ => match arg.strip_prefix("--session=") {
                    Some(name) => one_session(&mut invocation.session, name)?,
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

/// `--session` once. The same name again counts once. Two names are a usage error, as on
/// `orbe show`, which takes the flag once.
fn one_session(have: &mut Option<String>, name: &str) -> Result<(), String> {
    if have.as_ref().is_some_and(|old| old != name) {
        return Err("the argument '--session <NAME>' cannot be used multiple times".into());
    }
    *have = Some(name.to_string());
    Ok(())
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
}

struct Stale {
    session: Name,
    /// `<pid>.crashed` when the process left output.
    log: Option<PathBuf>,
}

impl Done {
    fn session(&self) -> &str {
        match self {
            Done::Reply(reply) => &reply.session,
            Done::Stale(stale) => stale.session.as_str(),
            Done::Unopened(session) => session.as_str(),
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
            Done::Unopened(_) => "no scene is open; its process is starting or ending".into(),
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
            Done::Unopened(session) => {
                json!({ "ok": true, "session": session.as_str(), "skipped": "no_scene" })
            }
        }
    }

    fn exit(&self) -> u8 {
        match self {
            Done::Reply(reply) => reply.exit(),
            Done::Stale(_) | Done::Unopened(_) => 0,
        }
    }
}

/// A refusal the client prints itself, on stderr or as one `--json` envelope. `at` is the
/// session's time and state when its process, or a missing socket, told the client. Without
/// it the envelope leaves them out rather than guess.
struct Refusal {
    session: Box<str>,
    error: Error,
    at: Option<(u64, State)>,
    /// The crash this command found, in the same place a process's reply puts it.
    previous_crash: Option<Box<str>>,
}

impl Refusal {
    fn reply(&self) -> Reply {
        let (t_ps, state) = self.at.unwrap_or((0, State::Closed));
        Reply {
            session: self.session.to_string(),
            t_ps,
            state,
            data: Data::None,
            error: Some(self.error.clone()),
            previous_crash: self.previous_crash.as_deref().map(str::to_string),
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
        session: session.into(),
        error: Error::new(code, message, hint),
        at: None,
        previous_crash: None,
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
                Found::Stale => Err(self.stale_session()),
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
        // Only a session that was already dead when this open began is "previous". A socket
        // this open's own process left is the attempt that just died.
        let mut began = false;
        let mut recovered = None;
        for _ in 1..=TRIES {
            let answered = match self.find()? {
                Found::Listening(stream) => self.send(stream, Ask::Run(Ok(command.clone()))),
                stale @ (Found::Missing | Found::Stale) => {
                    if matches!(stale, Found::Stale) && !began {
                        recovered = Some(self.preserve_crash());
                    }
                    began = true;
                    self.start()
                        .and_then(|stream| self.send_started(stream, Ask::Run(Ok(command.clone()))))
                }
            };
            match answered {
                Err(refused)
                    if matches!(refused.error.code, Code::NoSession | Code::SessionCrashed) => {}
                answered => return self.with_crash(answered, recovered),
            }
        }
        // The last attempt's output, when it wrote any. An earlier crash stays the file the
        // previous-session note names; this one is a different pid's.
        let hint = self
            .preserve_crash()
            .map(|log| format!("its last output is in {}", log.display()))
            .unwrap_or_default();
        self.with_crash(
            Err(self.refuse(
                Code::SessionCrashed,
                format!(
                    "the process of session `{}` ended before it answered `open`, {TRIES} times",
                    self.name
                ),
                hint,
            )),
            recovered,
        )
    }

    /// Ends the session: `close` to its process, or, for a socket a crashed process left, the
    /// socket and the other runtime files removed. Output that process left stays as
    /// `<pid>.crashed`.
    fn close(&self) -> Result<Done, Refusal> {
        match self.find()? {
            Found::Listening(stream) => self
                .send(stream, Ask::Run(Ok(Command::Close)))
                .map(Done::Reply),
            Found::Missing => Err(self.no_session()),
            Found::Stale => {
                let log = self.dead_files()?;
                Ok(Done::Stale(Stale {
                    session: self.name.clone(),
                    log,
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
            session: self.name.as_str().into(),
            error,
            at,
            previous_crash: None,
        }
    }

    /// Starts the session's process, detached in its own process group so a terminal's Ctrl-C
    /// reaches only the client, and waits for it to listen. Stderr is a pipe until the process
    /// opens its own log, so a failure before that is still visible.
    fn start(&self) -> Result<UnixStream, Refusal> {
        self.refuse_bad_log()?;
        let spawned = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .args(["daemon", &format!("--session={}", self.name)])
                .current_dir("/")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
        });
        let mut child = spawned.map_err(|error| {
            self.refuse(
                Code::IoError,
                format!("cannot start the session process: {error}"),
                "",
            )
        })?;
        let mut stderr = child.stderr.take();
        let mut lost = false;
        for _ in 0..POLLS {
            if let Ok(Some(status)) = child.try_wait() {
                let err = read_stderr(&mut stderr);
                if status.code() == Some(i32::from(orbe_session::EXIT_LOST)) {
                    lost = true;
                } else if status.success() {
                    return Err(self.no_session());
                } else {
                    return Err(self.failed_start(
                        format!("the session process exited ({status}) before it listened"),
                        &err,
                    ));
                }
            }
            if let Ok(Found::Listening(stream)) = self.find() {
                return Ok(stream);
            }
            std::thread::sleep(POLL);
        }
        let _ = child.kill();
        let _ = child.wait();
        let err = read_stderr(&mut stderr);
        let message = if lost {
            "the session process lost the session and no listener appeared within 2 s"
        } else {
            "the session process did not listen within 2 s"
        };
        Err(self.failed_start(message.into(), &err))
    }

    /// Anything but a file where the session log goes is refused before a process starts.
    fn refuse_bad_log(&self) -> Result<(), Refusal> {
        let log = self.place.log(&self.name);
        let Ok(meta) = fs::symlink_metadata(&log) else {
            return Ok(());
        };
        if meta.is_file() {
            return Ok(());
        }
        let shown = log.display();
        let (what, remove) = what_is(&meta);
        Err(self.refuse(
            Code::IoError,
            format!("{shown} is {what}, not the log of session `{}`", self.name),
            format!("remove it with `{remove} {shown}`"),
        ))
    }

    /// A start that failed. Its log stays, named in the hint, only when it holds output.
    /// Otherwise the process's own stderr, from before it opened the log, is the hint.
    fn failed_start(&self, message: String, stderr: &str) -> Refusal {
        let log = self.place.log(&self.name);
        let hint = if place::holds_output(&log) {
            format!("its output is in {}", log.display())
        } else {
            stderr.trim().to_string()
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
            Found::Stale => Err(self.stale_session()),
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
        self.dispatch(stream, ask, true)
    }

    /// `send` for the process this client just started. Its version file may still name the
    /// session it replaced. This does not park a crash log: `open` parks one for a session
    /// that was already dead, and parks this process's output only when that leaves the
    /// earlier one in place.
    fn send_started(&self, stream: UnixStream, ask: Ask) -> Result<Box<Reply>, Refusal> {
        self.dispatch(stream, ask, false)
    }

    fn dispatch(&self, stream: UnixStream, ask: Ask, park: bool) -> Result<Box<Reply>, Refusal> {
        arm_ctrlc(&stream);
        #[cfg(debug_assertions)]
        let stop = matches!(ask, Ask::Run(Ok(Command::Open(_))))
            .then(|| std::env::var("ORBE_TEST_STOP").ok())
            .flatten();
        #[cfg(not(debug_assertions))]
        let stop: Option<String> = None;
        #[cfg(debug_assertions)]
        let step_budget = matches!(ask, Ask::Run(Ok(Command::Step(_))))
            .then(|| std::env::var("ORBE_TEST_STEP_BUDGET").ok())
            .flatten();
        #[cfg(not(debug_assertions))]
        let step_budget: Option<String> = None;
        let request = Request {
            cwd: self.cwd.clone(),
            ask,
            stop,
            step_budget,
        };
        let deadline = reply_deadline(&request.ask);
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
            .map_err(|_| self.unanswered(park))?;
        let line = match read_reply(&stream, deadline, || self.process_dead()) {
            Ok(line) => line,
            Err(ReadEnd::TimedOut) => {
                return Err(self.refuse(
                    Code::IoError,
                    format!("session `{}` did not answer in time", self.name),
                    "",
                ));
            }
            Err(ReadEnd::Closed) => return Err(self.unanswered(park)),
        };
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
                Code::StepTimeout if self.flagged => {
                    if let Some((before, after)) = error.hint.rsplit_once("` to ") {
                        error.hint = format!("{before}{}` to {after}", self.session_flag());
                    }
                }
                Code::Cancelled if error.message.starts_with("the session was closed at ") => {
                    error.hint = self.open_hint("to start it again");
                }
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
    fn unanswered(&self, park: bool) -> Refusal {
        for _ in 0..=DYING {
            match self.find() {
                Ok(Found::Listening(_)) => std::thread::sleep(POLL),
                Ok(Found::Missing | Found::Stale) => {
                    return self.crashed_parked("ended before it answered", park);
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

    /// The session's process is gone. `park` keeps a nonempty log as `<pid>.crashed` and names
    /// it. A retry of `open` passes false so this attempt's log stays for the attempt that
    /// follows to keep, and an earlier crash file is left where it is.
    fn crashed_parked(&self, how: &str, park: bool) -> Refusal {
        let log = if park { self.preserve_crash() } else { None };
        self.crash_refusal(how, log)
    }

    fn crashed(&self, how: &str) -> Refusal {
        self.crashed_parked(how, true)
    }

    fn crash_refusal(&self, how: &str, log: Option<PathBuf>) -> Refusal {
        let mut hint = self.open_hint("to start it again");
        if let Some(log) = log {
            hint.push_str(&format!("; its last output is in {}", log.display()));
        }
        self.refuse(
            Code::SessionCrashed,
            format!("the process of session `{}` {how}", self.name),
            hint,
        )
    }

    /// The pid in the pid file is not a live process. No pid file is a process that has gone.
    fn process_dead(&self) -> bool {
        match self.place.read_pid(&self.name) {
            Some(pid) => !place::pid_alive(pid),
            None => true,
        }
    }

    /// Renames a dead session's nonempty log to `<pid>.crashed` before a new process truncates
    /// it. An empty log names nothing, and neither does an older pid's crash file. A live
    /// process keeps its log. The socket closes before `/proc` shows the process as a zombie,
    /// so a pid that still looks alive is given a moment to exit.
    fn preserve_crash(&self) -> Option<PathBuf> {
        let pid = self.place.read_pid(&self.name)?;
        if !Self::exited(pid) {
            return None;
        }
        let crash = self.place.crashed(&self.name, pid);
        if crash.exists() {
            return place::holds_output(&crash).then_some(crash);
        }
        let log = self.place.log(&self.name);
        if !place::holds_output(&log) {
            return None;
        }
        fs::rename(&log, &crash).ok()?;
        place::holds_output(&crash).then_some(crash)
    }

    /// `pid` has exited. A process that is still alive after the wait is running.
    fn exited(pid: u32) -> bool {
        for _ in 0..100 {
            if !place::pid_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        !place::pid_alive(pid)
    }

    /// A socket nothing answers on. A process that is still alive is reported and left in
    /// place. A process that has ended loses its runtime files, the same way `close` removes
    /// them, and the error names the log when that log is kept.
    fn stale_session(&self) -> Refusal {
        match self.dead_files() {
            Ok(log) => self.crash_refusal("ended without closing", log),
            Err(refused) => refused,
        }
    }

    /// The runtime files of a dead session, removed. `Err` is a process that is still alive,
    /// reported as `session_crashed` and left in place, or a file that could not be removed.
    fn dead_files(&self) -> Result<Option<PathBuf>, Refusal> {
        if self
            .place
            .read_pid(&self.name)
            .is_some_and(place::pid_alive)
        {
            return Err(self.crashed("ended without closing"));
        }
        self.remove_dead()
    }

    /// Removes the runtime files of a session whose process has ended. `<pid>.crashed` stays.
    /// A log that holds output is kept as that file. The directory goes when nothing remains.
    fn remove_dead(&self) -> Result<Option<PathBuf>, Refusal> {
        let crash = self.preserve_crash();
        for path in [
            self.socket.clone(),
            self.place.pid(&self.name),
            self.place.log(&self.name),
            self.place.version(&self.name),
            self.place.lock(&self.name),
        ] {
            if let Err(error) = fs::remove_file(&path)
                && error.kind() != ErrorKind::NotFound
            {
                return Err(self.refuse(
                    Code::IoError,
                    format!("cannot remove {}: {error}", path.display()),
                    "",
                ));
            }
        }
        let state = self.place.state(&self.name);
        if fs::read_dir(&state).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(&state);
        }
        Ok(crash)
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
            Code::VersionMismatch,
            format!("session `{}` runs orbe {version}", self.name),
            format!("run `orbe close{}`", self.session_flag()),
        ))
    }

    /// The open that found a stale session says so, whether the open then succeeds or is refused.
    fn with_crash(
        &self,
        answered: Result<Box<Reply>, Refusal>,
        recovered: Option<Option<PathBuf>>,
    ) -> Result<Box<Reply>, Refusal> {
        let Some(path) = recovered else {
            return answered;
        };
        let note = match path {
            Some(path) => {
                format!(
                    "the previous session crashed; its last output is in {}",
                    path.display()
                )
            }
            None => "the previous session crashed".into(),
        };
        match answered {
            Ok(mut reply) => {
                reply.previous_crash = Some(note);
                Ok(reply)
            }
            Err(mut refused) => {
                refused.previous_crash = Some(note.into());
                Err(refused)
            }
        }
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

    /// Closes every session for this user, whichever version started it, and removes what a
    /// process that ended left. A process with no scene is left to end on its own.
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
        Ok(closed)
    }
}

/// One Ctrl-C handler for the process. The first press stops the command in flight. The
/// second exits 130.
fn arm_ctrlc(stream: &UnixStream) {
    if let Ok(cloned) = stream.try_clone()
        && let Ok(mut slot) = HANGUP.lock()
    {
        *slot = Some(cloned);
    }
    CTRLC.call_once(|| {
        let _ = ctrlc::set_handler(|| {
            static PRESSED: AtomicBool = AtomicBool::new(false);
            if PRESSED.swap(true, Ordering::Relaxed) {
                std::process::exit(130);
            }
            if let Ok(slot) = HANGUP.lock()
                && let Some(stream) = slot.as_ref()
            {
                let _ = stream.shutdown(Shutdown::Write);
            }
        });
    });
}

static CTRLC: Once = Once::new();
static HANGUP: Mutex<Option<UnixStream>> = Mutex::new(None);

enum ReadEnd {
    TimedOut,
    Closed,
}

/// Reads one reply line. Each wait is a second. A dead session process ends the wait. `step`
/// may run up to its wall-time budget plus slack; `play` has no such bound, and a dead process
/// still ends it.
#[expect(
    clippy::disallowed_methods,
    reason = "a read ends when its deadline passes"
)]
fn read_reply(
    stream: &UnixStream,
    deadline: Duration,
    dead: impl Fn() -> bool,
) -> Result<String, ReadEnd> {
    let until = Instant::now() + deadline;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        if line.len() as u64 >= LINE {
            return Err(ReadEnd::Closed);
        }
        let now = Instant::now();
        if now >= until {
            return Err(ReadEnd::TimedOut);
        }
        let _ = stream.set_read_timeout(Some((until - now).min(Duration::from_secs(1))));
        match reader.read_line(&mut line) {
            Ok(0) => return Err(ReadEnd::Closed),
            Ok(_) if line.ends_with('\n') => return Ok(line),
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if dead() {
                    return Err(ReadEnd::Closed);
                }
            }
            Err(_) => return Err(ReadEnd::Closed),
        }
    }
}

fn reply_deadline(ask: &Ask) -> Duration {
    let slack = Duration::from_secs(30);
    match ask {
        Ask::Run(Ok(Command::Step(_))) => daemon::STEP_BUDGET + slack,
        Ask::Run(Ok(Command::Play)) => Duration::from_secs(24 * 60 * 60),
        Ask::Run(Err(_)) => Duration::from_secs(1),
        _ => Duration::from_secs(30),
    }
}

fn read_stderr(stderr: &mut Option<ChildStderr>) -> String {
    let Some(mut stderr) = stderr.take() else {
        return String::new();
    };
    let mut buf = String::new();
    let _ = stderr.read_to_string(&mut buf);
    buf
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
