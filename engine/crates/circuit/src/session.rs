use std::mem::size_of;

use sim_core::diagnostic::{Diagnostic, Solve};
use sim_core::finite::Finite;
use sim_core::i2c::{I2c, I2cRow};
use sim_core::logic::{Change, Level};
use sim_core::name::{Name, Ref};
use sim_core::netlist::{Analysis, Document, Netlist, Transient};
use sim_core::run_folder::{self, Results, RunFolder};
use sim_core::serial::{Serial as Delivered, SerialByte};
use sim_core::time::Time;
use sim_core::timestamp::Timestamp;

use crate::RunStats;
use crate::active::{self, Actives};
use crate::chip;
use crate::lte::{self, Charges, Segment, Verdict};
use crate::mna::{Memo, Solution};
use crate::newton::{
    Circuit, Conditions, Converged, Failure, History, Integration, Iterate, Offender, Side, newton,
};
use crate::observe::{self, Columns};
use crate::op;
use crate::ratings::{Ratings, Timing};
use crate::step::Plan;
use crate::tran::{self, Breakpoints, Outcome, Record, Rows};
use crate::watch::{self, Levels};

mod input;

pub use input::{Applied, Input, Rejected, Serial, Setting};

const STEP_BUDGET: u32 = 50;

/// Accepted steps one transient run may take. The operating point at t = 0 is not one of
/// them. A dense switch or source schedule spends them on landings, and reaching the limit
/// before `stop` ends the run with `solver.step_limit`.
///
/// 100000000 sits far above the steps a run whose rows fit needs on ordinary circuits.
/// At this step policy a 1 MHz pin into a 10 ns load costs about 170000 steps per
/// simulated ms, so the limit allows about 590 ms of it. The step policy, not this limit,
/// bounds an MCU-rate run.
const MAX_STEPS: u64 = 100_000_000;

/// Newton's budget for each step, each probe locating a chip event, each solve of a row, and
/// each rung of a restart's ladder, plus the accepted steps one run may take. A test lowers one
/// to make that solve fail, or `steps` to end a dense schedule at the step limit.
#[derive(Clone, Copy)]
pub(crate) struct Budget {
    pub(crate) step: u32,
    pub(crate) probe: u32,
    pub(crate) row: u32,
    pub(crate) restart: u32,
    pub(crate) steps: u64,
}

pub(crate) const BUDGET: Budget = Budget {
    step: STEP_BUDGET,
    probe: STEP_BUDGET,
    row: STEP_BUDGET,
    restart: op::OP_BUDGET,
    steps: MAX_STEPS,
};

/// The whole state a step starts from. Replaced, never mutated, on accept. A retry, a
/// rejected step or a probe only reads it.
pub(crate) struct Accepted {
    pub(crate) at: Time,
    /// After a restart, the restart solve at `at`, not the row at `at`.
    pub(crate) iterate: Iterate,
    /// The row at `at`, read before any restart there.
    pub(crate) row: Vec<Finite>,
    /// After a restart, `iterate` read on the interval that starts at `at`.
    pub(crate) restart_row: Option<Vec<Finite>>,
    pub(crate) segment: Segment,
    /// Each chip's state, by slot.
    chips: Vec<chip::State>,
    /// Each watch's level at `at`.
    levels: Levels,
    /// The furthest end of any attempt toward this step or one before it, rejected, stalled,
    /// stiffened or cut by a watch. An edge there or later cuts none of them.
    pub(crate) reach: Time,
}

/// A step from an accepted one that converged, passed its error check, is located at its
/// first watch flip, and is not yet accepted.
pub(crate) struct Landing {
    /// The step attempted, before a watch cut it.
    h: u64,
    pub(crate) at: Time,
    /// Where the step attempted ends.
    step_end: Time,
    pub(crate) converged: Converged,
    /// Each watch's level at `at`.
    levels: Levels,
    /// Whether a watch cut the step.
    pub(crate) cut: bool,
    error: lte::Error,
    /// The row at `at`, read on `converged`.
    pub(crate) row: Vec<Finite>,
    /// The plan the step was attempted with, after any reject or stall.
    pub(crate) plan: Plan,
    /// The accepted segment with the capacitors a stiffen holds on backward Euler, when one did.
    pub(crate) stiffened: Option<Segment>,
    /// The accepted step's reach, or the furthest end of an attempt toward this landing when
    /// that is later.
    pub(crate) reach: Time,
}

