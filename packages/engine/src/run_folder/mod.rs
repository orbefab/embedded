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
//! `observations.json` holds what the run reached: `{}` when nothing solved,
//! `{"op"}` for an operating point, and `{"end_ps", "op", "switches", "logic"?, "displays"?}`
//! for a transient that solved t = 0, whether or not it reached its stop.

use orbe_model::Time;

use crate::netlist::{self, Edge, Netlist};

mod replace;

use std::collections::BTreeMap;

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::diagnostic::{Diagnostic, Severity};
use crate::display::Observed;
use crate::finite::Finite;
use crate::logic::LogicSummary;
use crate::name::Ref;

use crate::timestamp::Timestamp;
use crate::trace::Trace;

pub const FORMAT_VERSION: u64 = 0;

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The order `diagnostics.json` uses. `t_ps`, then `code`, then `refs`, then the JSON bytes.
pub fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by_cached_key(|diagnostic| {
        let bytes = serde_json::to_vec(diagnostic).expect("diagnostic serializes");
        (diagnostic.sort_key(), bytes)
    });
}

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
        sort_diagnostics(&mut diagnostics);
        Results {
            observations,
            trace,
            diagnostics,
        }
    }

    /// The netlist was rejected. No observations.
    pub fn rejected(diagnostics: Vec<Diagnostic>) -> Results {
        Results::new(Observations::Nothing, None, diagnostics)
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

/// What a run reached, which an agent reads by ref.
#[derive(Debug)]
pub enum Observations {
    /// Nothing solved: a rejected netlist, a failed operating point, or a transient that
    /// failed at t = 0.
    Nothing,
    Op(OpPoint),
    /// A transient that solved t = 0, whether or not it reached its stop.
    Transient {
        /// The last time the run solved, which is the trace's last row. It is the stop
        /// exactly when the run reported no error.
        end: Time,
        op: OpPoint,
        /// Every contact edge before `end`, per switch.
        switches: BTreeMap<Ref, Vec<Edge>>,
        /// One summary per chip signal up to `end`. `None` without a chip. Every change is
        /// in the trace's `Logic`.
        logic: Option<LogicSummary>,
        /// Each display part's state at `end`, by name. `None` without a display part.
        displays: Option<BTreeMap<String, Observed>>,
        /// Each LED chain's colours at `end`, by name. `None` without a chain.
        leds: Option<BTreeMap<String, crate::leds::Chain>>,
    },
}

impl Observations {
    pub fn op(&self) -> Option<&OpPoint> {
        match self {
            Observations::Nothing => None,
            Observations::Op(op) | Observations::Transient { op, .. } => Some(op),
        }
    }

    pub fn end(&self) -> Option<Time> {
        match self {
            Observations::Transient { end, .. } => Some(*end),
            Observations::Nothing | Observations::Op(_) => None,
        }
    }

    pub fn switches(&self) -> Option<&BTreeMap<Ref, Vec<Edge>>> {
        match self {
            Observations::Transient { switches, .. } => Some(switches),
            Observations::Nothing | Observations::Op(_) => None,
        }
    }

    pub fn logic(&self) -> Option<&LogicSummary> {
        match self {
            Observations::Transient { logic, .. } => logic.as_ref(),
            Observations::Nothing | Observations::Op(_) => None,
        }
    }
}

/// `{}`, `{"op"}`, or `{"end_ps", "op", "switches", "logic"?, "displays"?, "leds"?}`.
impl Serialize for Observations {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Observations::Nothing => serializer.serialize_map(Some(0))?.end(),
            Observations::Op(op) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("op", op)?;
                map.end()
            }
            Observations::Transient {
                end,
                op,
                switches,
                logic,
                displays,
                leds,
            } => {
                let entries = 3
                    + usize::from(logic.is_some())
                    + usize::from(displays.is_some())
                    + usize::from(leds.is_some());
                let mut map = serializer.serialize_map(Some(entries))?;
                map.serialize_entry("end_ps", end)?;
                map.serialize_entry("op", op)?;
                map.serialize_entry("switches", switches)?;
                if let Some(logic) = logic {
                    map.serialize_entry("logic", logic)?;
                }
                if let Some(displays) = displays {
                    map.serialize_entry("displays", displays)?;
                }
                if let Some(leds) = leds {
                    map.serialize_entry("leds", leds)?;
                }
                map.end()
            }
        }
    }
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

/// Written, never read back. Field order is the file's order.
#[derive(Serialize)]
struct Manifest {
    run_format: u64,
    /// The input's `version`, when it reads as an unsigned integer.
    netlist_format: Option<u64>,
    engine_version: String,
    input: ManifestInput,
    /// Each blob the netlist names, in [`Netlist::blobs`] order. Absent when it names none, or
    /// when parsing rejected the input.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    blobs: Vec<ManifestBlob>,
    /// `None` when parsing rejected the input.
    seed: Option<u64>,
    created_at: String,
}

#[derive(Serialize)]
struct ManifestBlob {
    role: &'static str,
    sha256: String,
    bytes: u64,
    /// Where a flash region starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<u32>,
}

