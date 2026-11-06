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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use orbe_session::{Code, Command, Data, Error, Live, Reply, State};

use super::place::{Name, Place};
use super::{Request, read_line};

/// The process ends after this long with no command.
const IDLE: Duration = Duration::from_secs(60 * 60);

/// How long a reply waits for a client that does not read it.
const WRITE: Duration = Duration::from_secs(5);

/// A request, or why it did not read as one, with the connection its reply goes back on.
struct Job {
    request: Result<Request, Error>,
    hung_up: Arc<AtomicBool>,
    client: UnixStream,
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
        let (cwd, command) = match &job.request {
            Ok(Request {
                cwd,
                command: Ok(command),
            }) => (Path::new(cwd), command),
            Ok(Request {
                command: Err(error),
                ..
            })
            | Err(error) => return self.live.refuse(error.clone()),
        };
        if let Command::Open(_) = command
            && let Err(error) = std::env::set_current_dir(cwd)
        {
            return self.live.refuse(Error::new(
                Code::IoError,
                format!("cannot open from {}: {error}", cwd.display()),
                "run `orbe open` from a folder that exists",
            ));
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
    /// Whether a reply with no open scene ends the process: one to `open` or `close`, not a
    /// read that reached the process before its first `open`.
    fn ends_when_closed(&self) -> bool {
        matches!(
            self.request,
            Ok(Request {
                command: Ok(Command::Open(_) | Command::Close),
                ..
            })
        )
    }

    fn answer(self, reply: &Reply) {
        let mut wire = serde_json::to_string(reply).expect("a reply serializes");
        wire.push('\n');
        let _ = self.client.set_write_timeout(Some(WRITE));
        let _ = (&self.client).write_all(wire.as_bytes());
        let _ = self.client.shutdown(std::net::Shutdown::Both);
    }
}

pub fn run(name: &Name) -> ExitCode {
    let found = Place::find().and_then(|place| Ok((place.socket(name)?, place.log(name))));
    let (socket, log) = match found {
        Ok(paths) => paths,
        Err((message, hint)) => {
            eprintln!("error[io_error]: {message}\n  hint: {hint}");
            return ExitCode::from(3);
        }
    };
    let listener = match bind(&socket) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "error[io_error]: cannot listen at {}: {error}",
                socket.display()
            );
            return ExitCode::from(3);
        }
    };
    let bound = (inode(&socket), inode(&log));
    let (jobs, queue) = mpsc::channel();
    thread::spawn(move || accept(&listener, &jobs));
    let mut live = Live::new(name.as_str());
    if let Some(stop) = test_duration("ORBE_TEST_STOP") {
        live.stop_at(stop);
    }
    let mut held = Held { live, scene: None };
    serve(
        &mut held,
        &queue,
        test_duration("ORBE_TEST_IDLE_TIMEOUT").map_or(IDLE, std),
    );
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
    while let Ok(job) = queue.try_recv() {
        job.answer(&ending);
    }
    ExitCode::SUCCESS
}

/// Runs jobs in order until one to `open` or `close` leaves no scene open, or no job comes for
/// `idle`. A session at its stop stays, so reads still answer there.
fn serve(held: &mut Held, queue: &Receiver<Job>, idle: Duration) {
    while let Ok(job) = queue.recv_timeout(idle) {
        let ends_when_closed = job.ends_when_closed();
        let reply = held.run(&job);
        job.answer(&reply);
        if ends_when_closed && reply.state == State::Closed {
            return;
        }
    }
}

/// A duration such as `300ms` from a test seam: `ORBE_TEST_IDLE_TIMEOUT` shortens the idle
/// timeout, and `ORBE_TEST_STOP` the 2-hour stop.
fn test_duration(variable: &str) -> Option<orbe_session::Duration> {
    std::env::var(variable)
        .ok()
        .and_then(|text| orbe_session::Duration::parse(&text).ok())
}

fn std(duration: orbe_session::Duration) -> Duration {
    Duration::from_nanos(duration.as_ps() / 1000)
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

fn accept(listener: &UnixListener, jobs: &Sender<Job>) {
    for stream in listener.incoming().flatten() {
        let jobs = jobs.clone();
        thread::spawn(move || connection(stream, &jobs));
    }
}

fn connection(stream: UnixStream, jobs: &Sender<Job>) {
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
    let _ = jobs.send(Job {
        request,
        hung_up,
        client: stream,
    });
}
