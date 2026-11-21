//! `orbe daemon --session <name>`, the session's process. One thread owns the `Live` and runs
//! commands in the order they arrive on one channel, and writes each reply itself, so every
//! command it took is answered before it exits. Each connection carries one request, and a
//! client that hangs up before its reply cancels a running `step`, or the command it queued.
//!
//! Viewers' commands, their comings and goings, and `play`'s ticks reach the same thread on the
//! same channel, so a viewer's `press` runs between two commands or two ticks, never inside a
//! `step`, and nothing needs a lock.

use std::collections::BTreeMap;
use std::io::{self, BufReader, Read, Write};
use std::ops::ControlFlow;
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
use orbe_session::{Code, Command, Data, Error, Files, Live, Pace, Reply, Sample};
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

impl Held {
    fn run(&mut self, job: &Job) -> Reply {
        if job.hung_up.load(Ordering::Relaxed) {
            return self.live.refuse(Error::new(
                Code::Cancelled,
                "the command was cancelled before it ran",
                "the session did not change",
            ));
        }
        let (cwd, command, stop) = match &job.request {
            Ok(Request {
                ask: Ask::Show { page },
                ..
            }) => return self.show(page.as_deref()),
            Ok(Request {
                cwd,
                ask: Ask::Run(Ok(command)),
                stop,
            }) => (Path::new(cwd), command, stop.as_deref()),
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
        let watchers = &mut self.watchers;
        let mut reply = self.live.run_with(command, &mut |progress| {
            if watchers.due() {
                watchers.frame(progress.sample(), progress.serial(), None);
            }
            if job.hung_up.load(Ordering::Relaxed) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
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
        if let Some(url) = &self.url {
            return self.live.report(Data::Viewer { url: url.clone() });
        }
        let page = match Page::find(page) {
            Ok(page) => page,
            Err(error) => return self.live.refuse(error),
        };
        match viewer::start(page, Arc::clone(&self.models), self.events.clone()) {
            Ok(url) => {
                self.url = Some(url.clone());
                self.live.report(Data::Viewer { url })
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

/// Runs the session. `start` names the log `open` gave this process for its stderr. Once this
/// process holds the socket, that log becomes the session's, unless the session's log already
/// holds a crash's output. A process that loses the socket to another start removes its log.
pub fn run(name: &Name, start: Option<u64>) -> ExitCode {
    let found = Place::find().and_then(|place| {
        let start = start.map(|id| place.start_log(name, id));
        Ok((place.socket(name)?, place.log(name), start))
    });
    let (socket, log, start) = match found {
        Ok(paths) => paths,
        Err((message, hint)) => {
            eprintln!("error[io_error]: {message}\n  hint: {hint}");
            return ExitCode::from(3);
        }
    };
    let listener = match bind(&socket) {
        Ok(listener) => listener,
        Err(error) => {
            let lost = matches!(
                error.kind(),
                io::ErrorKind::AddrInUse | io::ErrorKind::NotFound
            );
            if lost && let Some(start) = &start {
                let _ = std::fs::remove_file(start);
            }
            eprintln!(
                "error[io_error]: cannot listen at {}: {error}",
                socket.display()
            );
            return ExitCode::from(3);
        }
    };
    let bound = (inode(&socket), start.as_deref().and_then(inode));
    // A crash left output in the log the hint names. Leave that file until it is read or a
    // later crash replaces it, and keep this process's stderr on its start log.
    let preserve = start.is_some() && holds_output(&log);
    if let Some(start) = &start
        && !preserve
        && let Err(error) = std::fs::rename(start, &log)
    {
        if inode(&socket) == bound.0 {
            let _ = std::fs::remove_file(&socket);
        }
        eprintln!(
            "error[io_error]: cannot move the log {} to {}: {error}",
            start.display(),
            log.display()
        );
        return ExitCode::from(3);
    }
    let crash = start.as_ref().zip(bound.1).map(|(start, inode)| CrashLog {
        start,
        log: &log,
        inode,
    });
    let idle = test_duration("ORBE_TEST_IDLE_TIMEOUT").unwrap_or(IDLE);
    let (events, queue) = mpsc::channel();
    let reading = Arc::new(AtomicUsize::new(0));
    let timer = unopened_timer(events.clone(), UNOPENED.min(idle), Arc::clone(&reading));
    let mut held = Held {
        live: Live::new(name.as_str(), Files::disk()),
        scene: None,
        watchers: Watchers::default(),
        url: None,
        models: Arc::new(Mutex::new(BTreeMap::new())),
        events: events.clone(),
        ticked: None,
    };
    thread::spawn(move || accept(&listener, &events, &reading));
    let (why, unanswered) = serve(&mut held, &queue, idle, timer, crash.as_ref());
    if bound.0.is_some() && inode(&socket) == bound.0 {
        let _ = std::fs::remove_file(&socket);
    }
    if let Some(crash) = &crash {
        crash.claim_if_read();
        if inode(crash.log) == Some(crash.inode) {
            let _ = std::fs::remove_file(crash.log);
        } else if inode(crash.start) == Some(crash.inode) {
            let _ = std::fs::remove_file(crash.start);
        }
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
    ExitCode::SUCCESS
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
    crash: Option<&CrashLog>,
) -> (&'static str, Option<(Job, Reply)>) {
    let mut timer = Some(timer);
    let mut last = wall();
    loop {
        if held.next_tick().is_some_and(|at| at <= wall()) {
            held.tick();
            last = wall();
        }
        // Armed only while paused and unwatched. A deadline taken from the last tick would
        // already have passed when a busy machine delivers this thread late, and the session
        // would end in the middle of `play`.
        let quiet = (held.watchers.viewers.is_empty() && !held.live.playing()).then(|| last + idle);
        let event = match [quiet, held.next_tick()].into_iter().flatten().min() {
            Some(at) => queue.recv_timeout(at.saturating_duration_since(wall())),
            None => queue.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match event {
            Ok(Event::Job(job)) => {
                last = wall();
                if let Some(crash) = crash {
                    crash.claim_if_read();
                }
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
/// the process waits before it reads its first connection's request.
fn test_duration(name: &str) -> Option<Duration> {
    std::env::var(name)
        .ok()
        .and_then(|text| orbe_session::Duration::parse(&text).ok())
        .map(|duration| Duration::from_nanos(duration.as_ps() / 1000))
}

/// `ORBE_TEST_STOP` from the shell that ran `open`, a test seam for a stop shorter than 2 hours.
fn test_stop(text: Option<&str>) -> Result<Option<(orbe_session::Duration, String)>, Error> {
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

/// `to` from the folder `from`, both real paths, such as `../scene.jsonc`.
fn relative(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let up = std::iter::repeat_n(Component::ParentDir, from.len() - common);
    up.chain(to[common..].iter().copied()).collect()
}

/// Binds the socket. A socket file nothing answers on is one a crashed process left, and is
/// replaced. Anything else at the path stays, and the bind fails.
fn bind(socket: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(socket) {
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            let is_socket = std::fs::symlink_metadata(socket)?.file_type().is_socket();
            if !is_socket || UnixStream::connect(socket).is_ok() {
                return Err(error);
            }
            std::fs::remove_file(socket)?;
            UnixListener::bind(socket)
        }
        bound => bound,
    }
}

/// A file's identity, so the process removes only the socket it bound and the log it got.
fn inode(path: &Path) -> Option<(u64, u64)> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| (m.dev(), m.ino()))
}

/// The start log and the session log while a crash's output still occupies the session log.
struct CrashLog<'a> {
    start: &'a Path,
    log: &'a Path,
    inode: (u64, u64),
}

impl CrashLog<'_> {
    /// The crash log was read after it was written. This process's stderr already points at
    /// the start log, so that file takes the session log's name and a clean exit removes it.
    fn claim_if_read(&self) {
        if inode(self.log) == Some(self.inode) {
            return;
        }
        let Ok(meta) = std::fs::symlink_metadata(self.log) else {
            return;
        };
        let read = meta.is_file() && meta.len() > 0 && meta.accessed().ok() > meta.modified().ok();
        if read {
            let _ = std::fs::rename(self.start, self.log);
        }
    }
}

fn holds_output(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

/// Counts each accepted connection in `reading` until its request is queued or it ends.
fn accept(listener: &UnixListener, jobs: &Sender<Event>, reading: &Arc<AtomicUsize>) {
    let mut delay = test_duration("ORBE_TEST_READ_DELAY");
    for stream in listener.incoming().flatten() {
        reading.fetch_add(1, Ordering::SeqCst);
        let (jobs, reading, delay) = (jobs.clone(), Arc::clone(reading), delay.take());
        thread::spawn(move || {
            if let Some(delay) = delay {
                thread::sleep(delay);
            }
            connection(stream, &jobs);
            reading.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

fn connection(stream: UnixStream, jobs: &Sender<Event>) {
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
