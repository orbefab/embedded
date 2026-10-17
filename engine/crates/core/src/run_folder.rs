//! Run folder v0.
//!
//! ```text
//! <out>/
//!   manifest.json
//!   netlist.json
//!   diagnostics.json
//!   observations.json
//!   traces/            transient only, see `trace`
//! ```
//!
//! `observations.json` is `{"op": {ref: number}}`, plus `switches` for a
//! transient, or `{}` when the run reported an error.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::diagnostic::{Diagnostic, Severity};
use crate::finite::Finite;
use crate::name::Ref;
use crate::netlist::{self, Edge, Netlist};
use crate::timestamp::Timestamp;
use crate::trace::Trace;

pub const FORMAT_VERSION: u64 = 0;

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What a simulation produced. Diagnostics are sorted on construction.
#[derive(Debug)]
pub struct Results {
    observations: Observations,
    trace: Option<Trace>,
    diagnostics: Vec<Diagnostic>,
}

impl Results {
    pub fn new(
        observations: Observations,
        trace: Option<Trace>,
        mut diagnostics: Vec<Diagnostic>,
    ) -> Results {
        diagnostics.sort_by_cached_key(|diagnostic| {
            let bytes = serde_json::to_vec(diagnostic).expect("diagnostic serializes");
            (diagnostic.sort_key(), bytes)
        });
        Results {
            observations,
            trace,
            diagnostics,
        }
    }

    /// The netlist was rejected. No observations.
    pub fn rejected(diagnostics: Vec<Diagnostic>) -> Results {
        Results::new(Observations::default(), None, diagnostics)
    }

    pub fn observations(&self) -> &Observations {
        &self.observations
    }

    /// The accepted rows of a transient, kept up to an error.
    pub fn trace(&self) -> Option<&Trace> {
        self.trace.as_ref()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn count(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity() == severity)
            .count()
    }
}

/// Point values an agent reads by ref.
#[derive(Debug, Default, Serialize)]
pub struct Observations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<OpPoint>,
    /// Every contact edge before the transient's stop, per switch. `None` for an operating point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switches: Option<BTreeMap<Ref, Vec<Edge>>>,
}

/// DC operating point. Net voltages and part quantities, keyed by `Ref`.
///
/// Serialized flat. There is no `values` object around the map.
#[derive(Debug, Default, Serialize)]
pub struct OpPoint {
    #[serde(flatten)]
    pub values: BTreeMap<Ref, Finite>,
}

/// A rendered run folder. Relative path to bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct RunFolder {
    files: BTreeMap<String, Vec<u8>>,
}

const INPUT_FILE: &str = "netlist.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    run_format: u64,
    netlist_format: u64,
    engine_version: String,
    input: ManifestInput,
    // Without this, serde reads a missing `seed` as `None`; an owned manifest must name it.
    #[serde(deserialize_with = "Option::deserialize")]
    seed: Option<u64>,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestInput {
    file: String,
    sha256: String,
    bytes: u64,
}

/// Render the folder. `netlist` is `None` when parsing rejected the input.
pub fn render(
    input: &[u8],
    netlist: Option<&Netlist>,
    results: &Results,
    created_at: Timestamp,
) -> RunFolder {
    let manifest = Manifest {
        run_format: FORMAT_VERSION,
        netlist_format: netlist::FORMAT_VERSION,
        engine_version: ENGINE_VERSION.to_string(),
        input: ManifestInput {
            file: INPUT_FILE.to_string(),
            sha256: hex(&Sha256::digest(input)),
            bytes: input.len() as u64,
        },
        seed: netlist.map(Netlist::seed),
        created_at: created_at.rfc3339(),
    };
    let mut files = BTreeMap::new();
    files.insert(
        "diagnostics.json".to_string(),
        to_pretty(results.diagnostics()),
    );
    files.insert("manifest.json".to_string(), to_pretty(&manifest));
    files.insert("netlist.json".to_string(), input.to_vec());
    files.insert(
        "observations.json".to_string(),
        to_pretty(results.observations()),
    );
    if let Some(trace) = results.trace() {
        for (name, bytes) in trace.files() {
            files.insert(format!("traces/{name}"), bytes);
        }
    }
    RunFolder { files }
}

fn to_pretty(value: &(impl Serialize + ?Sized)) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("run folder value serializes");
    bytes.push(b'\n');
    bytes
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

