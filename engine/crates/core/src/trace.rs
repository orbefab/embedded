//! Analog traces of a transient run.
//!
//! ```text
//! traces/
//!   index.json    the row count, the grid, the files, and the columns in ref order
//!   time.u64      one little-endian u64 per row, picoseconds, strictly increasing
//!   analog.f64    little-endian f64, column-major: column j is rows j*n to (j+1)*n - 1
//!   envelope.f64  little-endian f64, column-major: per column the minima then the maxima
//!                 of the rows - 1 intervals between the rows
//!   logic.vcd     when a chip in the netlist has logic signals, see `logic`
//!   serial.csv    when a chip in the netlist has a USART, see `serial`
//!   i2c.csv       when a chip in the netlist has a TWI, see `i2c`
//!   link.csv      when a chip in the netlist can send records, see `link`
//!   display.csv   when the netlist has a display part, see `display`
//!   display-<part>.rgb565
//! ```

use serde::Serialize;

use crate::display::Displays;
use crate::finite::Finite;
use crate::i2c::I2c;
use crate::link::Link;
use crate::logic::Logic;
use crate::name::{Quantity, Ref};
use crate::netlist::Transient;
use crate::serial::Serial;
use crate::time::Time;

/// Every observed quantity at each row: t = 0, every multiple of the grid, and the stop, or
/// the last time a failed run solved.
///
/// Between consecutive rows it keeps each column's minimum and maximum. The samples are the
/// two rows and every accepted step between them. A row closes the interval open since the
/// previous row.
#[derive(Debug, PartialEq)]
pub struct Trace {
    columns: Vec<Ref>,
    time: Vec<Time>,
    /// `values[column][row]`.
    values: Vec<Vec<f64>>,
    /// `mins[column][interval]`, `maxs[column][interval]`, one interval per pair of rows.
    mins: Vec<Vec<f64>>,
    maxs: Vec<Vec<f64>>,
    /// The interval open since the last row. `None` before the first row. A row
    /// replaces it with that row.
    open: Option<Open>,
    stop: Time,
    grid: u64,
    logic: Option<Logic>,
    serial: Option<Serial>,
    i2c: Option<I2c>,
    link: Option<Link>,
    displays: Option<Displays>,
}

/// Running minimum and maximum of the interval open since the last row, one pair per column.
#[derive(Debug, PartialEq)]
struct Open {
    min: Vec<f64>,
    max: Vec<f64>,
}

