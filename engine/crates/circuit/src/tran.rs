use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};

use sim_core::diagnostic::Diagnostic;
use sim_core::finite::Finite;
use sim_core::logic::{Change, Level, Logic};
use sim_core::name::Ref;
use sim_core::netlist::{Document, Edge, Model, Netlist, PartId, Transient};
use sim_core::run_folder::{Observations, OpPoint};
use sim_core::time::Time;
use sim_core::trace::Trace;

use crate::RunStats;
use crate::observe::{self, Columns};
use crate::session::{BUDGET, Probed, Session, Span};
use crate::step::EDGE_STEP;
use crate::topology;

/// After an error `observations` hold what the run reached, and the trace keeps the rows
/// before it.
pub(crate) struct Outcome {
    pub(crate) observations: Observations,
    pub(crate) trace: Option<Trace>,
    pub(crate) stats: RunStats,
}

/// Which accepted steps the trace keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rows {
    /// t = 0, every multiple of the grid, and `stop`. A row time no step lands on is a probe.
    Grid,
    /// Every accepted step, for the solver's tests. The steps are the same as `Grid`'s. A run
    /// with more steps than the row limit ends with `trace.too_large` and keeps the rows
    /// it wrote. `t_ps` is the step that did not fit.
    #[cfg(any(test, feature = "step-rows"))]
    Steps,
}

impl Rows {
    fn keeps(self, transient: &Transient, at: Time) -> bool {
        match self {
            Rows::Grid => transient.is_row(at),
            #[cfg(any(test, feature = "step-rows"))]
            Rows::Steps => true,
        }
    }

    /// The row times in `(from, to)`, which no accepted step lands on.
    fn rows_in(self, transient: Transient, from: Time, to: Time) -> impl Iterator<Item = Time> {
        let first = match self {
            Rows::Grid => Some(transient.next_row_after(from)),
            #[cfg(any(test, feature = "step-rows"))]
            Rows::Steps => None,
        };
        std::iter::successors(first, move |&row| Some(transient.next_row_after(row)))
            .take_while(move |&row| row < to)
    }
}

/// The recorded session on a transient, and the one way `simulate` and
/// `Session::record_document` open one. The session holds the netlist's floating-net warnings,
/// which its run reports. A grid with more rows than a trace holds is `trace.too_large`, before
/// any solve. Then the operating point and the step after it.
// With 32-bit pointers the error is under the lint's size limit, and the lint stays silent.
#[cfg_attr(
    target_pointer_width = "64",
    expect(
        clippy::result_large_err,
        reason = "the session in Ok is larger than the error, so boxing it saves nothing"
    )
)]
pub(crate) fn record(
    netlist: Netlist,
    transient: &Transient,
    rows: Rows,
    document: Option<Document>,
) -> Result<Session, Unopened> {
    let columns = Columns::new(&netlist);
    let limit = Trace::row_limit(columns.refs().len());
    if transient.rows() > limit as u64 {
        // `start` is not called, so the floating-net warnings are read here.
        return Err(Unopened {
            error: Diagnostic::TraceTooLarge {
                rows: transient.rows(),
                limit,
                columns: columns.refs().len(),
                at: None,
            },
            warnings: topology::floating_nets(&netlist),
        });
    }
    Session::start(
        netlist,
        transient,
        Vec::new(),
        BUDGET,
        columns,
        Some(rows),
        document,
    )
    .map_err(|failure| Unopened {
        error: failure.error,
        warnings: failure.warnings,
    })
}

/// The error that kept `record` from opening a session, and the warnings a run reports with it.
#[derive(Debug)]
pub(crate) struct Unopened {
    pub(crate) error: Diagnostic,
    pub(crate) warnings: Vec<Diagnostic>,
}

/// `record`, then steps to `stop`. The run's diagnostics go to `diagnostics`.
pub(crate) fn run(
    netlist: &Netlist,
    transient: &Transient,
    rows: Rows,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    to_stop(
        record(netlist.clone(), transient, rows, None),
        transient,
        diagnostics,
    )
}

