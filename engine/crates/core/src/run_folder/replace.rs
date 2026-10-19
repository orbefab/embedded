//! The filesystem transaction behind `RunFolder::write`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{INPUT_FILE, RunFolder};
use crate::timestamp::Timestamp;

/// What marks a previous run's manifest, from an engine of any version. Other keys are
/// ignored.
#[derive(Deserialize)]
struct RunShape {
    #[serde(rename = "run_format")]
    _run_format: u64,
    input: RunShapeInput,
    created_at: String,
}

#[derive(Deserialize)]
struct RunShapeInput {
    file: String,
}

impl RunFolder {
    /// Replace `out` with this folder.
    ///
    /// Files land in `<out>.partial.<pid>` and are renamed into place. That
    /// rename is the commit. A crash before it leaves `out` unchanged, and a
    /// failure while deleting the backup afterwards does not fail the run.
    /// An existing non-empty directory is replaced only when its
    /// `manifest.json` has the shape of a run folder from any version of
    /// the engine. Stale
    /// `<out>.partial.<digits>` and `<out>.replacing.<digits>` paths are
    /// removed only after that check passes.
    pub fn write(&self, out: &Path) -> io::Result<()> {
        restore_newest_replacing(out)?;
        if !may_replace(out)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "directory exists, is not empty, and is not a run folder",
            ));
        }
        remove_stale_siblings(out)?;
        let staging = sibling(out, "partial");
        if let Some(parent) = staging.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(&staging)?;
        if let Err(err) = write_files(&staging, &self.files) {
            let _ = fs::remove_dir_all(&staging);
            return Err(err);
        }

        let backup = sibling(out, "replacing");
        if out.exists() {
            fs::rename(out, &backup)?;
        }
        if let Err(err) = fs::rename(&staging, out) {
            let restored = restore_backup(&backup, out);
            if staging.exists() {
                let _ = fs::remove_dir_all(&staging);
            }
            restored?;
            return Err(err);
        }
        if backup.exists() {
            let _ = fs::remove_dir_all(&backup);
        }
        Ok(())
    }
}

fn write_files(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> io::Result<()> {
    for (rel, bytes) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    Ok(())
}

pub(super) fn sibling(out: &Path, kind: &str) -> PathBuf {
    named_sibling(out, kind, u64::from(std::process::id()))
}

fn named_sibling(out: &Path, kind: &str, id: u64) -> PathBuf {
    let mut file_name = out.file_name().unwrap_or_default().to_os_string();
    file_name.push(format!(".{kind}.{id}"));
    out.parent().unwrap_or(Path::new("")).join(file_name)
}

fn parent_dir(out: &Path) -> Option<&Path> {
    match out.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Some(Path::new(".")),
        other => other,
    }
}

struct Replacing {
    id: u64,
    path: PathBuf,
}

#[derive(Default)]
struct StaleSiblings {
    partials: Vec<PathBuf>,
    replacings: Vec<Replacing>,
}

fn list_stale(out: &Path) -> io::Result<StaleSiblings> {
    let Some(parent) = parent_dir(out) else {
        return Ok(StaleSiblings::default());
    };
    if !parent.exists() {
        return Ok(StaleSiblings::default());
    }
    let out_name = out.file_name().unwrap_or_default();
    let mut partials = Vec::new();
    let mut replacings = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        if stale_id(&name, out_name, "partial").is_some() {
            partials.push(entry.path());
        } else if let Some(id) = stale_id(&name, out_name, "replacing") {
            replacings.push(Replacing {
                id,
                path: entry.path(),
            });
        }
    }
    Ok(StaleSiblings {
        partials,
        replacings,
    })
}

fn restore_newest_replacing(out: &Path) -> io::Result<()> {
    if out.exists() {
        return Ok(());
    }
    let mut directories = Vec::new();
    for replacing in list_stale(out)?.replacings {
        if replacing.path.is_dir() {
            directories.push(replacing);
        }
    }
    directories.sort_by_key(|replacing| recency(&replacing.path, replacing.id));
    let Some(newest) = directories.pop() else {
        return Ok(());
    };
    fs::rename(newest.path, out)
}

fn remove_stale_siblings(out: &Path) -> io::Result<()> {
    let stale = list_stale(out)?;
    for path in stale.partials {
        remove_sibling(&path)?;
    }
    for replacing in stale.replacings {
        remove_sibling(&replacing.path)?;
    }
    Ok(())
}

