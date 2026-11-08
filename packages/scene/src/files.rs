//! Where the reader reads a project from: the disk, or a set of files held in memory, such as a
//! scene and its parts handed to a browser.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// The files a scene is read from.
#[derive(Clone, Debug)]
pub struct Files(Store);

#[derive(Clone, Debug)]
enum Store {
    Disk,
    /// File bytes by path, each path in [`normal`] form.
    Memory(BTreeMap<PathBuf, Vec<u8>>),
}

/// What is at a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Dir,
    Link,
    Other,
}

impl Files {
    pub fn disk() -> Files {
        Files(Store::Disk)
    }

    /// Files by path. A path is read as written, with `.` dropped and `..` taking off the part
    /// before it, and every folder above a file holds it. There are no links.
    pub fn memory<P: AsRef<Path>>(files: impl IntoIterator<Item = (P, Vec<u8>)>) -> Files {
        Files(Store::Memory(
            files
                .into_iter()
                .map(|(path, bytes)| (normal(path.as_ref()), bytes))
                .collect(),
        ))
    }

    pub fn is_file(&self, path: &Path) -> bool {
        self.kind(path) == Some(Kind::File)
    }

    pub(crate) fn is_dir(&self, path: &Path) -> bool {
        self.kind(path) == Some(Kind::Dir)
    }

    /// What is at `path`, through any links.
    fn kind(&self, path: &Path) -> Option<Kind> {
        match &self.0 {
            Store::Disk => fs::metadata(path).ok().map(|meta| disk_kind(&meta)),
            Store::Memory(files) => memory_kind(files, path),
        }
    }

    /// What is at `path` itself, a link not followed.
    pub(crate) fn link_kind(&self, path: &Path) -> Option<Kind> {
        match &self.0 {
            Store::Disk => fs::symlink_metadata(path).ok().map(|meta| {
                if meta.file_type().is_symlink() {
                    Kind::Link
                } else {
                    disk_kind(&meta)
                }
            }),
            Store::Memory(files) => memory_kind(files, path),
        }
    }

    pub(crate) fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match &self.0 {
            Store::Disk => fs::read(path),
            Store::Memory(files) => match files.get(&normal(path)) {
                Some(bytes) => Ok(bytes.clone()),
                None if memory_kind(files, path) == Some(Kind::Dir) => {
                    Err(io::ErrorKind::IsADirectory.into())
                }
                None => Err(io::ErrorKind::NotFound.into()),
            },
        }
    }

    pub(crate) fn read_to_string(&self, path: &Path) -> io::Result<String> {
        match &self.0 {
            Store::Disk => fs::read_to_string(path),
            Store::Memory(_) => String::from_utf8(self.read(path)?).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stream did not contain valid UTF-8",
                )
            }),
        }
    }

    /// The paths of what a folder holds, in no set order.
    pub(crate) fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        match &self.0 {
            Store::Disk => fs::read_dir(dir)?
                .map(|entry| entry.map(|e| e.path()))
                .collect(),
            Store::Memory(files) => {
                if memory_kind(files, dir) != Some(Kind::Dir) {
                    return Err(io::ErrorKind::NotFound.into());
                }
                let dir = normal(dir);
                let names: BTreeSet<_> = files
                    .keys()
                    .filter_map(|path| path.strip_prefix(&dir).ok()?.components().next())
                    .collect();
                Ok(names.into_iter().map(|name| dir.join(name)).collect())
            }
        }
    }

    /// The path with every link resolved, and with no `.` or `..`. An error when nothing is
    /// there.
    pub(crate) fn real(&self, path: &Path) -> io::Result<PathBuf> {
        match &self.0 {
            Store::Disk => path.canonicalize(),
            Store::Memory(files) => match memory_kind(files, path) {
                Some(_) => Ok(normal(path)),
                None => Err(io::ErrorKind::NotFound.into()),
            },
        }
    }
}

fn disk_kind(meta: &fs::Metadata) -> Kind {
    if meta.is_file() {
        Kind::File
    } else if meta.is_dir() {
        Kind::Dir
    } else {
        Kind::Other
    }
}

/// Paths sort part by part, so the paths below a folder come right after it, and the first path
/// from `path` on says whether it is a file, a folder or nothing.
fn memory_kind(files: &BTreeMap<PathBuf, Vec<u8>>, path: &Path) -> Option<Kind> {
    let path = normal(path);
    let (first, _) = files.range(path.clone()..).next()?;
    if *first == path {
        Some(Kind::File)
    } else {
        first.starts_with(&path).then_some(Kind::Dir)
    }
}

fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            part => out.push(part),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> Files {
        Files::memory([
            ("p/scene.jsonc", b"{}".to_vec()),
            ("./p/parts/led/part.jsonc", b"{}".to_vec()),
            ("p/parts/led/led.glb", vec![0xff]),
            ("p/parts-b", Vec::new()),
        ])
    }

    #[test]
    fn a_memory_set_has_the_folders_above_its_files() {
        let files = set();
        assert_eq!(files.kind(Path::new("")), Some(Kind::Dir));
        assert_eq!(files.kind(Path::new("p/parts")), Some(Kind::Dir));
        assert_eq!(
            files.kind(Path::new("p/parts/../parts-b")),
            Some(Kind::File)
        );
        assert_eq!(files.kind(Path::new("p/part")), None);
        assert!(files.is_file(Path::new("p/./parts/led/part.jsonc")));
        assert_eq!(
            files.list(Path::new("p")).unwrap(),
            [
                Path::new("p/parts"),
                Path::new("p/parts-b"),
                Path::new("p/scene.jsonc")
            ]
        );
        assert_eq!(
            files.real(Path::new("p/parts/x/../led")).unwrap(),
            Path::new("p/parts/led")
        );
        assert!(files.real(Path::new("p/parts/x")).is_err());
        assert_eq!(
            files.read(Path::new("p/parts")).unwrap_err().kind(),
            io::ErrorKind::IsADirectory
        );
        assert_eq!(
            files
                .read_to_string(Path::new("p/parts/led/led.glb"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn a_parent_past_the_start_stays() {
        assert_eq!(normal(Path::new("a/../../b/./c")), Path::new("../b/c"));
        assert_eq!(normal(Path::new("/a/../b")), Path::new("/b"));
    }
}