/// A transient run held between calls. `advance_to` accepts every step that lands by its time.
/// The step after the last one it accepted is solved and waits. A call never cuts that step
/// and never accepts it early, so a session advanced in any chunks takes the steps of one
/// advanced to `stop` at once. A read at `now` changes nothing.
pub struct Session {
    pub(crate) circuit: Circuit,
    pub(crate) columns: Columns,
    transient: Transient,
    pub(crate) budget: Budget,
    charges: Charges,
    pub(crate) breakpoints: Breakpoints,
    pub(crate) actives: Actives,
    /// The plan for the step after the last accepted one, before any reject or stall.
    pub(crate) plan: Plan,
    pub(crate) stats: RunStats,
    /// The latest systems the run's steps, restarts and event probes solved. A row inside a span
    /// and `Session::probe` solve without it, because both only read the session.
    memo: Memo,
    /// Each chip signal, `Ref::Logic`, chips in part order.
    signals: Vec<Ref>,
    /// The level of each signal at `now`, in the order of `signals`.
    logic: Vec<Level>,
    /// Chip-signal changes on accepted steps when the session records nothing, in signal order
    /// within one time. The levels at t = 0 are not changes. A recorded session leaves this
    /// empty and reads the recorder's changes, which `logic.vcd` is written from.
    /// `edges_forgotten` is the first change a reader may still see, counting from the first
    /// change. On an unrecorded session `edges[0]` is that change.
    edges: Vec<Change>,
    edges_forgotten: u64,
    /// Integral of each analog column from t = 0 through the last accepted step, in `refs`
    /// order. The unit is the column's unit times picoseconds.
    integrals: Vec<f64>,
    /// Bytes a USART sent whose stop bit ended at or before the last accepted step, in the
    /// order stop bits ended, then in `usarts` order. `serial_forgotten` is the first byte a
    /// reader may still see, counting from the first byte. An unrecorded session drains the
    /// bytes before that, so `serial[0]` is that byte. A recorded session keeps every byte.
    serial: Vec<SerialByte>,
    serial_forgotten: u64,
    /// TWI phases that ended at or before the last accepted step, in the order they ended, then
    /// in `twis` order. Only a recorded session keeps them, for `i2c.csv`.
    i2c: Vec<I2cRow>,
    pub(crate) record: Option<Record>,
    /// The JSON the session opened from, which `export` writes back.
    document: Option<Document>,
    /// The bytes `record_document` read, until an input changes the netlist they hold.
    opened: Option<Vec<u8>>,
    /// `net.floating` warnings. `start` fills this from the netlist.
    pub(crate) warnings: Vec<Diagnostic>,
    /// Rated quantities, sampled at t = 0 and on each accepted step.
    ratings: Ratings,
    pub(crate) state: State,
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum State {
    /// `accepted.at <= now < next.at`. `next` is solved from `accepted` and not accepted.
    Running {
        accepted: Accepted,
        now: Time,
        next: Landing,
    },
    /// At `stop`, or at the last step the run reached when a fault ended it. After a failed
    /// restart that is the landing, which never became an `Accepted`.
    Ended {
        at: Time,
        row: Vec<Finite>,
        end: End,
    },
}

impl State {
    /// What a session holds while a step consumes its state. Nothing reads it.
    const TAKEN: State = State::Ended {
        at: Time::ZERO,
        row: Vec::new(),
        end: End::Stop,
    };
}

/// Why a session ended.
pub(crate) enum End {
    Stop,
    /// The error that ended the run after t = 0, `solver.step_limit` included. A recorded
    /// session also ends on `trace.too_large`.
    Fault(Diagnostic),
}

/// How many serial bytes a reader has read. `SerialCursor::default()` has read none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SerialCursor(u64);

impl From<u64> for SerialCursor {
    fn from(read: u64) -> SerialCursor {
        SerialCursor(read)
    }
}

impl From<SerialCursor> for u64 {
    fn from(cursor: SerialCursor) -> u64 {
        cursor.0
    }
}

/// How many chip-signal changes a reader has read. `EdgeCursor::default()` has read none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeCursor(u64);

impl From<u64> for EdgeCursor {
    fn from(read: u64) -> EdgeCursor {
        EdgeCursor(read)
    }
}

impl From<EdgeCursor> for u64 {
    fn from(cursor: EdgeCursor) -> u64 {
        cursor.0
    }
}

/// Where a session stands after a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// `now` is before `stop`.
    Running,
    /// The session reached `stop`.
    Stopped,
    /// A fault ended the session, and `fault` returns it.
    Faulted,
}

/// Why `open` made no session.
#[derive(Clone, Debug, PartialEq)]
pub enum Refused {
    /// An `op` analysis has no time to advance.
    Op,
    /// `solver.too_large`, or the operating point at t = 0 failed to settle or gave a quantity
    /// that is not finite. For `record_document`, also `trace.too_large`.
    Failed(Diagnostic),
    /// The document `open_document` or `record_document` read is not a valid netlist.
    Invalid(Vec<Diagnostic>),
}

/// The transient a session runs. An `op` analysis has no time to advance.
fn transient(netlist: &Netlist) -> Result<Transient, Refused> {
    match netlist.analysis() {
        Analysis::Transient(transient) => Ok(*transient),
        Analysis::Op => Err(Refused::Op),
    }
}

/// Every analog column at one time, read in one call: each net's voltage, then each part's `i`,
/// `v` and `p`. The columns and their order are the trace's.
pub struct Probe<'s> {
    at: Time,
    columns: &'s Columns,
    values: Vec<Finite>,
    interpolated: bool,
    /// Integral of each column from t = 0 to `at`, in the column's unit times picoseconds.
    integrals: Vec<f64>,
}

impl std::fmt::Debug for Probe<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe")
            .field("at", &self.at)
            .field("values", &self.values)
            .field("interpolated", &self.interpolated)
            .field("integrals", &self.integrals)
            .finish()
    }
}

impl Probe<'_> {
    pub fn at(&self) -> Time {
        self.at
    }

    /// A `Ref::Net` or `Ref::Quantity` column. `None` for any other ref.
    pub fn get(&self, column: &Ref) -> Option<Finite> {
        self.columns.index(column).map(|k| self.values[k])
    }

    pub fn refs(&self) -> &[Ref] {
        self.columns.refs()
    }

    pub fn values(&self) -> &[Finite] {
        &self.values
    }

    /// Both solves failed, so the values are on the line between the accepted steps around it.
    pub fn interpolated(&self) -> bool {
        self.interpolated
    }

    /// The integral of each column from t = 0 to `at`, in the column's unit times picoseconds,
    /// in `refs` order.
    pub fn integrals(&self) -> &[f64] {
        &self.integrals
    }
}

/// Why `start` returned no session. `warnings` are the floating-net warnings a run reports
/// beside `error`.
#[derive(Debug)]
pub(crate) struct StartFailure {
    pub(crate) error: Diagnostic,
    pub(crate) warnings: Vec<Diagnostic>,
}

