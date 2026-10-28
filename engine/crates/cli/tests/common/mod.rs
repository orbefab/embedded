//! The repo's files, run folders read back and held to their contract, and `logic.vcd` with the
//! `observations.logic` summary it should agree with.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// The scanner the circuit's tests use, so both crates see the same examples.
#[path = "../../../circuit/tests/common/examples.rs"]
pub mod examples;

pub mod atmega;

pub fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

/// The records of Intel HEX `text`, whatever its line endings.
pub fn records(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

/// Every file under `dir`, by path relative to it.
pub fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out
}

pub fn u64s(bytes: &[u8]) -> Vec<u64> {
    let (words, rest) = bytes.as_chunks::<8>();
    assert!(rest.is_empty());
    words.iter().map(|&word| u64::from_le_bytes(word)).collect()
}

pub fn f64s(bytes: &[u8]) -> Vec<f64> {
    let (words, rest) = bytes.as_chunks::<8>();
    assert!(rest.is_empty());
    words.iter().map(|&word| f64::from_le_bytes(word)).collect()
}

/// `scripts/ngspice_refs.py` reads this same file. A fixture that has it keeps no waveform rows.
pub const ROWS_SKIPPED_FILE: &str = "rows-skipped";

pub fn skips_reference_rows(fixture: &Path) -> bool {
    fixture.join(ROWS_SKIPPED_FILE).is_file()
}

/// Picoseconds a coil current takes to move 1e-3 of `span` plus 1 pA, at the slope from
/// `(t_s[0], current[0])` to `(t_s[1], current[1])`. `t_s` is seconds.
pub fn coil_edge_bound_ps(span: f64, t_s: &[f64], current: &[f64]) -> f64 {
    let slope = ((current[1] - current[0]) / (t_s[1] * 1e12 - t_s[0] * 1e12)).abs();
    assert!(
        slope.is_finite() && slope > 0.0,
        "flat coil current around the edge"
    );
    let bound = (1e-3 * span + 1e-12) / slope;
    assert!(bound.is_finite(), "coil edge bound is not finite");
    bound
}

/// Whether a chip in `netlist` has a USART. Today only an ATmega328P has one, `USART0`, so a new
/// chip with a USART adds its kind here.
pub fn has_usart(netlist: &serde_json::Value) -> bool {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["kind"] == "atmega328p")
}

/// Whether a chip in `netlist` can send records. Today only a `spi_replay` can, so a new kind
/// that sends records adds its kind here.
pub fn sends_records(netlist: &serde_json::Value) -> bool {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["kind"] == "spi_replay")
}

/// A transient run folder read back. `analog` holds one column of `time.len()` rows per ref in
/// `refs`. `envelope` holds each column's interval minimums, then its maximums.
pub struct TransientFolder {
    pub files: BTreeMap<String, Vec<u8>>,
    pub time: Vec<u64>,
    pub refs: Vec<String>,
    pub analog: Vec<f64>,
    pub envelope: Vec<f64>,
}

impl TransientFolder {
    /// `(minimums, maximums)` of `reference` over each interval between rows.
    pub fn bounds(&self, reference: &str) -> (&[f64], &[f64]) {
        let j = self.refs.iter().position(|r| r == reference).unwrap();
        let width = self.time.len() - 1;
        let column = &self.envelope[2 * j * width..2 * (j + 1) * width];
        column.split_at(width)
    }
}