impl RunFolder {
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
    }

    /// Replace `out` with this folder.
    ///
    /// Files land in `<out>.partial.<pid>` and are renamed into place. That
    /// rename is the commit. A crash before it leaves `out` unchanged, and a
    /// failure while deleting the backup afterwards does not fail the run.
    /// An existing non-empty directory is replaced only when its
    /// `manifest.json` is a run folder from this engine. Stale
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

fn sibling(out: &Path, kind: &str) -> PathBuf {
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
    let Ok(manifest) = serde_json::from_slice::<Manifest>(bytes) else {
        return false;
    };
    manifest.run_format == FORMAT_VERSION
        && manifest.netlist_format == netlist::FORMAT_VERSION
        && manifest.input.file == INPUT_FILE
        && is_sha256_hex(&manifest.input.sha256)
        && Timestamp::parse(&manifest.created_at).is_some()
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Diagnostic;
    use crate::finite::Finite;
    use crate::name::NameError;
    use crate::time::Time;

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

    #[test]
    fn render_is_pure_and_stamps_the_input() {
        let input = b"{}";
        let a = rejected_folder(input, 0);
        let b = rejected_folder(input, 0);
        assert_eq!(a, b);
        assert_eq!(a.file("netlist.json"), Some(input.as_slice()));
        assert_eq!(a.file("observations.json"), Some(b"{}\n".as_slice()));
        assert_eq!(a.file("diagnostics.json"), Some(b"[]\n".as_slice()));
        let manifest: serde_json::Value =
            serde_json::from_slice(a.file("manifest.json").unwrap()).unwrap();
        assert_eq!(manifest["seed"], serde_json::Value::Null);
        assert_eq!(manifest["created_at"], "1970-01-01T00:00:00Z");
        assert_eq!(manifest["engine_version"], ENGINE_VERSION);
        assert_eq!(manifest["run_format"], 0);
        assert_eq!(manifest["netlist_format"], 0);
        assert_eq!(manifest["input"]["file"], "netlist.json");
        assert_eq!(manifest["input"]["bytes"], 2);
        assert_eq!(manifest["input"]["sha256"], hex(&Sha256::digest(input)));
        let text = std::str::from_utf8(a.file("manifest.json").unwrap()).unwrap();
        assert!(text.starts_with("{\n  \"run_format\": 0,\n"));
        assert!(text.contains("\"created_at\": \"1970-01-01T00:00:00Z\"\n}\n"));
    }

    #[test]
    fn known_unix_times_format_as_utc() {
        let stamp = |secs| {
            let folder = rejected_folder(b"{}", secs);
            let manifest: serde_json::Value =
                serde_json::from_slice(folder.file("manifest.json").unwrap()).unwrap();
            manifest["created_at"].as_str().unwrap().to_string()
        };
        let cases = [
            (0, "1970-01-01T00:00:00Z"),
            (86_400, "1970-01-02T00:00:00Z"),
            (1_582_934_400, "2020-02-29T00:00:00Z"),
            (1_609_459_200, "2021-01-01T00:00:00Z"),
            (4_102_444_800, "2100-01-01T00:00:00Z"),
            (253_402_300_800, "10000-01-01T00:00:00Z"),
        ];
        for (secs, text) in cases {
            assert_eq!(stamp(secs), text);
            assert_eq!(
                Timestamp::parse(text),
                Some(Timestamp::from_unix_seconds(secs)),
                "{text}"
            );
        }
        assert!(Timestamp::parse("not-a-date").is_none());
        assert!(Timestamp::parse("1970-01-01T00:00:00+00:00").is_none());
        assert!(Timestamp::parse("2100-02-29T00:00:00Z").is_none());
    }

    #[test]
    fn observations_have_no_values_object() {
        let mut values = BTreeMap::new();
        values.insert("#net.gnd".parse().unwrap(), Finite::new(0.0).unwrap());
        values.insert("#net.vout".parse().unwrap(), Finite::new(-0.0).unwrap());
        values.insert("#r1.i".parse().unwrap(), Finite::new(1.5).unwrap());
        let results = Results::new(
            Observations {
                op: Some(OpPoint { values }),
                switches: None,
            },
            None,
            vec![],
        );
        let folder = render(b"{}", None, &results, Timestamp::from_unix_seconds(0));
        let text = std::str::from_utf8(folder.file("observations.json").unwrap()).unwrap();
        assert!(!text.contains("values"));
        assert!(text.contains("\"#net.gnd\": 0.0"));
        assert!(text.contains("\"#net.vout\": 0.0"));
        assert!(text.find("\"#net.gnd\"").unwrap() < text.find("\"#r1.i\"").unwrap());
    }

    #[test]
    fn a_trace_lands_under_traces_and_a_rerun_replaces_it() {
        let mut trace = Trace::new(vec!["#net.a".parse().unwrap()], Time::from_ps(5));
        trace.push(Time::ZERO, &[Finite::new(1.0).unwrap()]);
        trace.push(Time::from_ps(5), &[Finite::new(2.0).unwrap()]);
        let results = Results::new(Observations::default(), Some(trace), vec![]);
        let folder = render(b"{}", None, &results, Timestamp::from_unix_seconds(0));
        let paths: Vec<_> = folder.files().map(|(path, _)| path).collect();
        assert_eq!(
            paths,
            [
                "diagnostics.json",
                "manifest.json",
                "netlist.json",
                "observations.json",
                "traces/analog.f64",
                "traces/index.json",
                "traces/time.u64",
            ]
        );
        assert_eq!(folder.file("traces/time.u64").unwrap().len(), 16);

        let dir = scratch("traces");
        folder.write(&dir).unwrap();
        assert_eq!(fs::read(dir.join("traces/analog.f64")).unwrap().len(), 16);
        rejected_folder(b"{}", 1).write(&dir).unwrap();
        assert!(!dir.join("traces").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diagnostic_order_ignores_emission_order() {
        let name = |path: &str, problem| Diagnostic::NetlistName {
            path: path.into(),
            name: path.into(),
            problem,
        };
        let bad = name("1", NameError::BadChar);
        let empty = name("", NameError::Empty);
        let forward = Results::new(
            Observations::default(),
            None,
            vec![bad.clone(), empty.clone()],
        );
        let reverse = Results::new(Observations::default(), None, vec![empty, bad]);
        assert_eq!(
            serde_json::to_vec(forward.diagnostics()).unwrap(),
            serde_json::to_vec(reverse.diagnostics()).unwrap()
        );

        let singular = Diagnostic::SolverSingular {
            at: Time::ZERO,
            unknown: "#v1".parse().unwrap(),
        };
        let syntax = Diagnostic::NetlistSyntax {
            path: String::new(),
            line: 1,
            column: 1,
            detail: "nope".into(),
        };
        let ordered = Results::new(Observations::default(), None, vec![singular, syntax]);
        assert_eq!(ordered.diagnostics()[0].code(), "netlist.syntax");
        assert_eq!(ordered.diagnostics()[1].code(), "solver.singular");
    }

    #[test]
    fn parsed_netlist_keeps_its_seed() {
        let input = fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/divider/netlist.json"),
        )
        .unwrap();
        let netlist = crate::netlist::parse(&input).unwrap();
        let folder = render(
            &input,
            Some(&netlist),
            &Results::rejected(vec![]),
            Timestamp::from_unix_seconds(0),
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(folder.file("manifest.json").unwrap()).unwrap();
        assert_eq!(manifest["seed"], 0);
        assert_eq!(folder.file("netlist.json").unwrap(), input.as_slice());
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

    #[test]
    fn write_refuses_an_unsupported_run_format() {
        let dir = scratch("future-format");
        fs::create_dir_all(&dir).unwrap();
        let sha = "a".repeat(64);
        fs::write(
            dir.join("manifest.json"),
            format!(
                r#"{{
              "run_format": 1,
              "netlist_format": 0,
              "engine_version": "0.1.0",
              "input": {{"file": "netlist.json", "sha256": "{sha}", "bytes": 2}},
              "seed": 0,
              "created_at": "1970-01-01T00:00:00Z"
            }}"#
            ),
        )
        .unwrap();
        fs::write(dir.join("notes.txt"), b"keep").unwrap();
        let err = rejected_folder(b"{}", 0).write(&dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(dir.join("notes.txt")).unwrap(), b"keep");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_refuses_a_forged_manifest() {
        let sha = "a".repeat(64);
        let base = format!(
            r#"{{"run_format":0,"netlist_format":0,"engine_version":"0.1.0","input":{{"file":"netlist.json","sha256":"{sha}","bytes":2}},"seed":0,"created_at":"1970-01-01T00:00:00Z"}}"#
        );
        let cases = [
            (
                "netlist-format",
                base.replace("\"netlist_format\":0", "\"netlist_format\":999"),
            ),
            ("sha256", base.replace(&sha, "zz")),
            (
                "created-at",
                base.replace("1970-01-01T00:00:00Z", "not-a-date"),
            ),
            ("no-seed", base.replace("\"seed\":0,", "")),
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