impl Session {
    /// Solves the operating point at t = 0 and the step after it, and keeps no trace. The step
    /// after t = 0 can fail, and then the session it returns has faulted.
    pub fn open(netlist: Netlist) -> Result<Session, Refused> {
        Session::open_with(netlist, None, BUDGET)
    }

    /// `open` on the netlist `json` parses to, keeping `json` so `export` can write it back.
    pub fn open_document(json: &[u8]) -> Result<Session, Refused> {
        let (document, netlist) = Document::parse(json).map_err(Refused::Invalid)?;
        Session::open_with(netlist, Some(document), BUDGET)
    }

    /// Tests only: `open_document` with Newton's budget for each row and each probe between
    /// steps lowered to `row`, so a test outside this crate can see a probe that fails twice.
    #[cfg(feature = "row-budget")]
    pub fn open_document_with_row_budget(json: &[u8], row: u32) -> Result<Session, Refused> {
        let (document, netlist) = Document::parse(json).map_err(Refused::Invalid)?;
        Session::open_with(netlist, Some(document), Budget { row, ..BUDGET })
    }

    /// `open_document` with the trace and logic history `sim run` keeps, so `finish` can write
    /// its run folder. It also refuses a grid with more rows than a trace holds, with
    /// `trace.too_large`.
    pub fn record_document(json: &[u8]) -> Result<Session, Refused> {
        let (document, netlist) = Document::parse(json).map_err(Refused::Invalid)?;
        let transient = transient(&netlist)?;
        let mut session = tran::record(netlist, &transient, Rows::Grid, Some(document))
            .map_err(|unopened| Refused::Failed(unopened.error))?;
        session.opened = Some(json.to_vec());
        Ok(session)
    }

    fn open_with(
        netlist: Netlist,
        document: Option<Document>,
        budget: Budget,
    ) -> Result<Session, Refused> {
        let transient = transient(&netlist)?;
        let columns = Columns::new(&netlist);
        Session::start(
            netlist,
            &transient,
            Vec::new(),
            budget,
            columns,
            None,
            document,
        )
        .map_err(|failure| Refused::Failed(failure.error))
    }

    /// Solves t = 0 and records it when `rows` is set, then solves the step after it. Fails only
    /// when t = 0 fails. The error carries the netlist's floating-net warnings.
    pub(crate) fn start(
        netlist: Netlist,
        transient: &Transient,
        actives: Vec<active::Chip>,
        budget: Budget,
        columns: Columns,
        rows: Option<Rows>,
        document: Option<Document>,
    ) -> Result<Session, Box<StartFailure>> {
        let warnings = crate::topology::floating_nets(&netlist);
        let fail = |error| {
            Box::new(StartFailure {
                error,
                warnings: warnings.clone(),
            })
        };
        let circuit = Circuit::with_actives(netlist, actives).map_err(fail)?;
        let charges = Charges::new(&circuit.netlist, &circuit.layout);
        let start = Iterate::initial(&circuit);
        let mut memo = Memo::default();
        let start = op::settle(
            &circuit,
            Some(&mut memo),
            Conditions::NOMINAL,
            start,
            circuit.power_on(),
            &Actives::power_on(&circuit),
            None,
            op::OP_BUDGET,
        )
        .map_err(fail)?;
        let row = observe::observe(
            &circuit,
            &columns,
            &start.iterate.solution,
            Time::ZERO,
            Side::Before,
        )
        .map_err(fail)?;
        let signals = circuit.logic_refs();
        let logic = circuit.logic_levels(&start.chips, &start.levels);
        let integrals = vec![0.0; columns.refs().len()];
        let mut ratings = Ratings::new(&circuit.netlist, &columns, Timing::Run);
        ratings.sample(Time::ZERO, &row);
        let record = rows
            .map(|rows| Record::start(transient, &columns, rows, &row, &signals, &logic))
            .transpose()
            .map_err(fail)?;
        let actives = Actives::new(&circuit, &start.levels);
        let breakpoints = Breakpoints::new(&circuit.netlist, transient.stop);
        let segment = Segment::start(&charges, &start.iterate.solution);
        let accepted = Accepted {
            at: Time::ZERO,
            iterate: start.iterate,
            row,
            restart_row: None,
            segment,
            chips: start.chips,
            levels: start.levels,
            reach: Time::ZERO,
        };
        let mut session = Session {
            circuit,
            columns,
            transient: *transient,
            budget,
            charges,
            breakpoints,
            actives,
            plan: Plan::RESTART,
            stats: RunStats::default(),
            memo,
            signals,
            logic,
            edges: Vec::new(),
            edges_forgotten: 0,
            integrals,
            serial: Vec::new(),
            serial_forgotten: 0,
            i2c: Vec::new(),
            record,
            document,
            opened: None,
            warnings,
            ratings,
            state: State::TAKEN,
        };
        session.state = session.solved(accepted);
        Ok(session)
    }

    /// The time the session stands at. While it runs that is the latest time a call asked for,
    /// capped at `stop`, and t = 0 before any call. Once it has ended it is the last accepted step.
    pub fn now(&self) -> Time {
        match &self.state {
            State::Running { now, .. } => *now,
            State::Ended { at, .. } => *at,
        }
    }

    /// The last accepted step, which is `now` once the session has ended.
    pub(crate) fn accepted(&self) -> Time {
        match &self.state {
            State::Running { accepted, .. } => accepted.at,
            State::Ended { at, .. } => *at,
        }
    }

    pub fn status(&self) -> Status {
        match &self.state {
            State::Running { .. } => Status::Running,
            State::Ended { end: End::Stop, .. } => Status::Stopped,
            State::Ended {
                end: End::Fault(_), ..
            } => Status::Faulted,
        }
    }

