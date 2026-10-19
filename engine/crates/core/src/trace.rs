//! Analog traces of a transient run.
//!
//! ```text
//! traces/
//!   index.json   the row count, the grid, the files, and the columns in ref order
//!   time.u64     one little-endian u64 per row, picoseconds, strictly increasing
//!   analog.f64   little-endian f64, column-major: column j is rows j*n to (j+1)*n - 1
//!   logic.vcd    with a chip only, see `logic`
//! ```

use serde::Serialize;

use crate::finite::Finite;
use crate::logic::Logic;
use crate::name::{Quantity, Ref};
use crate::netlist::Transient;
use crate::time::Time;

/// Every observed quantity at each row: t = 0, every multiple of the grid, and the stop, or
/// the last time a failed run solved.
#[derive(Debug, PartialEq)]
pub struct Trace {
    columns: Vec<Ref>,
    time: Vec<Time>,
    /// `values[column][row]`.
    values: Vec<Vec<f64>>,
    stop: Time,
    grid: u64,
    logic: Option<Logic>,
}

impl Trace {
    /// A transient whose grid gives more rows is refused with `trace.too_large`.
    pub const MAX_ROWS: usize = 1_000_000;
    /// 2^26 f64 values, 512 MiB.
    pub const MAX_VALUES: usize = 1 << 26;

    /// The row cap for a trace of `columns` columns.
    /// `columns` is at least 1. The cap is `min(MAX_ROWS, MAX_VALUES / columns)`.
    pub fn row_limit(columns: usize) -> usize {
        debug_assert!(columns >= 1);
        Self::MAX_ROWS.min(Self::MAX_VALUES / columns)
    }

    /// `columns` in ref order, the same keys as the operating point. Room for the rows a run
    /// of `transient` writes, up to the row limit.
    pub fn new(columns: Vec<Ref>, transient: &Transient) -> Trace {
        let rows = transient.rows().min(Self::row_limit(columns.len()) as u64) as usize;
        let values = columns.iter().map(|_| Vec::with_capacity(rows)).collect();
        Trace {
            columns,
            time: Vec::with_capacity(rows),
            values,
            stop: transient.stop,
            grid: transient.grid,
            logic: None,
        }
    }

    /// The chips' levels over the run, attached once at its end, after an error too.
    pub fn attach_logic(&mut self, logic: Logic) {
        assert!(self.logic.is_none(), "logic is attached once");
        self.logic = Some(logic);
    }

    pub fn logic(&self) -> Option<&Logic> {
        self.logic.as_ref()
    }

    /// Panics unless `at` is after the last row, the trace is under its row limit, and `row`
    /// has one value per column. Each is an engine bug, never an input problem.
    pub fn push(&mut self, at: Time, row: &[Finite]) {
        assert!(self.time.last().is_none_or(|&last| last < at));
        assert!(self.rows() < Self::row_limit(self.columns.len()));
        assert_eq!(row.len(), self.columns.len());
        self.time.push(at);
        for (column, value) in self.values.iter_mut().zip(row) {
            column.push(value.get());
        }
    }

    pub fn rows(&self) -> usize {
        self.time.len()
    }

    pub fn columns(&self) -> &[Ref] {
        &self.columns
    }

    pub fn time(&self) -> &[Time] {
        &self.time
    }

    pub fn column(&self, index: usize) -> &[f64] {
        &self.values[index]
    }

    /// The last row's time.
    pub fn end(&self) -> Option<Time> {
        self.time.last().copied()
    }

    /// File names under `traces/`, with their bytes.
    pub(crate) fn files(&self) -> Vec<(&'static str, Vec<u8>)> {
        let mut index = serde_json::to_vec_pretty(&self.index()).expect("trace index serializes");
        index.push(b'\n');
        let time = self.time.iter().flat_map(|t| t.as_ps().to_le_bytes());
        let analog = self.values.iter().flatten().flat_map(|v| v.to_le_bytes());
        let mut files = vec![
            (INDEX, index),
            (TIME, time.collect()),
            (ANALOG, analog.collect()),
        ];
        if let Some(logic) = &self.logic {
            files.push((VCD, logic.vcd()));
        }
        files
    }

    fn index(&self) -> Index<'_> {
        Index {
            format: FORMAT,
            rows: self.rows(),
            grid_ps: self.grid,
            stop_ps: self.stop,
            end_ps: self.end().unwrap_or(Time::ZERO),
            time: FileIndex {
                file: TIME,
                r#type: "u64le",
                unit: "ps",
            },
            analog: AnalogIndex {
                file: ANALOG,
                r#type: "f64le",
                order: "column_major",
                columns: self
                    .columns
                    .iter()
                    .map(|reference| Column {
                        reference,
                        unit: unit(reference),
                    })
                    .collect(),
            },
            logic: self.logic.as_ref().map(|logic| LogicIndex {
                file: VCD,
                r#type: "vcd",
                timescale: "1 ps",
                signals: logic
                    .vars()
                    .map(|(reference, scope, var, id)| Signal {
                        reference,
                        scope: scope.as_str(),
                        var: var.as_str(),
                        id,
                    })
                    .collect(),
            }),
        }
    }
}

