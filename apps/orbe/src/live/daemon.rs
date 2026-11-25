//! `orbe daemon --session <name>`, the session's process. One thread owns the `Live` and runs
//! commands in the order they arrive on one channel, and writes each reply itself, so every
//! command it took is answered before it exits. Each connection carries one request. A client
//! that hangs up cancels the command it sent. A `close` from another connection stops a
//! running `step` before that `close` runs, and that step's reply says the session was closed.
//!
//! Viewers' commands, their comings and goings, and `play`'s ticks reach the same thread on the
//! same channel, so a viewer's `press` runs between two commands or two ticks, never inside a
//! `step`, and nothing needs a lock.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::ops::ControlFlow;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use orbe_session::viewer::{self as messages, Feed, TICK};
use orbe_session::{Code, Command, Data, Error, Files, Live, Pace, Reply, Sample, Step};
use orbe_session::{EXIT_IO as IO, EXIT_LOST as LOST};
use serde_json::Value;

use super::page::Page;
use super::place::{Name, Place};
use super::viewer::{self, Models, Out, Viewer};
use super::{Ask, Request, read_line};

/// The process ends after this long with no command, while it is not playing and no viewer
/// watches. The hour does not run during `play`. A tick can be late when the machine is busy,
/// and that lateness is not an idle hour.
const IDLE: Duration = Duration::from_secs(60 * 60);

/// A process with no scene open ends this long after it starts listening, however many
/// commands reach it, so one that no `open` reached does not wait out the hour.
const UNOPENED: Duration = Duration::from_secs(5);

/// How often a process with no scene, past `UNOPENED`, looks again for a request still being read.
const RECHECK: Duration = Duration::from_millis(20);

/// How long a reply waits for a client that does not read it.
const WRITE: Duration = Duration::from_secs(5);

/// How long the ending process waits for its viewers to take their close.
const FAREWELL: Duration = Duration::from_secs(1);

/// Wall-time budget for one `step`, well under the 2-minute command limit agent harnesses
/// commonly use, so a slow scene pauses and reports cleanly instead of timing out the caller.
pub(super) const STEP_BUDGET: Duration = Duration::from_secs(30);

/// A request, or why it did not read as one, with the connection its reply goes back on.
pub(super) struct Job {
    request: Result<Request, Error>,
    hung_up: Arc<AtomicBool>,
    client: UnixStream,
}

/// What the main thread waits for: a command, a viewer, or the end of the time a process with
/// no scene open may wait for one.
pub(super) enum Event {
    Job(Job),
    Viewer(Viewer),
    Unopened,
}

/// The session's `Live`, the open scene's real path with the folder `open` ran in, so a client
/// in another folder sees the scene's path from there, and the viewers that watch it.
struct Held {
    live: Live,
    scene: Option<(PathBuf, PathBuf)>,
    watchers: Watchers,
    /// The page's URL, once `show` started the listener.
    url: Option<String>,
    models: Models,
    events: Sender<Event>,
    /// While `play` runs: when the last tick began.
    ticked: Option<Instant>,
    /// Set when a connection reads `close`, so the `step` already running stops. Cleared when
    /// that `close` itself runs, which is after the step has returned.
    closing: Arc<AtomicBool>,
}

/// The viewers, and the feed of messages they were sent.
#[derive(Default)]
struct Watchers {
    viewers: BTreeMap<u64, Sender<Out>>,
    feed: Feed,
    sent_at: Option<Instant>,
}

impl Watchers {
    fn send(&mut self, out: impl Fn() -> Out) {
        self.viewers.retain(|_, viewer| viewer.send(out()).is_ok());
    }

    /// Sends each viewer every message, a frame as the latest frame.
    fn send_all(&mut self, messages: Vec<Value>) {
        for message in messages {
            let text: Arc<str> = message.to_string().into();
            if message["type"] == "frame" {
                self.sent_at = Some(wall());
                self.send(|| Out::Frame(Arc::clone(&text)));
            } else {
                self.send(|| Out::Text(Arc::clone(&text)));
            }
        }
    }

    /// Sends a frame of `sample` when it shows something new, including serial text since the
    /// previous frame.
    fn frame(&mut self, sample: Sample, serial: &str, pace: Option<Pace>) {
        if self.viewers.is_empty() {
            return;
        }
        let frames = self.feed.burst(sample, serial, pace);
        self.send_all(frames);
    }