#[derive(Serialize)]
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
        netlist_format: match netlist {
            Some(netlist) => Some(netlist.version()),
            None => netlist::probe_version(input),
        },
        engine_version: ENGINE_VERSION.to_string(),
        input: ManifestInput {
            file: INPUT_FILE.to_string(),
            sha256: hex(&Sha256::digest(input)),
            bytes: input.len() as u64,
        },
        blobs: netlist
            .into_iter()
            .flat_map(Netlist::blobs)
            .map(|blob| ManifestBlob {
                role: blob.role.as_str(),
                sha256: blob.sha256.to_string(),
                bytes: blob.bytes.len() as u64,
                offset: blob.role.offset(),
            })
            .collect(),
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
        for (name, bytes) in trace.images() {
            files.insert(format!("traces/{name}"), bytes.to_vec());
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Diagnostic;
    use crate::finite::Finite;
    use crate::name::NameError;
    use std::fs;
    use std::path::PathBuf;

    fn rejected_folder(input: &[u8], at: u64) -> RunFolder {
        render(
            input,
            None,
            &Results::rejected(vec![]),
            Timestamp::from_unix_seconds(at),
        )
    }

    /// A run folder path inside a fresh temp folder, which holds its siblings too.
    fn scratch(name: &str) -> (orbe_scratch::Scratch, PathBuf) {
        let scratch = orbe_scratch::Scratch::new();
        let path = scratch.join(name);
        (scratch, path)
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
        assert_eq!(manifest["netlist_format"], serde_json::Value::Null);
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
        let results = Results::new(Observations::Op(OpPoint { values }), None, vec![]);
        let folder = render(b"{}", None, &results, Timestamp::from_unix_seconds(0));
        let text = std::str::from_utf8(folder.file("observations.json").unwrap()).unwrap();
        assert!(!text.contains("values"));
        assert!(text.contains("\"#net.gnd\": 0.0"));
        assert!(text.contains("\"#net.vout\": 0.0"));
        assert!(text.find("\"#net.gnd\"").unwrap() < text.find("\"#r1.i\"").unwrap());
    }

    #[test]
    fn a_transient_writes_end_ps_first_and_logic_only_with_a_chip() {
        let mut values = BTreeMap::new();
        values.insert("#net.a".parse().unwrap(), Finite::new(1.0).unwrap());
        let observations = Observations::Transient {
            end: Time::from_ps(7),
            op: OpPoint { values },
            switches: BTreeMap::from([("#sw1".parse().unwrap(), Vec::new())]),
            logic: None,
            displays: None,
            leds: None,
        };
        assert_eq!(
            serde_json::to_string(&observations).unwrap(),
            r##"{"end_ps":7,"op":{"#net.a":1.0},"switches":{"#sw1":[]}}"##
        );
        assert_eq!(serde_json::to_string(&Observations::Nothing).unwrap(), "{}");
    }

    #[test]
    fn a_trace_lands_under_traces_and_a_rerun_replaces_it() {
        let transient = netlist::Transient {
            stop: Time::from_ps(5),
            max_step: 5,
            grid: 5,
        };
        let mut trace = Trace::new(vec!["#net.a".parse().unwrap()], &transient);
        trace.push(Time::ZERO, &[Finite::new(1.0).unwrap()]);
        trace.push(Time::from_ps(5), &[Finite::new(2.0).unwrap()]);
        let results = Results::new(Observations::Nothing, Some(trace), vec![]);
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
                "traces/envelope.f64",
                "traces/index.json",
                "traces/time.u64",
            ]
        );
        assert_eq!(folder.file("traces/time.u64").unwrap().len(), 16);
        assert_eq!(folder.file("traces/envelope.f64").unwrap().len(), 16);

        let (_dir_scratch, dir) = scratch("traces");
        folder.write(&dir).unwrap();
        assert_eq!(fs::read(dir.join("traces/analog.f64")).unwrap().len(), 16);
        rejected_folder(b"{}", 1).write(&dir).unwrap();
        assert!(!dir.join("traces").exists());
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
            Observations::Nothing,
            None,
            vec![bad.clone(), empty.clone()],
        );
        let reverse = Results::new(Observations::Nothing, None, vec![empty, bad]);
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
        let ordered = Results::new(Observations::Nothing, None, vec![singular, syntax]);
        assert_eq!(ordered.diagnostics()[0].code(), "netlist.syntax");
        assert_eq!(ordered.diagnostics()[1].code(), "solver.singular");
    }

    #[test]
    fn parsed_netlist_keeps_its_seed() {
        let input = fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/circuits/divider/netlist.json"),
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
        assert_eq!(manifest["netlist_format"], 1);
        assert_eq!(folder.file("netlist.json").unwrap(), input.as_slice());
    }

    #[test]
    fn a_rejected_netlist_records_the_version_it_names() {
        let manifest = |input: &[u8]| -> serde_json::Value {
            serde_json::from_slice(rejected_folder(input, 0).file("manifest.json").unwrap())
                .unwrap()
        };
        let future = manifest(br#"{"version": 2, "seed": 7}"#);
        assert_eq!(future["netlist_format"], 2);
        assert_eq!(future["seed"], serde_json::Value::Null);
        for unreadable in [&br#"{"version": "0"}"#[..], b"{}", b"[]", b"not json"] {
            assert_eq!(
                manifest(unreadable)["netlist_format"],
                serde_json::Value::Null
            );
        }
    }
}