fn remove_sibling(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|err| sibling_error(path, err))?;
    let removed = if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    removed.map_err(|err| sibling_error(path, err))
}

fn sibling_error(path: &Path, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{err}: {}", path.display()))
}

fn recency(path: &Path, id: u64) -> (std::time::SystemTime, u64) {
    let modified = fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    (modified, id)
}

fn stale_id(file_name: &OsStr, out_name: &OsStr, kind: &str) -> Option<u64> {
    let name = os_bytes(file_name);
    let mut prefix = os_bytes(out_name);
    prefix.push(b'.');
    prefix.extend(kind.as_bytes());
    prefix.push(b'.');
    let rest = name.strip_prefix(prefix.as_slice())?;
    if rest.is_empty() || !rest.iter().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    std::str::from_utf8(rest).ok()?.parse().ok()
}

fn os_bytes(value: &OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        value.to_string_lossy().into_owned().into_bytes()
    }
}

fn restore_backup(backup: &Path, out: &Path) -> io::Result<()> {
    if !backup.exists() || out.exists() {
        return Ok(());
    }
    fs::rename(backup, out).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("{err}; backup left at {}", backup.display()),
        )
    })
}

fn may_replace(out: &Path) -> io::Result<bool> {
    if !out.exists() {
        return Ok(true);
    }
    if !out.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "path exists and is not a directory",
        ));
    }
    let mut entries = fs::read_dir(out)?;
    let nonempty = entries.next().transpose()?.is_some();
    if !nonempty {
        return Ok(true);
    }
    is_engine_manifest(&out.join("manifest.json"))
}

fn is_engine_manifest(path: &Path) -> io::Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let bytes = fs::read(path)?;
    Ok(manifest_is_ours(&bytes))
}

