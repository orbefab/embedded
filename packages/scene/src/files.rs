//! Where the reader reads a project from: the disk, or a set of files held in memory, such as a
//! scene and its parts handed to a browser. A Block reads its parts from a catalog, the one built
//! into this binary unless the files name another.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::catalog;

/// The files a scene is read from, and the catalog a Block among them reads.
#[derive(Clone, Debug)]
pub struct Files {
    store: Store,
    catalog: catalog::Source,
}

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
        Files {
            store: Store::Disk,
            catalog: catalog::Source::Builtin,
        }
    }

    /// Files by path. A stored path drops `.` and applies `..` lexically. A read applies `..`
    /// only through a directory the set contains, and a trailing separator names a directory.
    /// Every folder above a file holds it. There are no links.
    pub fn memory<P: AsRef<Path>>(files: impl IntoIterator<Item = (P, Vec<u8>)>) -> Files {
        Files {
            store: Store::Memory(
                files
                    .into_iter()
                    .map(|(path, bytes)| (normal(path.as_ref()), bytes))
                    .collect(),
            ),
            catalog: catalog::Source::Builtin,
        }
    }

    /// These files, with a Block's parts read from `parts` in place of the catalog built into
    /// this binary. Each key is a part's id and a path in its folder, such as
    /// `led-5mm/part.jsonc`.
    pub fn with_catalog(self, parts: impl IntoIterator<Item = (String, Vec<u8>)>) -> Files {
        Files {
            catalog: catalog::Source::Given(Arc::new(parts.into_iter().collect())),
            ..self
        }
    }

    pub(crate) fn catalog(&self) -> &catalog::Source {
        &self.catalog
    }

    pub fn is_file(&self, path: &Path) -> bool {
        self.kind(path) == Some(Kind::File)
    }

    pub(crate) fn is_dir(&self, path: &Path) -> bool {
        self.kind(path) == Some(Kind::Dir)
    }

    /// Whether reads come from the real filesystem. A hint that plans an add reads folders
    /// the same way `orbe add` does, which a memory set cannot. Planning an add is unix-only.
    #[cfg(unix)]
    pub(crate) fn is_disk(&self) -> bool {
        matches!(self.store, Store::Disk)
    }

    /// What is at `path`, through any links.
    fn kind(&self, path: &Path) -> Option<Kind> {
        match &self.store {
            Store::Disk => fs::metadata(path).ok().map(|meta| disk_kind(&meta)),
            Store::Memory(files) => memory_kind(files, path),
        }
    }

    /// What is at `path` itself, a link not followed.
    pub(crate) fn link_kind(&self, path: &Path) -> Option<Kind> {
        match &self.store {
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
        match &self.store {
            Store::Disk => fs::read(path),
            Store::Memory(files) => {
                let path = resolve(files, path)?;
                match files.get(&path) {
                    Some(bytes) => Ok(bytes.clone()),
                    None if lookup_exact(files, &path) == Some(Kind::Dir) => {
                        Err(io::ErrorKind::IsADirectory.into())
                    }
                    None => Err(io::ErrorKind::NotFound.into()),
                }
            }
        }
    }

    pub(crate) fn read_to_string(&self, path: &Path) -> io::Result<String> {
        match &self.store {
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
        match &self.store {
            Store::Disk => fs::read_dir(dir)?
                .map(|entry| entry.map(|e| e.path()))
                .collect(),
            Store::Memory(files) => {
                let dir = resolve(files, dir)?;
                match lookup_exact(files, &dir) {
                    Some(Kind::Dir) => {}
                    Some(_) => return Err(io::ErrorKind::NotADirectory.into()),
                    None => return Err(io::ErrorKind::NotFound.into()),
                }
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
        match &self.store {
            Store::Disk => path.canonicalize(),
            Store::Memory(files) => {
                let path = resolve(files, path)?;
                match lookup_exact(files, &path) {
                    Some(_) => Ok(path),
                    None => Err(io::ErrorKind::NotFound.into()),
                }
            }
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

fn memory_kind(files: &BTreeMap<PathBuf, Vec<u8>>, path: &Path) -> Option<Kind> {
    lookup_exact(files, &resolve(files, path).ok()?)
}

/// The file or folder at `path` after `.` and `..`, or the error disk would return.
/// `..` drops the name before it only when that name is a directory in `files`.
fn resolve(files: &BTreeMap<PathBuf, Vec<u8>>, path: &Path) -> io::Result<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {
                if !out.as_os_str().is_empty() {
                    require_dir(files, &out)?;
                }
            }
            Component::ParentDir => out = parent_dir(files, out)?,
            part => out = descend(files, out, part)?,
        }
    }
    if ends_with_separator(path) {
        require_dir(files, &out)?;
    }
    Ok(out)
}

/// Paths sort part by part, so the paths below a folder come right after it, and the first path
/// from `path` on says whether it is a file, a folder or nothing.
fn lookup_exact(files: &BTreeMap<PathBuf, Vec<u8>>, path: &Path) -> Option<Kind> {
    if path.as_os_str().is_empty() {
        return (!files.is_empty()).then_some(Kind::Dir);
    }
    let (first, _) = files.range(path.to_path_buf()..).next()?;
    if first == path {
        Some(Kind::File)
    } else {
        first.starts_with(path).then_some(Kind::Dir)
    }
}

fn require_dir(files: &BTreeMap<PathBuf, Vec<u8>>, path: &Path) -> io::Result<()> {
    match lookup_exact(files, path) {
        Some(Kind::Dir) => Ok(()),
        Some(_) => Err(io::ErrorKind::NotADirectory.into()),
        None => Err(io::ErrorKind::NotFound.into()),
    }
}

fn descend(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    mut out: PathBuf,
    part: Component<'_>,
) -> io::Result<PathBuf> {
    if !out.as_os_str().is_empty() {
        require_dir(files, &out)?;
    }
    out.push(part);
    Ok(out)
}

fn parent_dir(files: &BTreeMap<PathBuf, Vec<u8>>, mut out: PathBuf) -> io::Result<PathBuf> {
    if matches!(out.components().next_back(), Some(Component::Normal(_))) {
        require_dir(files, &out)?;
        out.pop();
        Ok(out)
    } else {
        // Above the view, or at `/`, there is no directory here to drop.
        out.push("..");
        Ok(out)
    }
}

fn ends_with_separator(path: &Path) -> bool {
    match path.as_os_str().as_encoded_bytes().last() {
        Some(b'/') => true,
        Some(b'\\') if std::path::is_separator('\\') => true,
        _ => false,
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
            files.real(Path::new("p/parts/led/../led")).unwrap(),
            Path::new("p/parts/led")
        );
        assert_eq!(
            files
                .real(Path::new("p/parts/x/../led"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
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

    /// `missing/../a` is missing, and a trailing slash on a file is not a directory.
    /// Disk and memory report the same kind, and a `..` through a real directory still reads.
    #[test]
    fn memory_matches_disk_on_a_missing_parent_and_a_trailing_slash() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("a"), b"A").unwrap();
        fs::write(root.join("sub").join("b"), b"B").unwrap();
        let disk = Files::disk();
        let memory = Files::memory([
            (root.join("a"), b"A".to_vec()),
            (root.join("sub").join("b"), b"B".to_vec()),
        ]);
        let missing = root.join("missing").join("..").join("a");
        let slash = PathBuf::from(format!("{}/", root.join("a").display()));
        let through = root.join("sub").join("..").join("a");

        for path in [&missing, &slash, &through] {
            assert_eq!(
                disk.is_file(path),
                memory.is_file(path),
                "is_file {}",
                path.display()
            );
            assert_eq!(
                outcome(disk.read(path)),
                outcome(memory.read(path)),
                "read {}",
                path.display()
            );
            assert_eq!(
                outcome(disk.real(path)),
                outcome(memory.real(path)),
                "real {}",
                path.display()
            );
        }
        assert_eq!(
            memory.read(&missing).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            memory.read(&slash).unwrap_err().kind(),
            io::ErrorKind::NotADirectory
        );
        assert_eq!(memory.read(&through).unwrap(), b"A");
        assert_eq!(disk.real(&through).unwrap(), memory.real(&through).unwrap());
    }

    fn outcome(result: io::Result<impl Sized>) -> Result<(), io::ErrorKind> {
        result.map(|_| ()).map_err(|error| error.kind())
    }
}