    /// The warnings `diagnostics.json` would list if the session ended now.
    ///
    /// Static warnings come first in that file's order, each with `t_ps` null. One
    /// `rating.current` or `rating.power` follows for each rated quantity that has gone
    /// over. Its `t_ps` is the first accepted step over. `peak` and `peak_ps` run through the
    /// last accepted step. `over_ps` runs through that step, not to `now`. A step that is over
    /// adds its time when the next step is accepted, so a quantity first over on the last
    /// accepted step reads 0. The fault that ends a run is [`Self::fault`].
    /// `trace.row_interpolated` is absent. It names a grid row, and only a recorded trace
    /// has those rows.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut found = self.warnings.clone();
        found.extend(self.ratings.findings());
        sim_core::run_folder::sort_diagnostics(&mut found);
        found
    }

    /// The error that ended a faulted session. `None` while it runs and once it stops.
    pub fn fault(&self) -> Option<&Diagnostic> {
        match &self.state {
            State::Ended {
                end: End::Fault(fault),
                ..
            } => Some(fault),
            _ => None,
        }
    }

    /// Accepts every step that lands at or before `to`, and `stop` caps `to`. A running session
    /// then stands at `to`, and a time at or before `now` does nothing. A fault ends the session
    /// at its last accepted step. Then the log holds every byte whose stop bit ended by the last
    /// accepted step, and no other.
    pub fn advance_to(&mut self, to: Time) -> Status {
        let to = to.min(self.transient.stop);
        let mut state = std::mem::replace(&mut self.state, State::TAKEN);
        loop {
            state = match state {
                State::Running { accepted, next, .. } if next.at <= to => {
                    self.accept(accepted, next, to)
                }
                _ => break,
            };
        }
        if let State::Running { now, .. } = &mut state {
            *now = (*now).max(to);
        }
        self.state = state;
        let accepted = self.accepted();
        self.serial.extend(self.actives.deliver(accepted));
        if self.record.is_some() {
            self.i2c.extend(self.actives.phases());
        } else {
            self.actives.phases().for_each(drop);
        }
        self.status()
    }

    /// Every analog column at `now`. On an accepted step it is that step's row. Between steps it
    /// is solved from the last accepted step the way a row is, so at a row time it is that row.
    /// The integrals run from t = 0 to that time. Between steps they add the trapezoid from the
    /// accepted step to these values, and that piece is not stored.
    pub fn probe(&self) -> Probe<'_> {
        let (at, values, interpolated, integrals) = match &self.state {
            State::Running {
                accepted,
                now,
                next,
            } if *now > accepted.at => {
                let (values, interpolated) = match self.span(accepted, next).probe(*now) {
                    Probed::Solved(values) => (values, false),
                    Probed::Interpolated(values) => (values, true),
                };
                let mut integrals = self.integrals.clone();
                let left = accepted.restart_row.as_ref().unwrap_or(&accepted.row);
                trapezoid(&mut integrals, (accepted.at, left), (*now, &values));
                (*now, values, interpolated, integrals)
            }
            State::Running { accepted, .. } => (
                accepted.at,
                accepted.row.clone(),
                false,
                self.integrals.clone(),
            ),
            State::Ended { at, row, .. } => (*at, row.clone(), false, self.integrals.clone()),
        };
        Probe {
            at,
            columns: &self.columns,
            values,
            interpolated,
            integrals,
        }
    }

    /// Each chip signal, a `Ref::Logic`, chips in part order.
    pub fn signals(&self) -> &[Ref] {
        &self.signals
    }

    /// The level of each chip signal at `now`, in the order of `signals`, with no solve. They
    /// are the levels `logic.vcd` holds there.
    pub fn levels(&self) -> &[Level] {
        &self.logic
    }

    /// The level of a chip signal, a `Ref::Logic`, at `now`, with no solve. It is the level
    /// `logic.vcd` holds there. `None` for any other ref.
    pub fn level(&self, signal: &Ref) -> Option<Level> {
        let k = self.signals.iter().position(|s| s == signal)?;
        Some(self.logic[k])
    }

    /// The cursor past every chip-signal change so far. `levels` at the same time is the t = 0
    /// levels with every change before this cursor applied.
    pub fn edge_cursor(&self) -> EdgeCursor {
        let count = match &self.record {
            Some(record) => record.changes().len() as u64,
            None => self.edges_forgotten + self.edges.len() as u64,
        };
        EdgeCursor(count)
    }

    /// Every chip-signal change after the ones `cursor` has read, up to the last accepted step,
    /// and the cursor past them. The changes are the ones `logic.vcd` holds, in signal order
    /// within one time. The levels at t = 0 are not changes. Solves nothing. A cursor past the
    /// end reads nothing and stays. `None` when `cursor` is before a change `forget_edges` forgot.
    pub fn edges_since(&self, cursor: EdgeCursor) -> Option<(&[Change], EdgeCursor)> {
        let cursor = u64::from(cursor);
        let read = match &self.record {
            Some(record) => read_retained(record.changes(), self.edges_forgotten, cursor),
            None => read_drained(&self.edges, self.edges_forgotten, cursor),
        };
        read.map(|(changes, end)| (changes, EdgeCursor(end)))
    }

    /// Forgets the chip-signal changes before `cursor`. An unrecorded session drains them. A
    /// recorded session only raises the read floor, because `logic.vcd` is written from the
    /// same list. Cursors keep counting from the first change. A later `edges_since` is
    /// unchanged except on a cursor before `cursor`.
    pub fn forget_edges(&mut self, cursor: EdgeCursor) {
        let cursor = u64::from(cursor);
        let len = self.record.as_ref().map(|record| record.changes().len());
        if let Some(len) = len {
            raise_floor(&mut self.edges_forgotten, len, cursor);
        } else {
            forget_prefix(&mut self.edges, &mut self.edges_forgotten, cursor);
        }
    }

    /// Each USART, chips in part order, then each chip's in datasheet order: the part's name and
    /// the USART's, such as `USART0`. A `SerialByte`'s `usart` indexes this.
    pub fn usarts(&self) -> &[(Name, &'static str)] {
        self.actives.usarts()
    }

    /// Every byte a USART sent after the ones `cursor` has read, up to the last whose stop bit
    /// ended at or before the last accepted step, and the cursor past them. So a byte appears at
    /// most one step after its stop bit ends, and once the session has ended every byte that
    /// ended by `now` is here. The bytes are in the order their stop bits ended, then in
    /// `usarts` order. On one USART that is the order they started. Solves nothing. A cursor
    /// past the end, from another session or a count the caller made, reads nothing and stays.
    /// `None` when `cursor` is before a byte `forget_serial` dropped.
    pub fn serial_since(&self, cursor: SerialCursor) -> Option<(&[SerialByte], SerialCursor)> {
        let cursor = u64::from(cursor);
        let read = if self.record.is_some() {
            read_retained(&self.serial, self.serial_forgotten, cursor)
        } else {
            read_drained(&self.serial, self.serial_forgotten, cursor)
        };
        read.map(|(bytes, end)| (bytes, SerialCursor(end)))
    }

    /// Forgets the serial bytes before `cursor`. An unrecorded session drains them. A recorded
    /// session only raises the read floor over the one list. Cursors keep counting from the
    /// first byte. A later `serial_since` is unchanged except on a cursor before `cursor`. A
    /// recorded finish still writes every delivered byte to `serial.csv`.
    pub fn forget_serial(&mut self, cursor: SerialCursor) {
        let cursor = u64::from(cursor);
        if self.record.is_some() {
            let len = self.serial.len();
            raise_floor(&mut self.serial_forgotten, len, cursor);
        } else {
            forget_prefix(&mut self.serial, &mut self.serial_forgotten, cursor);
        }
    }

    /// Advances to `stop` and returns the run folder `sim run` writes for the netlist this
    /// session ran, stamped `created_at`. Its `netlist.json` is the bytes `record_document` read
    /// when no input was applied, and otherwise the export of the ended session. `None` for a
    /// session `open` or `open_document` made, which records nothing.
    pub fn finish(mut self, created_at: Timestamp) -> Option<RunFolder> {
        self.record.as_ref()?;
        self.advance_to(self.transient.stop);
        let json = match self.opened.take() {
            Some(json) => json,
            None => self.export()?,
        };
        // `outcome` consumes the session, and `render` reads the netlist's version and seed.
        let netlist = self.circuit.netlist.clone();
        let mut diagnostics = Vec::new();
        let outcome = self.outcome(&mut diagnostics);
        let results = Results::new(outcome.observations, outcome.trace, diagnostics);
        Some(run_folder::render(
            &json,
            Some(&netlist),
            &results,
            created_at,
        ))
    }

    /// The observations and trace of a recorded session advanced to its end, and its warnings,
    /// ratings and fault in `diagnostics`. The trace holds the serial log when a chip has a USART,
    /// and the TWI phases when a chip has a TWI.
    pub(crate) fn outcome(self, diagnostics: &mut Vec<Diagnostic>) -> Outcome {
        let (State::Ended { at, row, end }, Some(record)) = (self.state, self.record) else {
            unreachable!("a recorded run finishes once it has ended");
        };
        diagnostics.extend(self.warnings);
        diagnostics.extend(self.ratings.findings());
        let (observations, mut trace) = record.finish(&self.circuit.netlist, at, &row, diagnostics);
        let usarts = self.actives.usarts();
        if !usarts.is_empty() {
            trace.attach_serial(Delivered::new(usarts.to_vec(), self.serial));
        }
        let twis = self.actives.twis();
        if !twis.is_empty() {
            trace.attach_i2c(I2c::new(twis.to_vec(), self.i2c));
        }
        if let End::Fault(fault) = end {
            diagnostics.push(fault);
        }
        Outcome {
            observations,
            trace: Some(trace),
            stats: self.stats,
        }
    }

    /// Ended at `stop`, running with the step after `accepted` solved, or ended at `accepted`
    /// when that step fails.
    fn solved(&mut self, accepted: Accepted) -> State {
        let end = if accepted.at == self.transient.stop {
            End::Stop
        } else {
            match self.next_landing(&accepted) {
                Ok(next) => {
                    return State::Running {
                        now: accepted.at,
                        accepted,
                        next,
                    };
                }
                Err(fault) => End::Fault(fault),
            }
        };
        State::Ended {
            at: accepted.at,
            row: accepted.row,
            end,
        }
    }

    /// The step after `accepted`. It writes back only the breakpoints and the cores run
    /// ahead, which a second call from the same accepted step leaves where they are.
    fn next_landing(&mut self, accepted: &Accepted) -> Result<Landing, Diagnostic> {
        if self.stats.landings >= self.budget.steps {
            return Err(Diagnostic::SolverStepLimit {
                at: accepted.at,
                limit: self.budget.steps,
            });
        }
        let circuit = &self.circuit;
        let mut plan = self.plan;
        let mut stiffened: Option<Segment> = None;
        // Newton iterations over every stalled attempt since the last accepted step. An attempt
        // rejected for its error converged, and does not count.
        let mut iterations = 0;
        let mut reach = accepted.reach;
        loop {
            let segment = stiffened.as_ref().unwrap_or(&accepted.segment);
            let h = plan.clip(
                accepted.at,
                self.breakpoints.next_after(&circuit.netlist, accepted.at),
            );
            let at = self.actives.horizon(Time::from_ps(accepted.at.as_ps() + h));
            reach = reach.max(at);
            let h = at.as_ps() - accepted.at.as_ps();
            let cores = self.actives.cores();
            match attempt(
                circuit,
                Some(&mut self.memo),
                accepted,
                segment,
                cores,
                h,
                &accepted.iterate,
                self.budget.step,
            ) {
                Ok(converged) => {
                    let error = lte::error(
                        &self.charges,
                        segment,
                        accepted.at,
                        &accepted.iterate.solution,
                        h,
                        &converged.iterate.solution,
                    );
                    match error.verdict(h) {
                        Verdict::Accept => {}
                        Verdict::Reject => {
                            plan = Plan::rejected(h, error.ratio, error.method);
                            continue;
                        }
                        Verdict::Stiffen(more) => {
                            self.stats.stiffens += 1;
                            stiffened = Some(segment.clone().stiffen(&more, h));
                            continue;
                        }
                    }
                    let levels = circuit
                        .watches
                        .levels(&converged.iterate.solution, Some(&accepted.levels));
                    let step_end = at;
                    let cut = levels != accepted.levels;
                    let (at, converged, levels) = if !cut {
                        (at, converged, levels)
                    } else {
                        let (located, converged) = watch::locate(
                            &circuit.watches,
                            &accepted.levels,
                            (accepted.at, &accepted.iterate.solution),
                            (at, converged),
                            |h| {
                                attempt(
                                    circuit,
                                    Some(&mut self.memo),
                                    accepted,
                                    segment,
                                    cores,
                                    h,
                                    &accepted.iterate,
                                    self.budget.probe,
                                )
                            },
                        )
                        .map_err(|(probe, failure)| {
                            probe_failure(circuit, accepted.at, probe, failure)
                        })?;
                        let levels = circuit
                            .watches
                            .levels(&converged.iterate.solution, Some(&accepted.levels));
                        (located, converged, levels)
                    };
                    // The row holds the chip states it was solved with.
                    let row = observe::observe(
                        circuit,
                        &self.columns,
                        &converged.iterate.solution,
                        at,
                        Side::Before,
                    )?;
                    return Ok(Landing {
                        h,
                        at,
                        step_end,
                        converged,
                        levels,
                        cut,
                        error,
                        row,
                        plan,
                        stiffened,
                        reach,
                    });
                }
                Err(Failure::Singular { row }) => {
                    return Err(Diagnostic::SolverSingular {
                        at,
                        unknown: circuit.layout.describe(&circuit.netlist, row),
                    });
                }
                Err(Failure::Stalled(stall)) => {
                    // A linear circuit stays non-finite at any step length.
                    if let Offender::NonFinite(_) = stall.offender
                        && circuit.linear
                    {
                        return Err(Diagnostic::SolverNonFinite {
                            at,
                            unknown: stall.offender.describe(circuit),
                        });
                    }
                    iterations += stall.iterations;
                    plan = Plan::halve(h).ok_or_else(|| Diagnostic::SolverConvergence {
                        at: accepted.at,
                        unknown: stall.offender.describe(circuit),
                        iterations,
                        gmin: None,
                        source_scale: 1.0,
                        solve: Solve::Step { step_ps: h },
                    })?;
                }
            }
        }
    }

    /// Records `landing`, takes its chip events and any restart, and solves the step after it.
    /// A landing within one step limit of `to`, the call's target, may be the call's last, so
    /// every core is kept there. An input after the call then runs a core again from that
    /// landing. A call to `stop` ends the session, so it keeps none.
    fn accept(&mut self, accepted: Accepted, landing: Landing, to: Time) -> State {
        let span = Span {
            circuit: &self.circuit,
            columns: &self.columns,
            cores: self.actives.cores(),
            budget: self.budget.row,
            from: &accepted,
            to: &landing,
        };
        if let Some(record) = &mut self.record
            && let Err(fault) = record.landed(&span)
        {
            return State::Ended {
                at: accepted.at,
                row: accepted.row,
                end: End::Fault(fault),
            };
        }
        self.ratings.sample(landing.at, &landing.row);
        // A refused step ends on the accepted row, so it is not counted. The count is before
        // the restart solve, so a failed restart still includes the landing it ends on.
        let left = accepted.restart_row.as_ref().unwrap_or(&accepted.row);
        trapezoid(
            &mut self.integrals,
            (accepted.at, left),
            (landing.at, &landing.row),
        );
        self.stats.landings += 1;
        let Landing {
            h,
            at,
            step_end,
            converged,
            levels,
            cut,
            error,
            row,
            plan,
            stiffened,
            reach,
        } = landing;
        let circuit = &self.circuit;
        let charges = &self.charges;
        let transient = &self.transient;
        let (ratio, method) = (error.ratio, error.method);
        // A watch cuts the step. A changed stamp, a chip's, an active chip's image, a
        // contact's or a source's, takes a restart solve.
        let chips = circuit.react(&accepted.chips, &accepted.levels, &levels);
        let (driven, stopped) = self.actives.land(
            circuit,
            at,
            &converged.iterate.solution,
            &accepted.levels,
            &levels,
            to < transient.stop && at.as_ps().saturating_add(transient.max_step) > to.as_ps(),
        );
        self.stats.stops += stopped;
        let restart = (self.breakpoints.is_edge(at) || chips != accepted.chips || driven)
            && at < transient.stop;
        let landed = at.as_ps() - accepted.at.as_ps();
        let segment = stiffened.unwrap_or(accepted.segment);
        let continued = |segment: Segment, last: Solution, next: &Solution| {
            let swings = if cut {
                segment.swings(charges, &last, landed, next)
            } else {
                error.swings
            };
            segment.advance(accepted.at, last, landed, swings)
        };
        let next = if restart {
            let held = restart_solve(
                circuit,
                &mut self.memo,
                at,
                &converged.iterate,
                chips,
                self.actives.cores(),
                &levels,
                self.budget.restart,
            )
            .and_then(|held| {
                let restart_row = observe::observe(
                    circuit,
                    &self.columns,
                    &held.iterate.solution,
                    at,
                    Side::After,
                )?;
                Ok((held, restart_row))
            });
            let (held, restart_row) = match held {
                Ok(held) => held,
                Err(fault) => {
                    return State::Ended {
                        at,
                        row,
                        end: End::Fault(fault),
                    };
                }
            };
            self.actives.input(circuit, at, &levels, &held.levels);
            let step = &converged.iterate.solution;
            let segment = continued(segment, accepted.iterate.solution, step).across(
                charges,
                step,
                &held.iterate.solution,
                transient.max_step,
            );
            Accepted {
                at,
                iterate: held.iterate,
                row,
                restart_row: Some(restart_row),
                segment,
                chips: held.chips,
                levels: held.levels,
                reach,
            }
        } else {
            Accepted {
                at,
                segment: continued(
                    segment,
                    accepted.iterate.solution,
                    &converged.iterate.solution,
                ),
                iterate: converged.iterate,
                row,
                restart_row: None,
                chips,
                levels: levels.clone(),
                reach,
            }
        };
        // A step cut short by a breakpoint or a watch keeps the step it wanted.
        self.plan = match (&next.segment, h == plan.h && at == step_end) {
            (Segment::Start { .. }, _) => Plan::RESTART,
            (_, true) => Plan::accepted(h, ratio, method, transient.max_step),
            (_, false) => plan,
        };
        // A flip the chips reacted to is recorded at `at`, even when the restart's
        // solve is back across the threshold. The restart's level is the next step's
        // prior, so the return is recorded on the next row.
        let recorded = next.levels.with_flips(&accepted.levels, &levels);
        let logic = circuit.logic_levels(&next.chips, &recorded);
        debug_assert_eq!(logic.len(), self.logic.len());
        if self.record.is_none() {
            for (signal, (&was, &is)) in self.logic.iter().zip(&logic).enumerate() {
                if was != is {
                    self.edges.push(Change {
                        at,
                        signal,
                        level: is,
                    });
                }
            }
        }
        self.logic = logic;
        if let Some(record) = &mut self.record {
            // Logic::set merges a same-time change, which would move a count cursor.
            debug_assert!(record.changes().last().is_none_or(|change| change.at < at));
            record.logic(at, &self.logic);
        }
        self.solved(next)
    }

    pub(crate) fn span<'s>(&'s self, from: &'s Accepted, to: &'s Landing) -> Span<'s> {
        Span {
            circuit: &self.circuit,
            columns: &self.columns,
            cores: self.actives.cores(),
            budget: self.budget.row,
            from,
            to,
        }
    }
}

