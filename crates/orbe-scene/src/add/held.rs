//! One `orbe add` at a time in a project. A run holds an exclusive OS lock on the project
//! folder itself from before recovery to after cleanup, and a second run does not wait for it.
//! Locking the folder writes nothing, so `--diff` works in a project it cannot write, and no
//! lock file can be deleted, linked elsewhere or left behind.

use std::fs::{File, TryLockError};
use std::path::Path;

use super::{RunError, io_at};

/// The held lock. The OS releases it when the run ends, killed or not.
pub(super) struct Held {
    _folder: File,
}

pub(super) enum Hold {
    Busy,
    Io(RunError),
}

pub(super) fn hold(root: &Path) -> Result<Held, Hold> {
    let folder = File::open(root).map_err(|error| Hold::Io(io_at(root, error)))?;
    match folder.try_lock() {
        Ok(()) => Ok(Held { _folder: folder }),
        Err(TryLockError::WouldBlock) => Err(Hold::Busy),
        Err(TryLockError::Error(error)) => Err(Hold::Io(io_at(root, error))),
    }
}