    /// Whether a running `step` owes the viewers a frame.
    fn due(&self) -> bool {
        !self.viewers.is_empty()
            && self
                .sent_at
                .is_none_or(|at| wall().duration_since(at) >= TICK)
    }
}

/// A step `close` stopped says the session was closed. The client turns the hint into the
/// `orbe open` for this folder. A hang-up keeps the hint that the session still stands.
fn say_session_closed(reply: &mut Reply) {
    let Some(error) = &mut reply.error else {
        return;
    };
    if error.code != Code::Cancelled {
        return;
    }
    let Some(at) = error.message.strip_prefix("the step was cancelled at ") else {
        return;
    };
    let at = at.to_string();
    error.message = format!("the session was closed at {at}");
    error.hint = "run `open` to start it again".into();
}

/// A step that hit its wall-time budget stops paused where it got, reports the reached time,
/// the asked time and the speed relative to real time, and hints the command for the rest.
fn say_step_timeout(
    reply: &mut Reply,
    step: &Step,
    from_ps: u64,
    end_ps: u64,
    budget: Duration,
    elapsed: Duration,
) {
    let Some(error) = &mut reply.error else {
        return;
    };
    if error.code != Code::Cancelled {
        return;
    }
    let reached_ps = reply.t_ps;
    let advanced_ps = reached_ps.saturating_sub(from_ps);
    let remaining_ps = end_ps.saturating_sub(reached_ps);
    let reached = orbe_session::Duration::from_ps(reached_ps);
    let remaining = orbe_session::Duration::from_ps(remaining_ps);
    let budget_ps = u64::try_from(budget.as_nanos())
        .unwrap_or(u64::MAX)
        .saturating_mul(1000);
    let budget_shown = orbe_session::Duration::from_ps(budget_ps);
    let pace = format_pace(elapsed, advanced_ps);
    let pace_note = if pace.is_empty() {
        String::new()
    } else {
        format!(" ({pace})")
    };
    error.code = Code::StepTimeout;
    match step {
        Step::For(asked) => {
            let asked_part = if from_ps == 0 {
                format!("asked for {asked}")
            } else {
                format!(
                    "{} of the {asked} asked",
                    orbe_session::Duration::from_ps(advanced_ps)
                )
            };
            error.message = format!(
                "step stopped at t={reached} ({asked_part}) after the {budget_shown} wall-time limit{pace_note}"
            );
            error.hint = format!(
                "the session is paused at t={reached}; run `orbe step {remaining}` to step the rest"
            );
        }
        Step::Until { condition, timeout } => {
            let asked_part = if from_ps == 0 {
                format!("timeout {timeout}")
            } else {
                format!(
                    "{} of the {timeout} timeout",
                    orbe_session::Duration::from_ps(advanced_ps)
                )
            };
            let cond = shell_condition(&condition.to_string());
            error.message = format!(
                "step --until stopped at t={reached} ({asked_part}) after the {budget_shown} wall-time limit{pace_note}"
            );
            error.hint = format!(
                "the condition was not met yet and the session is paused at t={reached}; run `orbe step --until {cond} --timeout {remaining}` to keep waiting"
            );
        }
    }
}

/// How fast the scene simulated over one step relative to real time.
fn format_pace(elapsed: Duration, sim_ps: u64) -> String {
    let wall_secs = elapsed.as_secs_f64();
    let sim_secs = sim_ps as f64 / 1e12;
    if sim_secs <= 0.0 || wall_secs <= 0.0 {
        return String::new();
    }
    if wall_secs >= sim_secs {
        format!(
            "{}x slower than real time",
            format_factor(wall_secs / sim_secs)
        )
    } else {
        let faster = sim_secs / wall_secs;
        if (faster * 10.0).round() <= 10.0 {
            "at 1x real time".into()
        } else {
            format!("{}x faster than real time", format_factor(faster))
        }
    }
}

