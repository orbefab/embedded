//! `orbe daemon --session <name>`, the session's process. One thread owns the `Live` and runs
//! commands in the order they arrive on one channel, and writes each reply itself, so every
//! command it took is answered before it exits. Each connection carries one request, and a
//! client that hangs up before its reply cancels a running `step`.

use std::io::{self, BufReader, Read, Write};
use std::ops::ControlFlow;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use orbe_session::{Code, Command, Error, Live, Reply, State};

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

impl Job {
    fn run(&self, live: &mut Live) -> Reply {
        let Request { cwd, command } = match &self.request {
            Ok(request) => request,
            Err(error) => return live.refuse(error.clone()),
        };
        if matches!(command, Command::Open(_))
            && let Err(error) = std::env::set_current_dir(cwd)
        {
            return live.refuse(Error::new(
                Code::IoError,
                format!("cannot open from {}: {error}", cwd.display()),
                "run `orbe open` from a folder that exists",
            ));
        }
        live.run_with(command, &mut |_| {
            if self.hung_up.load(Ordering::Relaxed) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
    }

    /// Whether a reply with no open scene ends the process: one to `open` or `close`, not a
    /// read that reached the process before its first `open`.
    fn ends_when_closed(&self) -> bool {
        matches!(
            self.request,
            Ok(Request {
                command: Command::Open(_) | Command::Close,
                ..
            })
        )
    }

    fn answer(self, live: &mut Live) -> State {
        let reply = self.run(live);
        let mut wire = serde_json::to_string(&reply).expect("a reply serializes");
        wire.push('\n');
        let _ = self.client.set_write_timeout(Some(WRITE));
        let _ = (&self.client).write_all(wire.as_bytes());
        let _ = self.client.shutdown(std::net::Shutdown::Both);
        reply.state
    }
}

pub fn run(name: &Name) -> ExitCode {
    let place = match Place::find() {
        Ok(place) => place,
        Err((message, hint)) => {
            eprintln!("error[io_error]: {message}\n  hint: {hint}");
            return ExitCode::from(3);
        }
    };
    let socket = place.socket(name);
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
    let bound = inode(&socket);
    let (jobs, queue) = mpsc::channel();
    thread::spawn(move || accept(&listener, &jobs));
    let mut live = Live::new(name.as_str());
    serve(&mut live, &queue, idle());
    if bound.is_some() && inode(&socket) == bound {
        let _ = std::fs::remove_file(&socket);
    }
    while let Ok(job) = queue.try_recv() {
        job.answer(&mut live);
    }
    let _ = std::fs::remove_file(place.log(name));
    ExitCode::SUCCESS
}

/// Runs jobs in order until one leaves no scene open, the session reaches its stop, or no job
/// comes for `idle`.
fn serve(live: &mut Live, queue: &Receiver<Job>, idle: Duration) {
    while let Ok(job) = queue.recv_timeout(idle) {
        let ends_when_closed = job.ends_when_closed();
        match job.answer(live) {
            State::Stopped => return,
            State::Closed if ends_when_closed => return,
            _ => {}
        }
    }
}

/// The idle timeout, which `ORBE_TEST_IDLE_TIMEOUT` (a duration such as `300ms`) shortens
/// for tests.
fn idle() -> Duration {
    std::env::var("ORBE_TEST_IDLE_TIMEOUT")
        .ok()
        .and_then(|text| orbe_session::Duration::parse(&text).ok())
        .map_or(IDLE, |d| Duration::from_nanos(d.as_ps() / 1000))
}

/// Binds the socket. A socket file nothing answers on is one a crashed process left, and is
/// replaced.
fn bind(socket: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(socket) {
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(socket).is_ok() {
                return Err(error);
            }
            std::fs::remove_file(socket)?;
            UnixListener::bind(socket)
        }
        bound => bound,
    }
}

/// The socket file's identity, so the process removes only the one it bound.
fn inode(socket: &Path) -> Option<(u64, u64)> {
    std::fs::symlink_metadata(socket)
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