/// A bump of this also bumps `run_folder::FORMAT_VERSION`.
const FORMAT: u64 = 0;
const INDEX: &str = "index.json";
const TIME: &str = "time.u64";
const ANALOG: &str = "analog.f64";
const VCD: &str = "logic.vcd";

fn unit(reference: &Ref) -> &'static str {
    match reference {
        Ref::Quantity(_, Quantity::I) => "A",
        Ref::Quantity(_, Quantity::P) => "W",
        Ref::Net(_) | Ref::Quantity(_, Quantity::V) => "V",
        Ref::Part(_) | Ref::Pin(..) | Ref::Logic(..) => {
            panic!("{reference} is not an observed quantity")
        }
    }
}

#[derive(Serialize)]
struct Index<'a> {
    format: u64,
    rows: usize,
    grid_ps: u64,
    stop_ps: Time,
    end_ps: Time,
    time: FileIndex,
    analog: AnalogIndex<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logic: Option<LogicIndex<'a>>,
}

#[derive(Serialize)]
struct FileIndex {
    file: &'static str,
    r#type: &'static str,
    unit: &'static str,
}

#[derive(Serialize)]
struct AnalogIndex<'a> {
    file: &'static str,
    r#type: &'static str,
    order: &'static str,
    columns: Vec<Column<'a>>,
}

#[derive(Serialize)]
struct LogicIndex<'a> {
    file: &'static str,
    r#type: &'static str,
    timescale: &'static str,
    signals: Vec<Signal<'a>>,
}

#[derive(Serialize)]
struct Signal<'a> {
    #[serde(rename = "ref")]
    reference: &'a Ref,
    scope: &'a str,
    var: &'a str,
    id: String,
}

#[derive(Serialize)]
struct Column<'a> {
    #[serde(rename = "ref")]
    reference: &'a Ref,
    unit: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::Level;

    fn finite(values: &[f64]) -> Vec<Finite> {
        values.iter().map(|&v| Finite::new(v).unwrap()).collect()
    }

    fn trace() -> Trace {
        let columns = vec!["#net.a".parse().unwrap(), "#r1.i".parse().unwrap()];
        let transient = Transient {
            stop: Time::from_ps(30),
            max_step: 10,
            grid: 10,
        };
        let mut trace = Trace::new(columns, &transient);
        trace.push(Time::ZERO, &finite(&[1.0, -0.0]));
        trace.push(Time::from_ps(10), &finite(&[2.5, 0.25]));
        trace
    }

    #[test]
    fn the_files_are_little_endian_and_column_major() {
        let [
            (index, index_bytes),
            (time, time_bytes),
            (analog, analog_bytes),
        ] = <[_; 3]>::try_from(trace().files()).unwrap();
        assert_eq!(
            (index, time, analog),
            ("index.json", "time.u64", "analog.f64")
        );
        let mut expected_time = 0u64.to_le_bytes().to_vec();
        expected_time.extend(10u64.to_le_bytes());
        assert_eq!(time_bytes, expected_time);
        let expected_analog: Vec<u8> = [1.0f64, 2.5, 0.0, 0.25]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(analog_bytes, expected_analog);
        let text = String::from_utf8(index_bytes).unwrap();
        assert_eq!(
            text,
            r##"{
  "format": 0,
  "rows": 2,
  "grid_ps": 10,
  "stop_ps": 30,
  "end_ps": 10,
  "time": {
    "file": "time.u64",
    "type": "u64le",
    "unit": "ps"
  },
  "analog": {
    "file": "analog.f64",
    "type": "f64le",
    "order": "column_major",
    "columns": [
      {
        "ref": "#net.a",
        "unit": "V"
      },
      {
        "ref": "#r1.i",
        "unit": "A"
      }
    ]
  }
}
"##
        );
    }

    #[test]
    fn logic_adds_the_vcd_and_maps_each_ref_to_its_scope_var_and_id() {
        let mut trace = trace();
        let signals = ["#u1.logic.out", "#u1.logic.trig"].map(|s| s.parse().unwrap());
        trace.attach_logic(Logic::new(signals.to_vec(), vec![Level::High, Level::Low]));
        let files = trace.files();
        let names: Vec<_> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["index.json", "time.u64", "analog.f64", "logic.vcd"]);
        let index: serde_json::Value = serde_json::from_slice(&files[0].1).unwrap();
        assert_eq!(
            index["logic"],
            serde_json::json!({
                "file": "logic.vcd",
                "type": "vcd",
                "timescale": "1 ps",
                "signals": [
                    { "ref": "#u1.logic.out", "scope": "u1", "var": "out", "id": "!" },
                    { "ref": "#u1.logic.trig", "scope": "u1", "var": "trig", "id": "\"" }
                ]
            })
        );
        let text = String::from_utf8(files[3].1.clone()).unwrap();
        assert!(text.contains("$var wire 1 \" trig $end\n"), "{text}");
    }

    #[test]
    #[should_panic]
    fn a_row_at_or_before_the_last_time_is_an_engine_bug() {
        let mut trace = trace();
        trace.push(Time::from_ps(10), &finite(&[0.0, 0.0]));
    }
}