fn manifest_is_ours(bytes: &[u8]) -> bool {
    serde_json::from_slice::<RunShape>(bytes).is_ok_and(|shape| {
        shape.input.file == INPUT_FILE && Timestamp::parse(&shape.created_at).is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_folder::{Results, Timestamp, render};

    fn rejected_folder(input: &[u8], at: u64) -> RunFolder {
        render(
            input,
            None,
            &Results::rejected(vec![]),
            Timestamp::from_unix_seconds(at),
        )
    }

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("sim-engine-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let _ = fs::remove_dir_all(sibling(&path, "partial"));
        let _ = fs::remove_dir_all(sibling(&path, "replacing"));
        path
    }

    fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            out.insert(
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            );
        }
        out
    }

    #[test]
    fn write_is_idempotent_and_replaces_a_previous_run() {
        let dir = scratch("idem");
        let folder = rejected_folder(b"{}", 0);
        folder.write(&dir).unwrap();
        let first = tree(&dir);
        folder.write(&dir).unwrap();
        assert_eq!(tree(&dir), first);
        assert!(dir.join("manifest.json").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_replaces_an_empty_directory() {
        let dir = scratch("empty");
        fs::create_dir_all(&dir).unwrap();
        rejected_folder(b"{}", 0).write(&dir).unwrap();
        assert!(dir.join("manifest.json").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_refuses_a_non_run_directory() {
        let dir = scratch("foreign");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("notes.txt"), b"keep").unwrap();
        let err = rejected_folder(b"{}", 0).write(&dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(dir.join("notes.txt")).unwrap(), b"keep");
        assert!(!dir.join("manifest.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_clears_a_leftover_staging_directory() {
        let dir = scratch("staging");
        let staging = sibling(&dir, "partial");
        fs::create_dir_all(staging.join("stale")).unwrap();
        fs::write(staging.join("stale").join("half.json"), b"{").unwrap();
        rejected_folder(b"{}", 0).write(&dir).unwrap();
        assert!(dir.join("observations.json").is_file());
        assert!(!staging.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_refuses_a_foreign_manifest() {
        let dir = scratch("foreign-manifest");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("manifest.json"), br#"{"app":"web"}"#).unwrap();
        fs::write(dir.join("notes.txt"), b"keep").unwrap();
        let err = rejected_folder(b"{}", 0).write(&dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read(dir.join("manifest.json")).unwrap(),
            br#"{"app":"web"}"#
        );
        assert_eq!(fs::read(dir.join("notes.txt")).unwrap(), b"keep");
        let _ = fs::remove_dir_all(&dir);
    }

    fn previous_manifest() -> String {
        let sha = "a".repeat(64);
        format!(
            r#"{{"run_format":0,"netlist_format":0,"engine_version":"0.1.0","input":{{"file":"netlist.json","sha256":"{sha}","bytes":2}},"seed":0,"created_at":"1970-01-01T00:00:00Z"}}"#
        )
    }

    #[test]
    fn write_replaces_a_previous_run_of_any_format() {
        let base = previous_manifest();
        let cases = [
            ("run-format", base.replace("\"run_format\":0", "\"run_format\":1")),
            (
                "netlist-format",
                base.replace("\"netlist_format\":0", "\"netlist_format\":999"),
            ),
            (
                "null-netlist-format",
                base.replace("\"netlist_format\":0", "\"netlist_format\":null"),
            ),
            ("extra-key", base.replace("\"seed\":0", "\"seed\":0,\"steps\":12")),
            ("no-seed", base.replace("\"seed\":0,", "")),
            ("bare", r#"{"run_format":3,"input":{"file":"netlist.json"},"created_at":"2030-01-01T00:00:00Z"}"#.to_string()),
        ];
        for (name, manifest) in cases {
            let dir = scratch(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("manifest.json"), &manifest).unwrap();
            fs::write(dir.join("old.txt"), b"old").unwrap();
            rejected_folder(b"{}", 0).write(&dir).unwrap();
            assert!(dir.join("observations.json").is_file(), "{name}");
            assert!(!dir.join("old.txt").exists(), "{name}");
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn write_refuses_a_manifest_without_the_run_shape() {
        let base = previous_manifest();
        let cases = [
            (
                "string-run-format",
                base.replace("\"run_format\":0", "\"run_format\":\"0\""),
            ),
            ("no-run-format", base.replace("\"run_format\":0,", "")),
            (
                "input-file",
                base.replace("\"file\":\"netlist.json\"", "\"file\":\"other.json\""),
            ),
            (
                "created-at",
                base.replace("1970-01-01T00:00:00Z", "not-a-date"),
            ),
            ("array", "[]".to_string()),
        ];
        for (name, manifest) in cases {
            let dir = scratch(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("manifest.json"), &manifest).unwrap();
            fs::write(dir.join("sentinel.txt"), b"keep").unwrap();
            let err = rejected_folder(b"{}", 0).write(&dir).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{name}");
            assert_eq!(
                fs::read(dir.join("sentinel.txt")).unwrap(),
                b"keep",
                "{name}"
            );
            assert_eq!(
                fs::read(dir.join("manifest.json")).unwrap(),
                manifest.as_bytes(),
                "{name}"
            );
            assert!(!dir.join("observations.json").exists(), "{name}");
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn write_refuses_without_removing_a_stale_partial() {
        let dir = scratch("refuse-partial");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("notes.txt"), b"keep").unwrap();
        let partial = named_sibling(&dir, "partial", 4242);
        let replacing = named_sibling(&dir, "replacing", 4242);
        let _ = fs::remove_dir_all(&partial);
        let _ = fs::remove_dir_all(&replacing);
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("sentinel"), b"stay").unwrap();
        fs::create_dir_all(&replacing).unwrap();
        fs::write(replacing.join("sentinel"), b"stay").unwrap();
        let err = rejected_folder(b"{}", 0).write(&dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(dir.join("notes.txt")).unwrap(), b"keep");
        assert_eq!(fs::read(partial.join("sentinel")).unwrap(), b"stay");
        assert_eq!(fs::read(replacing.join("sentinel")).unwrap(), b"stay");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&partial);
        let _ = fs::remove_dir_all(&replacing);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_backup_is_restored_before_the_ownership_check() {
        let target = scratch("symlink-target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("manifest.json"), br#"{"app":"web"}"#).unwrap();
        fs::write(target.join("sentinel"), b"keep").unwrap();
        let out = scratch("symlink-out");
        let replacing = named_sibling(&out, "replacing", 4242);
        let _ = fs::remove_file(&replacing);
        std::os::unix::fs::symlink(&target, &replacing).unwrap();
        let err = rejected_folder(b"{}", 0).write(&out).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"keep");
        assert!(!target.join("observations.json").exists());
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&replacing);
        let _ = fs::remove_dir_all(&target);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_previous_run_is_replaced_without_touching_its_target() {
        let target = scratch("symlink-run-target");
        let _ = fs::remove_dir_all(&target);
        rejected_folder(b"{}", 0).write(&target).unwrap();
        fs::write(target.join("sentinel"), b"keep").unwrap();
        let out = scratch("symlink-run-out");
        let _ = fs::remove_dir_all(&out);
        let replacing = named_sibling(&out, "replacing", 4343);
        let _ = fs::remove_file(&replacing);
        std::os::unix::fs::symlink(&target, &replacing).unwrap();
        rejected_folder(b"[]", 1).write(&out).unwrap();
        assert!(!fs::symlink_metadata(&out).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(out.join("netlist.json")).unwrap(), b"[]");
        assert!(fs::symlink_metadata(&replacing).is_err());
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"keep");
        assert_eq!(fs::read(target.join("netlist.json")).unwrap(), b"{}");
        let _ = fs::remove_dir_all(&out);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn write_removes_a_stale_partial_file() {
        let dir = scratch("partial-file");
        let partial = named_sibling(&dir, "partial", 7);
        let _ = fs::remove_file(&partial);
        fs::write(&partial, b"not a directory").unwrap();
        rejected_folder(b"{}", 0).write(&dir).unwrap();
        assert!(dir.join("manifest.json").is_file());
        assert!(!partial.exists());
        let _ = fs::remove_file(&partial);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_restores_a_stale_backup_then_replaces_it() {
        let dir = scratch("restore");
        let backup = named_sibling(&dir, "replacing", 999_999);
        fs::create_dir_all(&backup).unwrap();
        let previous = rejected_folder(b"old", 0);
        fs::write(
            backup.join("manifest.json"),
            previous.file("manifest.json").unwrap(),
        )
        .unwrap();
        fs::write(backup.join("old.txt"), b"old").unwrap();
        let partial = named_sibling(&dir, "partial", 999_999);
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("half.json"), b"{").unwrap();

        rejected_folder(b"{}", 0).write(&dir).unwrap();
        assert!(dir.join("observations.json").is_file());
        assert!(!dir.join("old.txt").exists());
        assert!(!backup.exists());
        assert!(!partial.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_restores_a_foreign_backup_and_then_refuses_it() {
        let dir = scratch("restore-foreign");
        let backup = named_sibling(&dir, "replacing", 999_999);
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("manifest.json"), br#"{"app":"web"}"#).unwrap();
        fs::write(backup.join("notes.txt"), b"keep").unwrap();
        let err = rejected_folder(b"{}", 0).write(&dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(dir.join("notes.txt")).unwrap(), b"keep");
        assert_eq!(
            fs::read(dir.join("manifest.json")).unwrap(),
            br#"{"app":"web"}"#
        );
        assert!(!backup.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn failed_backup_cleanup_still_commits_and_the_next_write_sweeps_it() {
        let dir = scratch("backup-cleanup");
        let _cleanup = BackupCleanup { dir: dir.clone() };
        rejected_folder(b"old", 0).write(&dir).unwrap();
        set_mode(&dir, 0o555);
        // A process that can write inside a 0555 directory can also delete it.
        if fs::write(dir.join("probe"), b"x").is_ok() {
            return;
        }
        rejected_folder(b"new", 1).write(&dir).unwrap();
        let backup = sibling(&dir, "replacing");
        assert!(backup.is_dir());
        assert_eq!(fs::read(dir.join("netlist.json")).unwrap(), b"new");
        set_mode(&backup, 0o755);
        rejected_folder(b"newer", 2).write(&dir).unwrap();
        assert!(!backup.exists());
        assert_eq!(fs::read(dir.join("netlist.json")).unwrap(), b"newer");
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::symlink_metadata(path).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(path, perms).unwrap();
    }

    #[cfg(unix)]
    struct BackupCleanup {
        dir: PathBuf,
    }

    #[cfg(unix)]
    impl Drop for BackupCleanup {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            for path in [
                sibling(&self.dir, "replacing"),
                sibling(&self.dir, "partial"),
                self.dir.clone(),
            ] {
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&path, perms);
                if meta.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
}
