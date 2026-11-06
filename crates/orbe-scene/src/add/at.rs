//! Paths below the staging folder, reached through open folders. Each folder is opened from
//! the one above it and never through a link, so a link swapped in for a folder stops the walk
//! instead of leading out of the project.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags};

pub(super) struct Folder(OwnedFd);

/// What is at a name, read without following a link.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Found {
    Folder,
    File(Vec<u8>),
    /// A file this process cannot read, by its inode, size and times.
    Unreadable(String),
    Link(OsString),
    Other,
}

const FOLDER: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

impl Folder {
    pub(super) fn open(path: &Path) -> io::Result<Folder> {
        Ok(Folder(rustix::fs::openat(
            CWD,
            path,
            FOLDER,
            Mode::empty(),
        )?))
    }

    fn sub(&self, name: &OsStr) -> io::Result<Folder> {
        Ok(Folder(rustix::fs::openat(
            &self.0,
            name,
            FOLDER,
            Mode::empty(),
        )?))
    }

    /// The folder that holds `rel`, a `/`-separated path below this one, and its last name.
    pub(super) fn parent_of<'r>(&self, rel: &'r OsStr) -> io::Result<(Folder, &'r OsStr)> {
        let mut names = rel.as_bytes().split(|byte| *byte == b'/');
        let last = names.next_back().unwrap_or_default();
        let mut folder = self.sub(OsStr::new("."))?;
        for name in names {
            folder = folder.sub(OsStr::from_bytes(name))?;
        }
        Ok((folder, OsStr::from_bytes(last)))
    }

    pub(super) fn names(&self) -> io::Result<Vec<OsString>> {
        let mut names = Vec::new();
        for entry in Dir::read_from(&self.0)? {
            let name = entry?.file_name().to_bytes().to_vec();
            if name != b"." && name != b".." {
                names.push(OsString::from_vec(name));
            }
        }
        Ok(names)
    }

    pub(super) fn find(&self, name: &OsStr) -> io::Result<Found> {
        let stat = rustix::fs::statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW)?;
        Ok(match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => Found::Folder,
            FileType::Symlink => Found::Link(OsString::from_vec(
                rustix::fs::readlinkat(&self.0, name, Vec::new())?.into_bytes(),
            )),
            FileType::RegularFile => {
                let flags = OFlags::RDONLY
                    .union(OFlags::NOFOLLOW)
                    .union(OFlags::NONBLOCK)
                    .union(OFlags::CLOEXEC);
                match rustix::fs::openat(&self.0, name, flags, Mode::empty()) {
                    Ok(fd) => {
                        let mut bytes = Vec::new();
                        File::from(fd).read_to_end(&mut bytes)?;
                        Found::File(bytes)
                    }
                    Err(rustix::io::Errno::ACCESS) => Found::Unreadable(format!(
                        "{}.{}.{}.{}.{}.{}",
                        stat.st_ino,
                        stat.st_size,
                        stat.st_mtime,
                        stat.st_mtime_nsec,
                        stat.st_ctime,
                        stat.st_ctime_nsec
                    )),
                    Err(error) => return Err(error.into()),
                }
            }
            _ => Found::Other,
        })
    }

    /// A link is removed, never followed.
    pub(super) fn remove_file(&self, name: &OsStr) -> io::Result<()> {
        Ok(rustix::fs::unlinkat(&self.0, name, AtFlags::empty())?)
    }

    pub(super) fn remove_folder(&self, name: &OsStr) -> io::Result<()> {
        Ok(rustix::fs::unlinkat(&self.0, name, AtFlags::REMOVEDIR)?)
    }

    /// Each path below this folder, `/`-separated, with what it holds, folders after what they
    /// hold, but `skip`.
    pub(super) fn walk(
        &self,
        skip: Option<&OsStr>,
    ) -> Result<Vec<(OsString, Found)>, (OsString, io::Error)> {
        let mut out = Vec::new();
        self.walk_into(&mut Vec::new(), skip, &mut out)?;
        Ok(out)
    }

    fn walk_into(
        &self,
        prefix: &mut Vec<u8>,
        skip: Option<&OsStr>,
        out: &mut Vec<(OsString, Found)>,
    ) -> Result<(), (OsString, io::Error)> {
        let at = |prefix: &[u8]| OsString::from_vec(prefix.to_vec());
        for name in self.names().map_err(|error| (at(prefix), error))? {
            let len = prefix.len();
            prefix.extend_from_slice(name.as_bytes());
            if skip.is_none_or(|skip| prefix.as_slice() != skip.as_bytes()) {
                let found = self.find(&name).map_err(|error| (at(prefix), error))?;
                if found == Found::Folder {
                    let folder = self.sub(&name).map_err(|error| (at(prefix), error))?;
                    prefix.push(b'/');
                    folder.walk_into(prefix, skip, out)?;
                    prefix.pop();
                }
                out.push((at(prefix), found));
            }
            prefix.truncate(len);
        }
        Ok(())
    }
}