/// Rounds a speed ratio to a whole number (at 10x and up, or exact whole numbers) or one decimal.
fn format_factor(ratio: f64) -> String {
    let tenths = (ratio * 10.0).round() as u64;
    if tenths >= 100 {
        format!("{}", ratio.round() as u64)
    } else if tenths.is_multiple_of(10) {
        format!("{}", tenths / 10)
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// Quotes a condition for a shell when it holds characters a shell interprets, such as `>`, `(`
/// or `"`. Plain conditions such as `@led1 lit` stay unquoted.
fn shell_condition(text: &str) -> String {
    let needs_quotes = text.bytes().any(|b| {
        matches!(
            b,
            b'"' | b'\''
                | b'<'
                | b'>'
                | b'('
                | b')'
                | b';'
                | b'&'
                | b'|'
                | b'$'
                | b'`'
                | b'\\'
                | b'*'
                | b'?'
                | b'['
                | b']'
                | b'#'
                | b'~'
                | b'!'
        )
    });
    if !needs_quotes {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

impl Held {
    fn run(&mut self, job: &Job) -> Reply {
        if matches!(
            &job.request,
            Ok(Request {
                ask: Ask::Run(Ok(Command::Close)),
                ..
            })
        ) {
            self.closing.store(false, Ordering::Relaxed);
        }
        if job.hung_up.load(Ordering::Relaxed) {
            return self.live.refuse(Error::new(
                Code::Cancelled,
                "the command was cancelled before it ran",
                "the session did not change",
            ));
        }
        let (cwd, command, stop, step_budget) = match &job.request {
            Ok(Request {
                ask: Ask::Show { page },
                ..
            }) => return self.show(page.as_deref()),
            Ok(Request {
                cwd,
                ask: Ask::Run(Ok(command)),
                stop,
                step_budget,
            }) => (
                Path::new(cwd),
                command,
                stop.as_deref(),
                step_budget.as_deref(),
            ),
            Ok(Request {
                ask: Ask::Run(Err(error)),
                ..
            })
            | Err(error) => return self.live.refuse(error.clone()),
        };
        if let Command::Open(_) = command {
            if let Err(error) = std::env::set_current_dir(cwd) {
                return self.live.refuse(Error::new(
                    Code::IoError,
                    format!("cannot open from {}: {error}", cwd.display()),
                    "run `orbe open` from a folder that exists",
                ));
            }
            match test_stop(stop) {
                Ok(stop) => self.live.stop_at(stop),
                Err(error) => return self.live.refuse(error),
            }
        }
        let closing = Arc::clone(&self.closing);
        let watchers = &mut self.watchers;
        let mut closed = false;
        let started = matches!(command, Command::Step(_)).then(wall);
        let budget = test_step_budget(step_budget);
        let mut timed_out: Option<(u64, u64, Duration)> = None;
        let mut reply = self.live.run_with(command, &mut |progress| {
            if watchers.due() {
                watchers.frame(progress.sample(), progress.serial(), None);
            }
            // `close` ends the session. A hang-up leaves it where the step stopped.
            if closing.load(Ordering::Relaxed) {
                closed = true;
                ControlFlow::Break(())
            } else if job.hung_up.load(Ordering::Relaxed) {
                ControlFlow::Break(())
            } else if let Some(started) = started {
                let elapsed = wall().duration_since(started);
                if elapsed >= budget {
                    timed_out = Some((progress.from_ps, progress.end_ps, elapsed));
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            } else {
                ControlFlow::Continue(())
            }
        });
        if closed {
            say_session_closed(&mut reply);
        } else if let (Command::Step(step), Some((from_ps, end_ps, elapsed))) = (command, timed_out)
        {
            say_step_timeout(&mut reply, step, from_ps, end_ps, budget, elapsed);
        }
        match (command, &mut reply.data) {
            (Command::Open(scene), Data::Opened { .. }) => {
                self.scene = scene
                    .canonicalize()
                    .ok()
                    .map(|real| (cwd.to_path_buf(), real));
            }
            (Command::Close, Data::Closed { .. }) => self.scene = None,
            (_, Data::Snapshot(snapshot)) => {
                if let Some((opened_in, real)) = &self.scene
                    && opened_in != cwd
                {
                    snapshot.scene = relative(cwd, real).display().to_string();
                }
            }
            _ => {}
        }
        reply
    }

    /// Starts the listener on the first call, and gives the page's URL.
    fn show(&mut self, page: Option<&Path>) -> Reply {
        if self.live.scene().is_none() {
            return self
                .live
                .refuse(Error::new(Code::NoSession, "no scene is open", ""));
        }
        let viewers = self.watchers.viewers.len();
        if let Some(url) = &self.url {
            return self.live.report(Data::Viewer {
                url: url.clone(),
                viewers,
            });
        }
        let page = match Page::find(page) {
            Ok(page) => page,
            Err(error) => return self.live.refuse(error),
        };
        match viewer::start(page, Arc::clone(&self.models), self.events.clone()) {
            Ok(url) => {
                self.url = Some(url.clone());
                self.live.report(Data::Viewer { url, viewers })
            }
            Err(error) => self.live.refuse(Error::new(
                Code::IoError,
                format!("cannot listen for viewers on 127.0.0.1: {error}"),
                "",
            )),
        }
    }

    /// What follows a command that ran, from a client or a viewer: the models a new scene
    /// names, the play clock started or stopped, and what the viewers see of it.
    fn after(&mut self, reply: &Reply) {
        if let Data::Opened { .. } = reply.data {
            self.models_from_scene();
        }
        self.ticked = match (self.live.playing(), self.ticked) {
            (true, None) => Some(wall()),
            (true, ticked) => ticked,
            (false, _) => None,
        };
        if !self.watchers.viewers.is_empty() {
            let after = self.watchers.feed.after(&self.live, reply);
            self.watchers.send_all(after);
        }
    }

    fn frame(&mut self, pace: Option<Pace>) {
        let Held { live, watchers, .. } = self;
        if let Some(sample) = live.sample() {
            watchers.frame(sample, live.serial(), pace);
        }
    }

    /// The part models the scene message names, each read from the open scene's folder.
    fn models_from_scene(&mut self) {
        let folder = self.scene.as_ref().and_then(|(_, real)| real.parent());
        let parts = self.live.scene().map(|scene| scene["parts"].clone());
        let mut models = self.models.lock().unwrap();
        models.clear();
        let (Some(folder), Some(serde_json::Value::Object(parts))) = (folder, parts) else {
            return;
        };
        for part in parts.values() {
            if let Some(url) = part["model"].as_str() {
                models.insert(url.to_string(), folder.to_path_buf());
            }
        }
    }

    /// One `play` tick, the wall time since the last.
    fn tick(&mut self) {
        let Some(ticked) = self.ticked else { return };
        let now = wall();
        let pace = self.live.tick(now.duration_since(ticked));
        self.ticked = pace.is_some().then_some(now);
        self.frame(pace);
    }

    /// When the next tick is due, while `play` runs.
    fn next_tick(&self) -> Option<Instant> {
        self.ticked.map(|at| at + TICK)
    }

    fn viewer(&mut self, event: Viewer) {
        match event {
            Viewer::Joined(id, out) => {
                let (scene, frame): (Vec<Value>, Vec<Value>) = self
                    .watchers
                    .feed
                    .join(&self.live)
                    .into_iter()
                    .partition(|message| message["type"] == "scene");
                for message in scene {
                    let _ = out.send(Out::Text(message.to_string().into()));
                }
                self.watchers.viewers.insert(id, out);
                self.watchers.send_all(frame);
            }
            Viewer::Command(id, text) => {
                let (reply, message) = messages::answer(&mut self.live, &text);
                if let Some(viewer) = self.watchers.viewers.get(&id) {
                    let _ = viewer.send(Out::Text(message.to_string().into()));
                }
                self.after(&reply);
            }
            Viewer::Left(id) => {
                self.watchers.viewers.remove(&id);
            }
        }
    }

    /// Tells each viewer why the session ended, closes it, and waits a moment for them all.
    fn farewell(&mut self, why: &str) {
        let last: Arc<str> = messages::closed(why).to_string().into();
        let (done, heard) = mpsc::channel();
        let count = self.watchers.viewers.len();
        self.watchers
            .send(|| Out::Close(Arc::clone(&last), done.clone()));
        let until = wall() + FAREWELL;
        for _ in 0..count {
            let left = until.saturating_duration_since(wall());
            if heard.recv_timeout(left).is_err() {
                break;
            }
        }
    }
}

impl Job {
    fn answer(self, reply: &Reply) {
        let mut wire = serde_json::to_string(reply).expect("a reply serializes");
        wire.push('\n');
        let _ = self.client.set_write_timeout(Some(WRITE));
        let _ = (&self.client).write_all(wire.as_bytes());
        let _ = self.client.shutdown(std::net::Shutdown::Both);
    }
}

/// Runs the session. The process holds `<name>/lock` until it exits, writes its pid, and logs
/// to `<name>/log`. A panic writes `<name>/<pid>.crashed`. A second start that finds the lock
/// held leaves this process to serve. On the way out the socket, pid, log, version and lock
/// go; a crash file stays.
pub fn run(name: &Name) -> ExitCode {
    let place = match Place::find() {
        Ok(place) => place,
        Err((message, hint)) => {
            eprintln!("error[io_error]: {message}\n  hint: {hint}");
            return ExitCode::from(IO);
        }
    };
    if let Err((message, hint)) = place.ensure(name) {
        eprintln!("error[io_error]: {message}\n  hint: {hint}");
        return ExitCode::from(IO);
    }
    let lock_path = place.lock(name);
    let lock = match lock_session(&lock_path) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return ExitCode::from(LOST),
        Err(error) => {
            eprintln!(
                "error[io_error]: cannot lock {}: {error}",
                lock_path.display()
            );
            return ExitCode::from(IO);
        }
    };
    let log_path = place.log(name);
    let log = match open_log(&log_path) {
        Ok(log) => log,
        Err((message, hint)) => {
            eprintln!("error[io_error]: {message}\n  hint: {hint}");
            return ExitCode::from(IO);
        }
    };
    let pid = std::process::id();
    let pid_path = place.pid(name);
    if let Err(error) = std::fs::write(&pid_path, format!("{pid}\n")) {
        eprintln!(
            "error[io_error]: cannot write {}: {error}",
            pid_path.display()
        );
        return ExitCode::from(IO);
    }
    if let Err(error) = rustix::stdio::dup2_stderr(log.as_fd()) {
        eprintln!("error[io_error]: cannot open the log: {error}");
        return ExitCode::from(IO);
    }
    drop(log);
    install_crash_hook(place.crashed(name, pid), log_path.clone());

    let socket = match place.socket(name) {
        Ok(socket) => socket,
        Err((message, hint)) => {
            eprintln!("error[io_error]: {message}\n  hint: {hint}");
            return ExitCode::from(IO);
        }
    };
    let listener = match bind(&socket) {
        Ok(listener) => listener,
        Err(error) => {
            let lost = error.kind() == io::ErrorKind::AddrInUse;
            eprintln!(
                "error[io_error]: cannot listen at {}: {error}",
                socket.display()
            );
            return ExitCode::from(if lost { LOST } else { IO });
        }
    };
    let version = place.version(name);
    if let Err(error) = std::fs::write(&version, format!("{}\n", env!("CARGO_PKG_VERSION"))) {
        let _ = std::fs::remove_file(&socket);
        eprintln!(
            "error[io_error]: cannot write {}: {error}",
            version.display()
        );
        return ExitCode::from(IO);
    }
    let bound = inode(&socket);
    let version_inode = inode(&version);
    let log_inode = inode(&log_path);
    let pid_inode = inode(&pid_path);
    #[cfg(debug_assertions)]
    let idle = test_duration("ORBE_TEST_IDLE_TIMEOUT").unwrap_or(IDLE);
    #[cfg(not(debug_assertions))]
    let idle = IDLE;
    let (events, queue) = mpsc::channel();
    let reading = Arc::new(AtomicUsize::new(0));
    let closing = Arc::new(AtomicBool::new(false));
    let timer = unopened_timer(events.clone(), UNOPENED.min(idle), Arc::clone(&reading));
    let mut held = Held {
        live: Live::new(name.as_str(), Files::disk()),
        scene: None,
        watchers: Watchers::default(),
        url: None,
        models: Arc::new(Mutex::new(BTreeMap::new())),
        events: events.clone(),
        ticked: None,
        closing: Arc::clone(&closing),
    };
    thread::spawn(move || accept(listener, &events, &reading, &closing));
    let (why, unanswered) = serve(&mut held, &queue, idle, timer);
    if bound.is_some() && inode(&socket) == bound {
        let _ = std::fs::remove_file(&socket);
    }
    if version_inode.is_some() && inode(&version) == version_inode {
        let _ = std::fs::remove_file(&version);
    }
    if log_inode.is_some() && inode(&log_path) == log_inode {
        let _ = std::fs::remove_file(&log_path);
    }
    if pid_inode.is_some() && inode(&pid_path) == pid_inode {
        let _ = std::fs::remove_file(&pid_path);
    }
    if let Some((job, reply)) = unanswered {
        job.answer(&reply);
    }
    held.farewell(why);
    let ending = held.live.refuse(Error::new(
        Code::NoSession,
        format!("session `{name}` has ended"),
        "",
    ));
    while let Ok(event) = queue.try_recv() {
        if let Event::Job(job) = event {
            job.answer(&ending);
        }
    }
    let _ = std::fs::remove_file(&lock_path);
    drop(lock);
    remove_state_if_empty(&place.state(name));
    ExitCode::SUCCESS
}

/// Exclusive hold of the session, for as long as `lock` is open. A lock another process holds
/// is [`io::ErrorKind::WouldBlock`].
fn lock_session(path: &Path) -> io::Result<OwnedFd> {
    use rustix::fs::{FlockOperation, Mode, OFlags};
    let flags = OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let fd = rustix::fs::open(path, flags, Mode::RUSR | Mode::WUSR).map_err(io::Error::from)?;
    match rustix::fs::flock(&fd, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(fd),
        Err(rustix::io::Errno::WOULDBLOCK) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "the session lock is held",
        )),
        Err(errno) => Err(io::Error::from(errno)),
    }
}

/// The session log, created or truncated, and never a symlink or anything but a file.
fn open_log(path: &Path) -> Result<File, (String, String)> {
    use rustix::fs::{FileType, Mode, OFlags};
    let shown = path.display();
    let flags = OFlags::WRONLY
        | OFlags::CREATE
        | OFlags::TRUNC
        | OFlags::CLOEXEC
        | OFlags::NOFOLLOW
        | OFlags::NONBLOCK;
    let fd = match rustix::fs::open(path, flags, Mode::RUSR | Mode::WUSR) {
        Ok(fd) => fd,
        Err(errno) => {
            let (what, remove) = match errno {
                rustix::io::Errno::LOOP => ("a symlink", "rm"),
                rustix::io::Errno::ISDIR => ("a folder", "rm -r"),
                rustix::io::Errno::NXIO => ("a special file", "rm"),
                _ => {
                    return Err((
                        format!("cannot open the log {shown}: {errno}"),
                        String::new(),
                    ));
                }
            };
            return Err((
                format!("{shown} is {what}, not the log of this session"),
                format!("remove it with `{remove} {shown}`"),
            ));
        }
    };
    let stat = rustix::fs::fstat(&fd).map_err(|errno| {
        (
            format!("cannot open the log {shown}: {errno}"),
            String::new(),
        )
    })?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err((
            format!("{shown} is a special file, not the log of this session"),
            format!("remove it with `rm {shown}`"),
        ));
    }
    Ok(File::from(fd))
}

/// A panic leaves `<pid>.crashed`, the panic text and the log. A broken pipe is a client that
/// went away, and exits 0.
fn install_crash_hook(crash: PathBuf, log: PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        if super::broken_pipe(info) {
            std::process::exit(0);
        }
        let mut text = format!("panic: {info}\n");
        if let Ok(bytes) = std::fs::read(&log) {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
        let _ = std::fs::write(&crash, text);
        eprintln!("{info}");
        std::process::exit(101);
    }));
}