/// From an accepted step to the landing after it, which no breakpoint falls inside.
pub(crate) struct Span<'s> {
    circuit: &'s Circuit,
    columns: &'s Columns,
    cores: &'s [active::Core],
    /// Newton's budget for each solve.
    budget: u32,
    pub(crate) from: &'s Accepted,
    pub(crate) to: &'s Landing,
}

pub(crate) enum Probed {
    Solved(Vec<Finite>),
    /// Both solves failed, so the row is on the line between the span's ends.
    Interpolated(Vec<Finite>),
}

impl Span<'_> {
    /// The row at `at`, inside the span. Solved from `from`, retried from the landing's solve,
    /// and on the line between the span's ends when both fail. It is never accepted.
    pub(crate) fn probe(&self, at: Time) -> Probed {
        match self
            .solve(at, &self.from.iterate)
            .or_else(|_| self.solve(at, &self.to.converged.iterate))
        {
            Ok(row) => Probed::Solved(row),
            Err(_) => {
                let from = self.from.restart_row.as_ref().unwrap_or(&self.from.row);
                Probed::Interpolated(line((self.from.at, from), (self.to.at, &self.to.row), at))
            }
        }
    }

    /// The row at `at` solved from `start` the way the landing was. An error names the unknown
    /// or the part the solve failed at.
    pub(crate) fn solve(&self, at: Time, start: &Iterate) -> Result<Vec<Finite>, Ref> {
        debug_assert!(
            self.from.at < at && at < self.to.at,
            "a row at {at:?} is not inside the span"
        );
        let h = at.as_ps() - self.from.at.as_ps();
        let segment = self.to.stiffened.as_ref().unwrap_or(&self.from.segment);
        let converged = attempt(
            self.circuit,
            None,
            self.from,
            segment,
            self.cores,
            h,
            start,
            self.budget,
        )
        .map_err(|failure| match failure {
            Failure::Singular { row } => self.circuit.layout.describe(&self.circuit.netlist, row),
            Failure::Stalled(stall) => stall.offender.describe(self.circuit),
        })?;
        observe::row(
            self.circuit,
            self.columns,
            &converged.iterate.solution,
            at,
            Side::Before,
        )
    }
}

