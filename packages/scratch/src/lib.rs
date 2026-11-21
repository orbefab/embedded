//! The temp folder the repo's tests use. It is removed when the [`Scratch`] drops, which a
//! panicking test does too, and a folder a test locked is opened first so the removal reaches
//! inside it.

use std::ops::Deref;
use std::path::Path;

/// A new, empty folder under the system temp folder, removed with everything in it on drop.
pub struct Scratch(tempfile::TempDir);

impl Scratch {
    pub fn new() -> Scratch {
        Scratch(
            tempfile::Builder::new()
                .prefix("orbe-")
                .tempdir()
                .expect("a temp folder"),
        )
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Default for Scratch {
    fn default() -> Scratch {
        Scratch::new()
    }
}

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.path()
    }
}

impl Drop for Scratch {
    /// Gives every folder inside back its owner's read, write and search bits, so the
    /// `TempDir` dropped after this can list and empty it. A file needs none: removing one
    /// takes only its folder's bits.
    fn drop(&mut self) {
        #[cfg(unix)]
        open(self.path());
    }
}

#[cfg(unix)]
fn open(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            open(&entry.path());
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_locked_tree_is_removed_on_drop() {
        let scratch = Scratch::new();
        let root = scratch.to_path_buf();
        let inner = root.join("a/b");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("f"), "x").unwrap();
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o000)).unwrap();
        fs::set_permissions(root.join("a"), fs::Permissions::from_mode(0o500)).unwrap();
        drop(scratch);
        assert!(!root.exists());
    }

    #[test]
    fn a_panicking_test_leaves_no_folder() {
        let mut root = None;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let scratch = Scratch::new();
            root = Some(scratch.to_path_buf());
            panic!("the test fails");
        }));
        assert!(caught.is_err());
        assert!(!root.unwrap().exists());
    }
}