fn remove_state_if_empty(state: &Path) {
    let Ok(mut entries) = std::fs::read_dir(state) else {
        return;
    };
    if entries.next().is_none() {
        let _ = std::fs::remove_dir(state);
    }
}

/// Runs jobs, viewers' commands and `play`'s ticks in order until a `close` closes the scene,
/// or nothing comes for `idle` while it is not playing and no viewer watches. Before the first
/// scene opens, every command but `open` gets `no_session` and changes nothing, and the process
/// ends `UNOPENED` after it started. An `open` that fails while no scene is open ends it at
/// once, and the socket is removed before the client reads the refusal, so a retry starts
/// clean. A session at its stop stays, so reads still answer there.
/// Returns why it ended, for the viewers, and the `close` or the failed `open` that ended it,
/// still unanswered.
fn serve(
    held: &mut Held,
    queue: &Receiver<Event>,
    idle: Duration,
    timer: Sender<()>,
) -> (&'static str, Option<(Job, Reply)>) {
    let mut timer = Some(timer);
    let mut last = wall();
    loop {
        if held.next_tick().is_some_and(|at| at <= wall()) {
            held.tick();
            last = wall();
        }
        // Armed while paused, including while a viewer is connected. A tab left open is not a
        // command. A deadline taken from the last tick would already have passed when a busy
        // machine delivers this thread late, and the session would end in the middle of `play`.
        let quiet = (!held.live.playing()).then(|| last + idle);
        let event = match [quiet, held.next_tick()].into_iter().flatten().min() {
            Some(at) => queue.recv_timeout(at.saturating_duration_since(wall())),
            None => queue.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match event {
            Ok(Event::Job(job)) => {
                last = wall();
                let reply = held.run(&job);
                if let Data::Closed { .. } = reply.data {
                    return ("`orbe close` ended the session", Some((job, reply)));
                }
                // The socket goes before the client reads this refusal. A retry then finds
                // nothing, as it would have on the first try. A scene that was already open
                // stays: a refused open leaves that session as it was.
                if reply.error.is_some()
                    && held.live.scene().is_none()
                    && matches!(
                        &job.request,
                        Ok(Request {
                            ask: Ask::Run(Ok(Command::Open(_))),
                            ..
                        })
                    )
                {
                    return ("no scene was opened", Some((job, reply)));
                }
                job.answer(&reply);
                held.after(&reply);
                if let Data::Opened { .. } = reply.data {
                    timer = None;
                }
            }
            Ok(Event::Viewer(viewer)) => {
                last = wall();
                held.viewer(viewer);
            }
            Ok(Event::Unopened) if timer.is_some() => return ("no scene was opened", None),
            Ok(Event::Unopened) => {}
            Err(RecvTimeoutError::Timeout) => {
                if quiet.is_some_and(|at| at <= wall()) {
                    return ("the session ended after an hour with no command", None);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return ("the session ended", None),
        }
    }
}

/// Sends `Unopened` once `wait` has passed and no accepted connection is still being read,
/// unless the first scene opens before then and drops the sender this returns.
fn unopened_timer(events: Sender<Event>, wait: Duration, reading: Arc<AtomicUsize>) -> Sender<()> {
    let (timer, opened) = mpsc::channel();
    thread::spawn(move || {
        let mut wait = wait;
        while opened.recv_timeout(wait) == Err(RecvTimeoutError::Timeout) {
            if reading.load(Ordering::SeqCst) == 0 {
                let _ = events.send(Event::Unopened);
                return;
            }
            wait = RECHECK;
        }
    });
    timer
}

/// `ORBE_TEST_IDLE_TIMEOUT`, which also caps `UNOPENED`, and `ORBE_TEST_READ_DELAY`, how long
/// the process waits before it reads its first connection's request. A release build has
/// neither switch.
#[cfg(debug_assertions)]
fn test_duration(name: &str) -> Option<Duration> {
    std::env::var(name)
        .ok()
        .and_then(|text| orbe_session::Duration::parse(&text).ok())
        .map(|duration| Duration::from_nanos(duration.as_ps() / 1000))
}

/// `ORBE_TEST_STEP_BUDGET` from the shell that ran `step`, a test seam for a wall-time budget
/// shorter than [`STEP_BUDGET`]. A release build ignores it.
fn test_step_budget(text: Option<&str>) -> Duration {
    #[cfg(not(debug_assertions))]
    {
        let _ = text;
        STEP_BUDGET
    }
    #[cfg(debug_assertions)]
    {
        text.and_then(|value| orbe_session::Duration::parse(value).ok())
            .map(|duration| Duration::from_nanos(duration.as_ps() / 1000))
            .unwrap_or(STEP_BUDGET)
    }
}

/// `ORBE_TEST_STOP` from the shell that ran `open`, a test seam for a stop shorter than 2 hours.
/// A release build ignores it.
fn test_stop(text: Option<&str>) -> Result<Option<(orbe_session::Duration, String)>, Error> {
    #[cfg(not(debug_assertions))]
    {
        let _ = text;
        return Ok(None);
    }
    #[cfg(debug_assertions)]
    {
        let Some(text) = text else {
            return Ok(None);
        };
        let stop = orbe_session::Duration::parse(text).map_err(|reason| {
            Error::new(
                Code::Usage,
                format!("ORBE_TEST_STOP={text} is not a duration: {reason}"),
                "set ORBE_TEST_STOP to a duration such as 50ms, or unset it",
            )
        })?;
        Ok(Some((stop, "ORBE_TEST_STOP".into())))
    }
}

/// `to` from the folder `from`, both real paths, such as `../scene.jsonc`.
fn relative(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let up = std::iter::repeat_n(Component::ParentDir, from.len() - common);
    up.chain(to[common..].iter().copied()).collect()
}

/// Binds the socket. The caller holds the session lock, so a socket file left by a process
/// that ended is this start's to replace. A socket that still answers is another start's.
fn bind(socket: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(socket) {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => replace_dead(socket, error),
        Err(error) => Err(error),
    }
}

/// Replaces a socket nothing answers on. A file that is already gone is bound. A socket that
/// answers, or anything that is not a socket, stays.
fn replace_dead(socket: &Path, busy: io::Error) -> io::Result<UnixListener> {
    match std::fs::symlink_metadata(socket) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => UnixListener::bind(socket),
        Err(error) => Err(error),
        Ok(meta) if !meta.file_type().is_socket() || UnixStream::connect(socket).is_ok() => {
            Err(busy)
        }
        Ok(_) => {
            if let Err(error) = std::fs::remove_file(socket)
                && error.kind() != io::ErrorKind::NotFound
            {
                return Err(error);
            }
            UnixListener::bind(socket)
        }
    }
}

/// A file's identity, so the process removes only the socket it bound and the log it got.
fn inode(path: &Path) -> Option<(u64, u64)> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| (m.dev(), m.ino()))
}