/// A probe of `locate` that failed at `probe`. It is not retried shorter: the step it
/// re-solves already converged.
fn probe_failure(circuit: &Circuit, from: Time, probe: Time, failure: Failure) -> Diagnostic {
    match failure {
        Failure::Singular { row } => Diagnostic::SolverSingular {
            at: probe,
            unknown: circuit.layout.describe(&circuit.netlist, row),
        },
        Failure::Stalled(stall) => Diagnostic::SolverConvergence {
            at: probe,
            unknown: stall.offender.describe(circuit),
            iterations: stall.iterations,
            gmin: None,
            source_scale: 1.0,
            solve: Solve::Probe {
                step_ps: probe.as_ps() - from.as_ps(),
            },
        },
    }
}

/// A drained log, whose first item is absolute index `forgotten`.
fn read_drained<T>(items: &[T], forgotten: u64, cursor: u64) -> Option<(&[T], u64)> {
    read_window(items, forgotten, forgotten, cursor)
}

/// A log that still holds every entry. `forgotten` is only the read floor.
fn read_retained<T>(items: &[T], forgotten: u64, cursor: u64) -> Option<(&[T], u64)> {
    read_window(items, 0, forgotten, cursor)
}

/// The items from absolute index `cursor` on, and the index past the log. `items[0]` is
/// absolute index `base`. `None` when `cursor` is before `forgotten`. A `cursor` past the end
/// reads nothing and stays.
fn read_window<T>(items: &[T], base: u64, forgotten: u64, cursor: u64) -> Option<(&[T], u64)> {
    if cursor < forgotten {
        return None;
    }
    debug_assert!(cursor >= base);
    let end = base + items.len() as u64;
    match usize::try_from(cursor - base)
        .ok()
        .and_then(|start| items.get(start..))
    {
        Some(tail) => Some((tail, end)),
        None => Some((&[], cursor)),
    }
}

