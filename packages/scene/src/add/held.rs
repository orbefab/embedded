//! One `orbe add` at a time in a project. A run holds an exclusive OS lock on the project
//! folder itself from before recovery to after cleanup, and a second run does not wait for it.
//! Locking the folder writes nothing, and no lock file can be deleted, linked elsewhere or
//! left behind.

use std::fs::{File, TryLockError};
use std::path::Path;

use super::{RunError, io_at};

/// The held lock. The OS releases it when the run ends, killed or not.
pub(super) struct Held {
    _folder: File,
    #[cfg(test)]
    _open: forks::Open,
}

pub(super) enum Hold {
    Busy,
    Io(RunError),
}

pub(super) fn hold(root: &Path) -> Result<Held, Hold> {
    #[cfg(test)]
    let open = forks::open();
    let folder = File::open(root).map_err(|error| Hold::Io(io_at(root, error)))?;
    match folder.try_lock() {
        Ok(()) => Ok(Held {
            _folder: folder,
            #[cfg(test)]
            _open: open,
        }),
        Err(TryLockError::WouldBlock) => Err(Hold::Busy),
        Err(TryLockError::Error(error)) => Err(Hold::Io(io_at(root, error))),
    }
}

/// A child forked while a lock is held shares the locked file until it execs, so a test that
/// held the lock and let it go finds it still held. Tests fork only while no lock is open.
#[cfg(test)]
pub(super) mod forks {
    use std::sync::{Condvar, Mutex};

    static OPEN: Mutex<usize> = Mutex::new(0);
    static CLOSED: Condvar = Condvar::new();

    pub(in crate::add) struct Open;

    pub(super) fn open() -> Open {
        *OPEN.lock().unwrap() += 1;
        Open
    }

    impl Drop for Open {
        fn drop(&mut self) {
            *OPEN.lock().unwrap() -= 1;
            CLOSED.notify_all();
        }
    }

    /// Runs `fork` once no lock is open, and keeps new locks from opening until it returns.
    pub(in crate::add) fn quiet<T>(fork: impl FnOnce() -> T) -> T {
        let open = CLOSED
            .wait_while(OPEN.lock().unwrap(), |open| *open > 0)
            .unwrap();
        let out = fork();
        drop(open);
        out
    }
}