/// A recorded session stepped to `stop`, or the warnings and error of one `record` did not open.
fn to_stop(
    recorded: Result<Session, Unopened>,
    transient: &Transient,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    match recorded {
        Ok(mut session) => {
            session.advance_to(transient.stop);
            session.outcome(diagnostics)
        }
        Err(unopened) => {
            diagnostics.extend(unopened.warnings);
            diagnostics.push(unopened.error);
            Outcome {
                observations: Observations::Nothing,
                trace: None,
                stats: RunStats::default(),
            }
        }
    }
}

/// Everything an accepted step feeds.
pub(crate) struct Record {
    trace: Trace,
    transient: Transient,
    rows: Rows,
    /// `trace.row_interpolated` for each row whose solve failed twice.
    interpolated: Vec<Diagnostic>,
    op: OpPoint,
    /// `None` without a chip.
    logic: Option<Logic>,
}

impl Record {
    /// A record whose first step is the operating point at t = 0, read as `row`.
    pub(crate) fn start(
        transient: &Transient,
        columns: &Columns,
        rows: Rows,
        row: &[Finite],
        signals: &[Ref],
        levels: &[Level],
    ) -> Result<Record, Diagnostic> {
        let mut record = Record {
            trace: Trace::new(columns.refs().to_vec(), transient),
            transient: *transient,
            rows,
            interpolated: Vec::new(),
            op: observe::op_point(columns, row),
            logic: (!signals.is_empty()).then(|| Logic::new(signals.to_vec(), levels.to_vec())),
        };
        record.accept(Vec::new(), Time::ZERO, row)?;
        Ok(record)
    }

    /// The rows inside `span`, then the step it lands on.
    pub(crate) fn landed(&mut self, span: &Span<'_>) -> Result<(), Diagnostic> {
        let mut probes = Vec::new();
        // The step converged, so a row that fails both solves never rejects it.
        for at in self.rows.rows_in(self.transient, span.from.at, span.to.at) {
            let probe = match span.probe(at) {
                Probed::Solved(probe) => probe,
                Probed::Interpolated(probe) => {
                    self.interpolated.push(Diagnostic::TraceRowInterpolated {
                        at,
                        from: span.from.at,
                        to: span.to.at,
                    });
                    probe
                }
            };
            probes.push((at, probe));
        }
        self.accept(probes, span.to.at, &span.to.row)
    }

    /// `probes` are the rows before the step. They close intervals. They are not accepted
    /// steps, so the session does not sample a rating on them. A non-row step folds into the
    /// open interval, and a row closes it. A row past the trace's limit is `trace.too_large`,
    /// and the step is neither stored nor folded.
    fn accept(
        &mut self,
        probes: Vec<(Time, Vec<Finite>)>,
        at: Time,
        row: &[Finite],
    ) -> Result<(), Diagnostic> {
        let kept = self.rows.keeps(&self.transient, at);
        if kept {
            let columns = self.trace.columns().len();
            let limit = Trace::row_limit(columns);
            if self.trace.rows() >= limit {
                return Err(Diagnostic::TraceTooLarge {
                    rows: self.trace.rows() as u64 + 1,
                    limit,
                    columns,
                    at: Some(at),
                });
            }
        }
        for (probe_at, probe) in probes {
            self.trace.push(probe_at, &probe);
        }
        if kept {
            self.trace.push(at, row);
        } else {
            self.trace.sample(row);
        }
        Ok(())
    }

    /// The one constructor of a transient's observations, after success and after an error.
    /// The last accepted step, at `at` with `row`, is the last row. `end` is its time, the
    /// switches are the edges before it, and the logic summary comes from the `Logic` the
    /// trace keeps.
    pub(crate) fn finish(
        self,
        netlist: &Netlist,
        at: Time,
        row: &[Finite],
        diagnostics: &mut Vec<Diagnostic>,
    ) -> (Observations, Trace) {
        diagnostics.extend(self.interpolated);
        let mut trace = self.trace;
        if !self.rows.keeps(&self.transient, at) {
            trace.push(at, row);
        }
        let end = trace.end().expect("the operating point is row 0");
        let summary = self.logic.as_ref().map(Logic::summary);
        if let Some(logic) = self.logic {
            trace.attach_logic(logic);
        }
        let observations = Observations::Transient {
            end,
            op: self.op,
            switches: switches(netlist, end),
            logic: summary,
            displays: None,
        };
        (observations, trace)
    }