/// How many items `cursor` drops from the front of a drained log. `cursor` is clamped to the
/// end of the log. `None` when that point is at or before the forgotten count.
fn prefix_len(len: usize, forgotten: u64, cursor: u64) -> Option<usize> {
    let end = forgotten + len as u64;
    let drop_to = cursor.min(end);
    if drop_to <= forgotten {
        return None;
    }
    Some((drop_to - forgotten) as usize)
}

/// Moves the read floor of a log that still holds every entry. `len` is that log's length.
/// A `cursor` at or before the forgotten count changes nothing.
fn raise_floor(forgotten: &mut u64, len: usize, cursor: u64) {
    let drop_to = cursor.min(len as u64);
    if drop_to > *forgotten {
        *forgotten = drop_to;
    }
}

/// `docs/engine.md` states these sizes.
const _: () = assert!(match size_of::<usize>() {
    8 => size_of::<Change>() == 24 && size_of::<SerialByte>() == 16,
    4 => size_of::<Change>() == 16 && size_of::<SerialByte>() == 16,
    _ => false,
});

fn forget_prefix<T>(items: &mut Vec<T>, forgotten: &mut u64, cursor: u64) {
    let Some(n) = prefix_len(items.len(), *forgotten, cursor) else {
        return;
    };
    drop(items.drain(..n));
    *forgotten += n as u64;
}