impl Open {
    fn fold(&mut self, row: &[Finite]) {
        for ((min, max), value) in self.min.iter_mut().zip(&mut self.max).zip(row) {
            let value = value.get();
            *min = (*min).min(value);
            *max = (*max).max(value);
        }
    }
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
    /// of `transient` writes, up to the row limit, and for the intervals between them.
    pub fn new(columns: Vec<Ref>, transient: &Transient) -> Trace {
        let rows = transient.rows().min(Self::row_limit(columns.len()) as u64) as usize;
        let intervals = rows.saturating_sub(1);
        let mins = columns
            .iter()
            .map(|_| Vec::with_capacity(intervals))
            .collect();
        let maxs = columns
            .iter()
            .map(|_| Vec::with_capacity(intervals))
            .collect();
        let values = columns.iter().map(|_| Vec::with_capacity(rows)).collect();
        Trace {
            columns,
            mins,
            maxs,
            open: None,
            time: Vec::with_capacity(rows),
            values,
            stop: transient.stop,
            grid: transient.grid,
            logic: None,
            serial: None,
            i2c: None,
            link: None,
            displays: None,
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

    /// The bytes the chips' USARTs sent over the run, attached once at its end, after an error
    /// too.
    pub fn attach_serial(&mut self, serial: Serial) {
        assert!(self.serial.is_none(), "serial is attached once");
        self.serial = Some(serial);
    }

    /// The phases the chips' TWIs saw over the run, attached once at its end, after an error
    /// too.
    pub fn attach_i2c(&mut self, i2c: I2c) {
        assert!(self.i2c.is_none(), "i2c is attached once");
        self.i2c = Some(i2c);
    }

    /// The records the chips sent each other over the run, attached once at its end, after an
    /// error too.
    pub fn attach_link(&mut self, link: Link) {
        assert!(self.link.is_none(), "link is attached once");
        self.link = Some(link);
    }

    /// What the display parts showed over the run, attached once at its end, after an error
    /// too.
    pub fn attach_displays(&mut self, displays: Displays) {
        assert!(self.displays.is_none(), "displays are attached once");
        self.displays = Some(displays);
    }

    /// Folds a non-row accepted step into the interval open since the last row. The engine
    /// folds every accepted step it does not push, so an interval's samples are its two rows
    /// and every accepted step strictly between them. Panics before the first row, which is
    /// always a row: there is nothing open to fold into.
    pub fn sample(&mut self, row: &[Finite]) {
        assert_eq!(row.len(), self.columns.len());
        let Some(open) = &mut self.open else {
            panic!("a folded step before the first row is an engine bug");
        };
        open.fold(row);
    }

    /// Panics unless `at` is after the last row, the trace is under its row limit, and `row`
    /// has one value per column. Each is an engine bug, never an input problem. The row
    /// closes the interval open since the previous row and opens the next one at itself.
    pub fn push(&mut self, at: Time, row: &[Finite]) {
        assert!(self.time.last().is_none_or(|&last| last < at));
        assert!(self.rows() < Self::row_limit(self.columns.len()));
        assert_eq!(row.len(), self.columns.len());
        self.close_interval(row);
        self.time.push(at);
        for (column, value) in self.values.iter_mut().zip(row) {
            column.push(value.get());
        }
    }

    /// Closes the interval that ends at this row and opens the next one at it. The closed
    /// interval's samples are the row that opened it, every accepted step folded in since,
    /// and this row. The first row only opens the first interval at itself.
    fn close_interval(&mut self, row: &[Finite]) {
        let Some(mut open) = self.open.take() else {
            let min: Vec<f64> = row.iter().map(|value| value.get()).collect();
            let max = min.clone();
            self.open = Some(Open { min, max });
            return;
        };
        open.fold(row);
        for (column, value) in self.mins.iter_mut().zip(&open.min) {
            column.push(*value);
        }
        for (column, value) in self.maxs.iter_mut().zip(&open.max) {
            column.push(*value);
        }
        for (slot, value) in open.min.iter_mut().zip(row) {
            *slot = value.get();
        }
        open.max.copy_from_slice(&open.min);
        self.open = Some(open);
    }

    pub fn rows(&self) -> usize {
        self.time.len()
    }

    /// One less than there are rows: interval k lies between rows k and k + 1.
    pub fn intervals(&self) -> usize {
        self.mins.first().map_or(0, Vec::len)
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

    /// The minimum of each interval, aligned with `column`.
    pub fn min_column(&self, index: usize) -> &[f64] {
        &self.mins[index]
    }

    /// The maximum of each interval, aligned with `column`.
    pub fn max_column(&self, index: usize) -> &[f64] {
        &self.maxs[index]
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
        let envelope = self
            .mins
            .iter()
            .zip(&self.maxs)
            .flat_map(|(min, max)| min.iter().chain(max))
            .flat_map(|v| v.to_le_bytes());
        let mut files = vec![
            (INDEX, index),
            (TIME, time.collect()),
            (ANALOG, analog.collect()),
            (ENVELOPE, envelope.collect()),
        ];
        if let Some(logic) = &self.logic {
            files.push((VCD, logic.vcd()));
        }
        if let Some(serial) = &self.serial {
            files.push((SERIAL, serial.csv()));
        }
        if let Some(i2c) = &self.i2c {
            files.push((I2C, i2c.csv()));
        }
        if let Some(link) = &self.link {
            files.push((LINK, link.csv().expect("a duty resolution is below 32")));
        }
        if let Some(displays) = &self.displays {
            files.push((DISPLAY, displays.csv()));
        }
        files
    }

    /// Each pixel panel's frame memory file under `traces/`, with its bytes. Their names come
    /// from part names, so they are not in `files`.
    pub(crate) fn images(&self) -> impl Iterator<Item = (String, &[u8])> {
        self.displays.iter().flat_map(Displays::images)
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
            envelope: EnvelopeIndex {
                file: ENVELOPE,
                r#type: "f64le",
                order: "column_major",
                values: ["min", "max"],
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
const ENVELOPE: &str = "envelope.f64";
const VCD: &str = "logic.vcd";
const SERIAL: &str = "serial.csv";
const I2C: &str = "i2c.csv";
const LINK: &str = "link.csv";
const DISPLAY: &str = "display.csv";

fn unit(reference: &Ref) -> &'static str {
    match reference {
        Ref::Quantity(
            _,
            Quantity::Ib
            | Quantity::Ic
            | Quantity::Id
            | Quantity::Iin
            | Quantity::Iout
            | Quantity::I,
        ) => "A",
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
    envelope: EnvelopeIndex,
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
struct EnvelopeIndex {
    file: &'static str,
    r#type: &'static str,
    order: &'static str,
    /// Per column, the minima block and then the maxima block.
    values: [&'static str; 2],
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
    use crate::i2c::{I2cPhase, I2cRow};
    use crate::logic::Level;
    use crate::name::Name;
    use crate::serial::SerialByte;

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
            (envelope, envelope_bytes),
        ] = <[_; 4]>::try_from(trace().files()).unwrap();
        assert_eq!(
            (index, time, analog, envelope),
            ("index.json", "time.u64", "analog.f64", "envelope.f64")
        );
        let mut expected_time = 0u64.to_le_bytes().to_vec();
        expected_time.extend(10u64.to_le_bytes());
        assert_eq!(time_bytes, expected_time);
        let expected_analog: Vec<u8> = [1.0f64, 2.5, 0.0, 0.25]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(analog_bytes, expected_analog);
        // One interval per column: its minimum, then its maximum.
        let expected_envelope: Vec<u8> = [1.0f64, 2.5, 0.0, 0.25]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(envelope_bytes, expected_envelope);
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
  },
  "envelope": {
    "file": "envelope.f64",
    "type": "f64le",
    "order": "column_major",
    "values": [
      "min",
      "max"
    ]
  }
}
"##
        );
    }

    #[test]
    fn a_folded_step_widens_the_open_interval_and_a_row_closes_it() {
        let mut trace = trace();
        assert_eq!(trace.intervals(), 1);
        assert_eq!(trace.min_column(0), [1.0]);
        assert_eq!(trace.max_column(0), [2.5]);
        // Two accepted steps after the last row, then a failed run's last row.
        trace.sample(&finite(&[2.5, 0.25]));
        trace.sample(&finite(&[0.5, 1.5]));
        trace.push(Time::from_ps(20), &finite(&[1.5, -0.0]));
        assert_eq!(trace.rows(), 3);
        assert_eq!(trace.intervals(), 2);
        assert_eq!(trace.min_column(0), [1.0, 0.5]);
        assert_eq!(trace.max_column(0), [2.5, 2.5]);
        assert_eq!(trace.min_column(1), [0.0, 0.0]);
        assert_eq!(trace.max_column(1), [0.25, 1.5]);
        let files = trace.files();
        let envelope: Vec<f64> = files[3]
            .1
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c))
            .collect();
        assert_eq!(envelope, [1.0, 0.5, 2.5, 2.5, 0.0, 0.0, 0.25, 1.5]);
    }

    #[test]
    fn one_row_writes_an_empty_envelope_and_keeps_the_key() {
        let columns = vec!["#net.a".parse().unwrap()];
        let transient = Transient {
            stop: Time::from_ps(30),
            max_step: 10,
            grid: 10,
        };
        let mut trace = Trace::new(columns, &transient);
        trace.push(Time::ZERO, &finite(&[1.0]));
        assert_eq!(trace.intervals(), 0);
        let files = trace.files();
        assert_eq!(files[3].0, "envelope.f64");
        assert!(files[3].1.is_empty());
        let index: serde_json::Value = serde_json::from_slice(&files[0].1).unwrap();
        assert_eq!(index["envelope"]["file"], "envelope.f64");
        assert_eq!(
            index["envelope"]["values"],
            serde_json::json!(["min", "max"])
        );
    }

    #[test]
    #[should_panic]
    fn a_step_folded_before_the_first_row_is_an_engine_bug() {
        let columns = vec!["#net.a".parse().unwrap()];
        let transient = Transient {
            stop: Time::from_ps(30),
            max_step: 10,
            grid: 10,
        };
        let mut trace = Trace::new(columns, &transient);
        trace.sample(&finite(&[1.0]));
    }

    #[test]
    fn logic_adds_the_vcd_and_maps_each_ref_to_its_scope_var_and_id() {
        let mut trace = trace();
        let signals = ["#u1.logic.out", "#u1.logic.trig"].map(|s| s.parse().unwrap());
        trace.attach_logic(Logic::new(signals.to_vec(), vec![Level::High, Level::Low]));
        let files = trace.files();
        let names: Vec<_> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "index.json",
                "time.u64",
                "analog.f64",
                "envelope.f64",
                "logic.vcd"
            ]
        );
        let index: serde_json::Value = serde_json::from_slice(&files[0].1).unwrap();
        assert_eq!(index["envelope"]["file"], "envelope.f64");
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
        let text = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(text.contains("$var wire 1 \" trig $end\n"), "{text}");
    }

    #[test]
    fn serial_comes_after_the_vcd_and_leaves_the_index_as_it_was() {
        let mut trace = trace();
        let signals = ["#u1.logic.PD1".parse().unwrap()];
        trace.attach_logic(Logic::new(signals.to_vec(), vec![Level::High]));
        let index = trace.files().swap_remove(0).1;
        let bytes = vec![SerialByte {
            at: Time::from_ps(5),
            usart: 0,
            byte: 72,
        }];
        let usarts = vec![(Name::new("u1").unwrap(), "USART0")];
        trace.attach_serial(Serial::new(usarts, bytes));
        let files = trace.files();
        let names: Vec<_> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "index.json",
                "time.u64",
                "analog.f64",
                "envelope.f64",
                "logic.vcd",
                "serial.csv"
            ]
        );
        assert_eq!(files[0].1, index);
        assert_eq!(files[5].1, b"t_ps,part,usart,byte\n5,u1,USART0,72\n");
    }

    #[test]
    fn i2c_comes_after_serial_and_leaves_the_index_as_it_was() {
        let mut trace = trace();
        let index = trace.files().swap_remove(0).1;
        let u1 = Name::new("u1").unwrap();
        trace.attach_serial(Serial::new(vec![(u1.clone(), "USART0")], Vec::new()));
        let rows = vec![I2cRow {
            at: Time::from_ps(7),
            twi: 0,
            phase: I2cPhase::Stop,
        }];
        trace.attach_i2c(I2c::new(vec![u1], rows));
        let files = trace.files();
        let names: Vec<_> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "index.json",
                "time.u64",
                "analog.f64",
                "envelope.f64",
                "serial.csv",
                "i2c.csv"
            ]
        );
        assert_eq!(files[0].1, index);
        assert_eq!(
            files[5].1,
            b"t_ps,part,phase,address,read,byte,ack\n7,u1,stop,,,,\n"
        );
    }

    #[test]
    fn link_comes_after_i2c_and_leaves_the_index_as_it_was() {
        let mut trace = trace();
        let index = trace.files().swap_remove(0).1;
        let u1 = Name::new("u1").unwrap();
        trace.attach_i2c(I2c::new(vec![u1.clone()], Vec::new()));
        trace.attach_link(Link::new(vec![u1], Vec::new()));
        let files = trace.files();
        let names: Vec<_> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "index.json",
                "time.u64",
                "analog.f64",
                "envelope.f64",
                "i2c.csv",
                "link.csv"
            ]
        );
        assert_eq!(files[0].1, index);
        assert_eq!(
            files[5].1,
            b"t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash\n"
        );
    }

    #[test]
    #[should_panic]
    fn a_row_at_or_before_the_last_time_is_an_engine_bug() {
        let mut trace = trace();
        trace.push(Time::from_ps(10), &finite(&[0.0, 0.0]));
    }
}