/// Reads the run folder `out` of the transient example `name` and asserts what every one keeps:
/// its files, with `traces/logic.vcd` exactly when its observations have logic and
/// `traces/serial.csv` and `traces/i2c.csv` exactly when it has an ATmega328P, which has a
/// USART and a TWI, `traces/link.csv` exactly when a chip can send records, a row at each
/// `grid_ps` to `stop_ps`, row 0 equal to `observations.op`, and an envelope that holds both rows
/// of each interval.
pub fn transient_folder(name: &str, out: &Path) -> TransientFolder {
    let found = files(out);
    let names: Vec<_> = found.keys().map(String::as_str).collect();
    let observations: serde_json::Value =
        serde_json::from_slice(&found["observations.json"]).unwrap();
    let logic = observations.get("logic").is_some();
    let input: serde_json::Value = serde_json::from_slice(&found["netlist.json"]).unwrap();
    let usart = has_usart(&input);
    let mut expected = vec![
        "diagnostics.json",
        "manifest.json",
        "netlist.json",
        "observations.json",
        "traces/analog.f64",
        "traces/envelope.f64",
    ];
    if usart {
        expected.push("traces/i2c.csv");
    }
    expected.push("traces/index.json");
    if sends_records(&input) {
        expected.push("traces/link.csv");
    }
    if logic {
        expected.push("traces/logic.vcd");
    }
    if usart {
        expected.push("traces/serial.csv");
    }
    expected.push("traces/time.u64");
    assert_eq!(names, expected, "{name}");
    let index: serde_json::Value = serde_json::from_slice(&found["traces/index.json"]).unwrap();
    assert_eq!(index.get("logic").is_some(), logic, "{name}");
    let rows = index["rows"].as_u64().unwrap() as usize;
    let time = u64s(&found["traces/time.u64"]);
    let analog = f64s(&found["traces/analog.f64"]);
    let columns = index["analog"]["columns"].as_array().unwrap();
    assert_eq!(time.len(), rows, "{name}");
    assert_eq!(analog.len(), rows * columns.len(), "{name}");
    let stop = &input["analysis"]["transient"]["stop_ps"];
    let grid = &input["analysis"]["transient"]["grid_ps"];
    assert_eq!(&index["stop_ps"], stop);
    assert_eq!(&index["grid_ps"], grid);
    assert_eq!(&index["end_ps"], stop);
    let (stop, grid) = (stop.as_u64().unwrap(), grid.as_u64().unwrap());
    assert_eq!(rows as u64, stop.div_ceil(grid) + 1, "{name}");
    let want: Vec<u64> = (0..rows as u64).map(|k| (k * grid).min(stop)).collect();
    assert_eq!(time, want, "{name}");

    // The file's own text: `serde_json::Value` sorts keys and may round a parse by an ulp.
    let text = std::str::from_utf8(&found["observations.json"]).unwrap();
    let (op_text, _) = text.split_once("\"switches\"").unwrap();
    let op: Vec<(&str, &str)> = op_text
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"')?.split_once("\": "))
        .filter(|(key, _)| key.starts_with('#'))
        .map(|(key, value)| (key, value.trim_end_matches(',')))
        .collect();
    let refs: Vec<String> = columns
        .iter()
        .map(|c| c["ref"].as_str().unwrap().to_string())
        .collect();
    let keys: Vec<_> = op.iter().map(|(key, _)| *key).collect();
    assert_eq!(keys, refs, "{name}: columns are the op keys in order");
    for (j, (key, value)) in op.iter().enumerate() {
        let row_0 = serde_json::to_string(&analog[j * rows]).unwrap();
        assert_eq!(row_0, *value, "{name} {key}");
    }
    assert!(
        analog
            .iter()
            .all(|v| v.is_finite() && v.to_bits() != (-0.0f64).to_bits())
    );

    let envelope = f64s(&found["traces/envelope.f64"]);
    assert_eq!(envelope.len(), 2 * (rows - 1) * columns.len(), "{name}");
    assert!(envelope.iter().all(|v| v.is_finite()), "{name}");
    let folder = TransientFolder {
        files: found,
        time,
        refs,
        analog,
        envelope,
    };
    for (j, reference) in folder.refs.iter().enumerate() {
        let (mins, maxs) = folder.bounds(reference);
        let values = &folder.analog[j * rows..(j + 1) * rows];
        for k in 0..rows - 1 {
            let (low, high) = (values[k], values[k + 1]);
            assert!(
                mins[k] <= low.min(high) && low.max(high) <= maxs[k],
                "{name} {reference} interval {k}"
            );
        }
    }
    folder
}

/// `{ref: {initial, edges: [{t_ps, level}]}}` read from a logic.vcd.
pub fn read_vcd(text: &str) -> serde_json::Value {
    let mut tokens = text.split_whitespace();
    let mut scope = "";
    let mut names = Vec::new();
    let mut signals = serde_json::Map::new();
    let mut at = 0;
    while let Some(token) = tokens.next() {
        match token {
            "$timescale" => assert_eq!(
                tokens.by_ref().take(3).collect::<Vec<_>>(),
                ["1", "ps", "$end"]
            ),
            "$scope" => scope = tokens.nth(1).unwrap(),
            "$var" => {
                let var: Vec<_> = tokens.by_ref().take(5).collect();
                names.push((var[2].to_string(), format!("#{scope}.logic.{}", var[3])));
            }
            "$upscope" | "$enddefinitions" | "$dumpvars" | "$end" => {}
            _ if token.starts_with('#') => at = token[1..].parse::<u64>().unwrap(),
            _ => {
                let (bit, id) = token.split_at(1);
                let level = if bit == "1" { "high" } else { "low" };
                let (_, key) = names.iter().find(|(name, _)| name == id).unwrap();
                match signals.get_mut(key) {
                    None => {
                        signals.insert(
                            key.clone(),
                            serde_json::json!({ "initial": level, "edges": [] }),
                        );
                    }
                    Some(signal) => signal["edges"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({ "t_ps": at, "level": level })),
                }
            }
        }
    }
    serde_json::Value::Object(signals)
}

/// `observations.logic` computed from `read_vcd`'s edges.
pub fn summary(signals: &serde_json::Value) -> serde_json::Value {
    let signals = signals.as_object().unwrap().iter().map(|(key, signal)| {
        let edges = signal["edges"].as_array().unwrap();
        let initial = &signal["initial"];
        let time = |edge: Option<&serde_json::Value>| edge.map(|edge| edge["t_ps"].clone());
        let summary = serde_json::json!({
            "initial": initial,
            "final": edges.last().map_or(initial, |edge| &edge["level"]),
            "count": edges.len(),
            "first_ps": time(edges.first()),
            "last_ps": time(edges.last()),
        });
        (key.clone(), summary)
    });
    serde_json::Value::Object(signals.collect())
}