/// The width is picoseconds, so the unit is the column's unit times picoseconds.
fn trapezoid(integrals: &mut [f64], from: (Time, &[Finite]), to: (Time, &[Finite])) {
    debug_assert_eq!(integrals.len(), from.1.len());
    debug_assert_eq!(integrals.len(), to.1.len());
    debug_assert!(to.0 >= from.0);
    let h = (to.0.as_ps() - from.0.as_ps()) as f64;
    for ((sum, left), right) in integrals.iter_mut().zip(from.1).zip(to.1) {
        *sum += h * 0.5 * (left.get() + right.get());
    }
}

/// The straight line from the row `from` to the row `to`, read at `at` between them.
fn line(from: (Time, &[Finite]), to: (Time, &[Finite]), at: Time) -> Vec<Finite> {
    let share = (at.as_ps() - from.0.as_ps()) as f64 / (to.0.as_ps() - from.0.as_ps()) as f64;
    from.1
        .iter()
        .zip(to.1)
        .map(|(a, b)| {
            Finite::new(a.get() * (1.0 - share) + b.get() * share)
                .expect("between two finite values")
        })
        .collect()
}

/// One step of `h` ps from `from`, each capacitor on its method in `segment`. Both are only
/// read, so a retry calls this again.
#[allow(clippy::too_many_arguments)]
pub(crate) fn attempt(
    circuit: &Circuit,
    memo: Option<&mut Memo>,
    from: &Accepted,
    segment: &Segment,
    cores: &[active::Core],
    h: u64,
    start: &Iterate,
    budget: u32,
) -> Result<Converged, Failure> {
    let conditions = Conditions {
        at: Time::from_ps(from.at.as_ps() + h),
        history: History::Step(Integration {
            method: segment.method(),
            stiff: segment.stiff(),
            h_ps: h,
            previous: &from.iterate.solution,
        }),
        chips: &from.chips,
        cores,
        ..Conditions::NOMINAL
    };
    newton(circuit, memo, &conditions, start.clone(), budget)
}

/// The solve after a switch edge, a source jump or a chip drive change at `at`: every capacitor
/// held at its voltage in `step`, the new contacts, sources and `chips` in force, settled on the operating point
/// ladder with `budget` per rung. `levels` were read on `step`. It is history for the next
/// step, not a row.
#[allow(clippy::too_many_arguments)]
fn restart_solve(
    circuit: &Circuit,
    memo: &mut Memo,
    at: Time,
    step: &Iterate,
    chips: Vec<chip::State>,
    cores: &[active::Core],
    levels: &Levels,
    budget: u32,
) -> Result<op::Settled, Diagnostic> {
    let conditions = Conditions {
        at,
        history: History::Restart {
            previous: &step.solution,
        },
        ..Conditions::NOMINAL
    };
    op::settle(
        circuit,
        Some(memo),
        conditions,
        step.clone(),
        chips,
        cores,
        Some(levels),
        budget,
    )
}