    /// Each chip signal's level from `at` on, in the order of the signals `start` took.
    pub(crate) fn logic(&mut self, at: Time, levels: &[Level]) {
        if let Some(logic) = &mut self.logic {
            for (signal, &level) in levels.iter().enumerate() {
                logic.set(at, signal, level);
            }
        }
    }

    /// The changes `logic.vcd` is written from, in signal order within one time. Empty when
    /// the netlist has no chip.
    pub(crate) fn changes(&self) -> &[Change] {
        match &self.logic {
            Some(logic) => logic.changes(),
            None => &[],
        }
    }
}

/// Where a step must land: every contact edge, source jump, and potentiometer move,
/// 1 ns after t = 0 and after each one, and `stop`. The deck ramps a contact or a
/// source over that nanosecond. It skips a moving potentiometer. Each part keeps its
/// next edge only, so no edge list is built and none is capped. Time only moves forward.
pub(crate) struct Breakpoints {
    stop: Time,
    /// Each part's next edge before `stop`, earliest first. A part has at most one entry, so
    /// no part is walked twice.
    edges: BinaryHeap<Reverse<(Time, PartId)>>,
    /// 1 ns after t = 0 and after each edge passed, before `stop`, ascending.
    ramp_ends: VecDeque<Time>,
}

impl Breakpoints {
    pub(crate) fn new(netlist: &Netlist, stop: Time) -> Breakpoints {
        let mut edges = BinaryHeap::new();
        for (id, part) in netlist.parts() {
            if let Some(at) = part.model.next_change_after(Time::ZERO)
                && at < stop
            {
                edges.push(Reverse((at, id)));
            }
        }
        let mut breakpoints = Breakpoints {
            stop,
            edges,
            ramp_ends: VecDeque::new(),
        };
        breakpoints.ramp_after(Time::ZERO);
        breakpoints
    }

    /// The earliest landing after `now`, which is at most `stop`.
    pub(crate) fn next_after(&mut self, netlist: &Netlist, now: Time) -> Time {
        while let Some(&Reverse((at, id))) = self.edges.peek()
            && at <= now
        {
            self.edges.pop();
            self.ramp_after(at);
            if let Some(next) = netlist.part(id).model.next_change_after(at)
                && next < self.stop
            {
                self.edges.push(Reverse((next, id)));
            }
        }
        while self.ramp_ends.front().is_some_and(|&end| end <= now) {
            self.ramp_ends.pop_front();
        }
        let edge = self.edges.peek().map_or(self.stop, |&Reverse((at, _))| at);
        let ramp_end = self.ramp_ends.front().copied().unwrap_or(self.stop);
        edge.min(ramp_end)
    }

    /// Part `id`'s edge at `at`, appended after every change it had, which is before `stop`. A
    /// part with an entry waits on an earlier change, and its walk reaches `at` in turn.
    pub(crate) fn arm(&mut self, id: PartId, at: Time) {
        if !self.edges.iter().any(|&Reverse((_, part))| part == id) {
            self.edges.push(Reverse((at, id)));
        }
    }

    /// Whether an edge, a jump, or a move falls at `at`, a time `next_after` returned.
    pub(crate) fn is_edge(&self, at: Time) -> bool {
        self.edges
            .peek()
            .is_some_and(|&Reverse((edge, _))| edge == at)
    }

    fn ramp_after(&mut self, edge: Time) {
        let Some(end) = edge.as_ps().checked_add(EDGE_STEP) else {
            return;
        };
        if end < self.stop.as_ps() && self.ramp_ends.back().is_none_or(|back| back.as_ps() < end) {
            self.ramp_ends.push_back(Time::from_ps(end));
        }
    }
}

/// Every contact edge before `end`, per switch.
fn switches(netlist: &Netlist, end: Time) -> BTreeMap<Ref, Vec<Edge>> {
    netlist
        .parts()
        .filter_map(|(_, part)| match &part.model {
            Model::Switch(switch) => Some((
                Ref::Part(part.name.clone()),
                switch
                    .schedule
                    .events()
                    .iter()
                    .filter(|edge| edge.at < end)
                    .copied()
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
