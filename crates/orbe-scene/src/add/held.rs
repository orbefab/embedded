//! One `orbe add` at a time in a project. A run holds an exclusive OS lock on
//! `.orbe-add.lock` at the project root from before recovery to after cleanup, and a second run
//! does not wait for it. The lock lives at the root because `parts/` may not exist yet, and
//! `--diff` must not create it.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use super::{RunError, io_at};

pub(super) const LOCK_FILE: &str = ".orbe-add.lock";

/// The held lock. Dropping it deletes the file, then releases the lock. A run that is killed
/// leaves the file, and the OS releases its lock, so the next run takes it.
pub(super) struct Held {
    file: File,
    path: PathBuf,
}

pub(super) enum Hold {
    /// Another run holds the lock on this file.
    Busy(PathBuf),
    Io(RunError),
}

pub(super) fn hold(root: &Path) -> Result<Held, Hold> {
    let path = root.join(LOCK_FILE);
    for _ in 0..16 {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| Hold::Io(io_at(&path, error)))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(Hold::Busy(path)),
            Err(TryLockError::Error(error)) => return Err(Hold::Io(io_at(&path, error))),
        }
        // The run that held it may have deleted the file between our open and our lock.
        if same_file(&file, &path) {
            return Ok(Held { file, path });
        }
    }
    Err(Hold::Busy(path))
}

impl Drop for Held {
    fn drop(&mut self) {
        forget(&self.path);
        let _ = self.file.unlock();
    }
}

#[cfg(unix)]
fn same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), std::fs::metadata(path)) {
        (Ok(held), Ok(named)) => held.dev() == named.dev() && held.ino() == named.ino(),
        _ => false,
    }
}

/// Without file ids the file is never deleted, so the open file is always the named one.
#[cfg(not(unix))]
fn same_file(_: &File, _: &Path) -> bool {
    true
}

#[cfg(unix)]
fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(not(unix))]
fn forget(_: &Path) {}