/// Counts each accepted connection in `reading` until its request is queued or it ends.
/// `EMFILE` and `ENFILE` wait and try again. Closing the listener ends the loop.
fn accept(
    listener: UnixListener,
    jobs: &Sender<Event>,
    reading: &Arc<AtomicUsize>,
    closing: &Arc<AtomicBool>,
) {
    #[cfg(debug_assertions)]
    let mut delay = test_duration("ORBE_TEST_READ_DELAY");
    #[cfg(not(debug_assertions))]
    let mut delay: Option<Duration> = None;
    let mut backoff = Duration::from_millis(20);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                backoff = Duration::from_millis(20);
                reading.fetch_add(1, Ordering::SeqCst);
                let (jobs, reading, delay) = (jobs.clone(), Arc::clone(reading), delay.take());
                let closing = Arc::clone(closing);
                thread::spawn(move || {
                    if let Some(delay) = delay {
                        thread::sleep(delay);
                    }
                    connection(stream, &jobs, &closing);
                    reading.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if too_many_files(&error) => {
                thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_millis(200));
            }
            Err(_) => break,
        }
    }
}

fn too_many_files(error: &io::Error) -> bool {
    let Some(code) = error.raw_os_error() else {
        return false;
    };
    matches!(
        rustix::io::Errno::from_raw_os_error(code),
        rustix::io::Errno::MFILE | rustix::io::Errno::NFILE
    )
}

