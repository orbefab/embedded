//! Analog traces of a transient run.
//!
//! ```text
//! traces/
//!   index.json   the row count, the files, and the columns in ref order
//!   time.u64     one little-endian u64 per row, picoseconds, strictly increasing
//!   analog.f64   little-endian f64, column-major: column j is rows j*n to (j+1)*n - 1
//! ```

use serde::Serialize;

use crate::finite::Finite;
use crate::name::{Quantity, Ref};
use crate::time::Time;

/// Every observed quantity at every accepted time.
#[derive(Debug, PartialEq)]
pub struct Trace {
    columns: Vec<Ref>,
    time: Vec<Time>,
    /// `values[column][row]`.
    values: Vec<Vec<f64>>,
    stop: Time,
}

impl Trace {
    /// A transient run that would store more rows ends with `trace.too_large`.
    pub const MAX_ROWS: usize = 1_000_000;
    /// 2^26 f64 values, 512 MiB.
    pub const MAX_VALUES: usize = 1 << 26;

    /// The row cap for a trace of `columns` columns.
    /// `columns` is at least 1. The cap is `min(MAX_ROWS, MAX_VALUES / columns)`.
    pub fn row_limit(columns: usize) -> usize {
        debug_assert!(columns >= 1);
        Self::MAX_ROWS.min(Self::MAX_VALUES / columns)
    }

    /// `columns` in ref order, the same keys as the operating point.
    pub fn new(columns: Vec<Ref>, stop: Time) -> Trace {
        let values = vec![Vec::new(); columns.len()];
        Trace {
            columns,
            time: Vec::new(),
            values,
            stop,
        }
    }

    /// Panics unless `at` is after the last row and `row` has one value per column.
    /// Either is an engine bug, never an input problem.
    pub fn push(&mut self, at: Time, row: &[Finite]) {
        assert!(self.time.last().is_none_or(|&last| last < at));
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
    pub(crate) fn files(&self) -> [(&'static str, Vec<u8>); 3] {
        let mut index = serde_json::to_vec_pretty(&self.index()).expect("trace index serializes");
        index.push(b'\n');
        let time = self.time.iter().flat_map(|t| t.as_ps().to_le_bytes());
        let analog = self.values.iter().flatten().flat_map(|v| v.to_le_bytes());
        [
            (INDEX, index),
            (TIME, time.collect()),
            (ANALOG, analog.collect()),
        ]
    }

    fn index(&self) -> Index<'_> {
        Index {
            format: FORMAT,
            rows: self.rows(),
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
        }
    }
}

const FORMAT: u64 = 0;
const INDEX: &str = "index.json";
const TIME: &str = "time.u64";
const ANALOG: &str = "analog.f64";

fn unit(reference: &Ref) -> &'static str {
    match reference {
        Ref::Quantity(_, Quantity::I) => "A",
        Ref::Quantity(_, Quantity::P) => "W",
        Ref::Net(_) | Ref::Quantity(_, Quantity::V) => "V",
        Ref::Part(_) | Ref::Pin(..) => panic!("{reference} is not an observed quantity"),
    }
}

#[derive(Serialize)]
struct Index<'a> {
    format: u64,
    rows: usize,
    stop_ps: Time,
    end_ps: Time,
    time: FileIndex,
    analog: AnalogIndex<'a>,
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
struct Column<'a> {
    #[serde(rename = "ref")]
    reference: &'a Ref,
    unit: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finite(values: &[f64]) -> Vec<Finite> {
        values.iter().map(|&v| Finite::new(v).unwrap()).collect()
    }

    fn trace() -> Trace {
        let columns = vec!["#net.a".parse().unwrap(), "#r1.i".parse().unwrap()];
        let mut trace = Trace::new(columns, Time::from_ps(30));
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
        ] = trace().files();
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
    #[should_panic]
    fn a_row_at_or_before_the_last_time_is_an_engine_bug() {
        let mut trace = trace();
        trace.push(Time::from_ps(10), &finite(&[0.0, 0.0]));
    }
}
