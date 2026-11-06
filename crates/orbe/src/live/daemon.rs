//! `orbe daemon --session <name>`, the session's process. One thread owns the `Live` and runs
//! commands in the order they arrive on one channel, and writes each reply itself, so every
//! command it took is answered before it exits. Each connection carries one request, and a
//! client that hangs up before its reply cancels a running `step`, or the command it queued.

use std::io::{self, BufReader, Read, Write};
use std::ops::ControlFlow;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use orbe_session::{Code, Command, Data, Error, Live, Reply};

use super::place::{Name, Place};
use super::{Request, read_line};

/// The process ends after this long with no command.
const IDLE: Duration = Duration::from_secs(60 * 60);

/// A process with no scene open ends this long after it starts listening, however many
/// commands reach it, so one that no `open` reached does not wait out the hour.
const UNOPENED: Duration = Duration::from_secs(5);

/// How often a process with no scene, past `UNOPENED`, looks again for a request still being read.
const RECHECK: Duration = Duration::from_millis(20);

/// How long a reply waits for a client that does not read it.
const WRITE: Duration = Duration::from_secs(5);

/// A request, or why it did not read as one, with the connection its reply goes back on.
struct Job {
    request: Result<Request, Error>,
    hung_up: Arc<AtomicBool>,
    client: UnixStream,
}

/// What the main thread waits for: a command, or the end of the time a process with no scene
/// open may wait for one.
enum Event {
    Job(Job),
    Unopened,
}

/// The session's `Live`, and the open scene's real path with the folder `open` ran in, so a
/// client in another folder sees the scene's path from there.
struct Held {
    live: Live,
    scene: Option<(PathBuf, PathBuf)>,
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
                cwd,
                command: Ok(command),
                stop,
            }) => (Path::new(cwd), command, stop.as_deref()),
            Ok(Request {
                command: Err(error),
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
        let mut reply = self.live.run_with(command, &mut |_| {
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

/// Runs the session. `start` names the log `open` gave this process for its stderr: once this
/// process holds the socket, that log becomes the session's, and a process that loses the
/// socket to another start removes it.
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
    if let Some(start) = &start
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
    let idle = test_duration("ORBE_TEST_IDLE_TIMEOUT").unwrap_or(IDLE);
    let (events, queue) = mpsc::channel();
    let reading = Arc::new(AtomicUsize::new(0));
    let timer = unopened_timer(events.clone(), UNOPENED.min(idle), Arc::clone(&reading));
    thread::spawn(move || accept(&listener, &events, &reading));
    let mut held = Held {
        live: Live::new(name.as_str()),
        scene: None,
    };
    serve(&mut held, &queue, idle, timer);
    for (path, identity) in [(&socket, bound.0), (&log, bound.1)] {
        if identity.is_some() && inode(path) == identity {
            let _ = std::fs::remove_file(path);
        }
    }
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

/// Runs jobs in order until a `close` closes the scene, or no job comes for `idle`. Before the
/// first scene opens, every command but `open` gets `no_session` and changes nothing, and the
/// process ends `UNOPENED` after it started. A session at its stop stays, so reads still
/// answer there.
fn serve(held: &mut Held, queue: &Receiver<Event>, idle: Duration, timer: Sender<()>) {
    let mut timer = Some(timer);
    while let Ok(event) = queue.recv_timeout(idle) {
        let job = match event {
            Event::Job(job) => job,
            Event::Unopened if timer.is_some() => return,
            Event::Unopened => continue,
        };
        let reply = held.run(&job);
        job.answer(&reply);
        match reply.data {
            Data::Opened { .. } => timer = None,
            Data::Closed { .. } => return,
            _ => {}
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