fn connection(stream: UnixStream, jobs: &Sender<Event>, closing: &AtomicBool) {
    let Some(line) = read_line(&mut BufReader::new(&stream)) else {
        return;
    };
    let request = serde_json::from_str::<Request>(&line).map_err(|error| {
        Error::new(
            Code::Usage,
            format!("the session process could not read the request: {error}"),
            "run the `orbe` that started the session",
        )
    });
    if matches!(
        &request,
        Ok(Request {
            ask: Ask::Run(Ok(Command::Close)),
            ..
        })
    ) {
        closing.store(true, Ordering::Relaxed);
    }
    let hung_up = Arc::new(AtomicBool::new(false));
    if let Ok(watched) = stream.try_clone() {
        let hung_up = Arc::clone(&hung_up);
        thread::spawn(move || {
            let mut rest = [0; 64];
            while matches!((&watched).read(&mut rest), Ok(n) if n > 0) {}
            hung_up.store(true, Ordering::Relaxed);
        });
    }
    let _ = jobs.send(Event::Job(Job {
        request,
        hung_up,
        client: stream,
    }));
}

/// The wall clock, which paces `play`, spaces frames and times the waits. It decides when time
/// moves, never what a reply at a given time says.
#[expect(clippy::disallowed_methods, reason = "`play` runs on the wall clock")]
pub(super) fn wall() -> Instant {
    Instant::now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pace_rounds_to_whole_or_one_decimal() {
        assert_eq!(
            format_pace(Duration::from_secs(30), 937_500_000_000),
            "32x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 1_157_000_000_000),
            "26x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 20_000_000_000_000),
            "1.5x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 15_000_000_000_000),
            "2x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 3_010_000_000_000),
            "10x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 60_000_000_000_000),
            "2x faster than real time"
        );
    }

    #[test]
    fn shell_condition_quotes_only_when_needed() {
        assert_eq!(shell_condition("@led1 lit"), "@led1 lit");
        assert_eq!(shell_condition("no diagnostics"), "no diagnostics");
        assert_eq!(shell_condition("@led1.i > 5mA"), "'@led1.i > 5mA'");
        assert_eq!(
            shell_condition("serial contains \"ready\""),
            "'serial contains \"ready\"'"
        );
    }

    #[test]
    fn client_deadline_exceeds_step_budget() {
        let step = Ask::Run(Ok(Command::Step(Step::For(
            orbe_session::Duration::from_ms(1),
        ))));
        assert!(super::super::reply_deadline(&step) > STEP_BUDGET);
    }
}
