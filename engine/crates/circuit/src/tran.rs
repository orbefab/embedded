use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};

use sim_core::diagnostic::{Diagnostic, Solve};
use sim_core::finite::Finite;
use sim_core::logic::Logic;
use sim_core::name::Ref;
use sim_core::netlist::{Edge, Model, Netlist, Transient};
use sim_core::run_folder::{Observations, OpPoint};
use sim_core::time::Time;
use sim_core::trace::Trace;

use crate::RunStats;
use crate::active::{self, Actives};
use crate::chip;
use crate::lte::{self, Charges, Segment, Verdict};
use crate::mna::Solution;
use crate::newton::{
    Circuit, Conditions, Converged, Failure, History, Integration, Iterate, Offender, Side, newton,
};
use crate::observe::{self, Columns};
use crate::op;
use crate::ratings::{Ratings, Timing};
use crate::step::{EDGE_STEP, Plan};
use crate::watch::{self, Levels};

pub(crate) const STEP_BUDGET: u32 = 50;

/// Accepted steps one transient run may take. The operating point at t = 0 is not one of
/// them. A dense switch or source schedule spends them on landings, and reaching the limit
/// before `stop` ends the run with `solver.step_limit`.
///
/// 100000000 sits far above the steps a run whose rows fit needs on ordinary circuits.
/// A release build reaches it in about 40 s on a resistive pulse and about 70 s on a 1 MHz
/// pin into a 10 ns load. At this step policy that pin costs about 171000 steps per
/// simulated ms, so the limit allows about 580 ms of it. The step policy, not this limit,
/// bounds an MCU-rate run.
pub(crate) const MAX_STEPS: u64 = 100_000_000;

/// Newton's budget for each step, each probe locating a chip event, each solve of a row, and
/// each rung of a restart's ladder, plus the accepted steps one run may take. A test lowers one
/// to make that solve fail, or `steps` to end a dense schedule at the step limit.
#[derive(Clone, Copy)]
struct Budget {
    step: u32,
    probe: u32,
    row: u32,
    restart: u32,
    steps: u64,
}

const BUDGET: Budget = Budget {
    step: STEP_BUDGET,
    probe: STEP_BUDGET,
    row: STEP_BUDGET,
    restart: op::OP_BUDGET,
    steps: MAX_STEPS,
};

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

/// The grid is checked against the trace's row limit first, before any solve. Then the
/// operating point, then steps to `stop`. Rating findings and any solver error go to
/// `diagnostics`.
pub(crate) fn run(
    netlist: &Netlist,
    transient: &Transient,
    rows: Rows,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    let columns = Columns::new(netlist);
    let limit = Trace::row_limit(columns.refs().len());
    if transient.rows() > limit as u64 {
        diagnostics.push(Diagnostic::TraceTooLarge {
            rows: transient.rows(),
            limit,
            columns: columns.refs().len(),
            at: None,
        });
        return Outcome {
            observations: Observations::Nothing,
            trace: None,
            stats: RunStats::default(),
        };
    }
    run_with(
        netlist,
        transient,
        Vec::new(),
        BUDGET,
        rows,
        columns,
        diagnostics,
    )
}

fn run_with(
    netlist: &Netlist,
    transient: &Transient,
    actives: Vec<active::Chip>,
    budget: Budget,
    rows: Rows,
    columns: Columns,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    match Session::open_recorded(netlist, transient, actives, budget, columns, rows) {
        Ok(mut session) => {
            session.advance_to(transient.stop);
            session.finish(diagnostics)
        }
        Err(failed) => {
            diagnostics.push(failed);
            Outcome {
                observations: Observations::Nothing,
                trace: None,
                stats: RunStats::default(),
            }
        }
    }
}

/// Everything an accepted step feeds.
struct Record {
    trace: Trace,
    transient: Transient,
    rows: Rows,
    /// `trace.row_interpolated` for each row whose solve failed twice.
    interpolated: Vec<Diagnostic>,
    ratings: Ratings,
    op: OpPoint,
    /// `None` without a chip.
    logic: Option<Logic>,
}

impl Record {
    /// A record whose first step is the operating point at t = 0, read as `row`.
    fn start(
        netlist: &Netlist,
        transient: &Transient,
        columns: &Columns,
        rows: Rows,
        row: &[Finite],
    ) -> Result<Record, Diagnostic> {
        let mut record = Record {
            trace: Trace::new(columns.refs().to_vec(), transient),
            transient: *transient,
            rows,
            interpolated: Vec::new(),
            ratings: Ratings::new(netlist, columns, Timing::Run),
            op: observe::op_point(columns, row),
            logic: None,
        };
        record.accept(Vec::new(), Time::ZERO, row)?;
        Ok(record)
    }

    /// The rows inside `span`, then the step it lands on.
    fn landed(&mut self, span: &Span<'_>) -> Result<(), Diagnostic> {
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

    /// Ratings sample every accepted step. `probes` are the rows before it, which close
    /// intervals and sample nothing else. A non-row step folds into the open interval, and a
    /// row closes it. A row past the trace's limit is `trace.too_large`, and the step is
    /// neither stored nor folded.
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
        self.ratings.sample(at, row);
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
    fn finish(
        self,
        netlist: &Netlist,
        at: Time,
        row: &[Finite],
        diagnostics: &mut Vec<Diagnostic>,
    ) -> (Observations, Trace) {
        diagnostics.extend(self.ratings.finish());
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
        };
        (observations, trace)
    }

    /// Every chip signal's level from `at` on: one `Ref::Logic` per signal, chips in part order.
    fn logic(&mut self, at: Time, circuit: &Circuit, chips: &[chip::State], levels: &Levels) {
        let now = circuit.logic_levels(chips, levels);
        match &mut self.logic {
            Some(logic) => {
                for (signal, level) in now.into_iter().enumerate() {
                    logic.set(at, signal, level);
                }
            }
            None if now.is_empty() => {}
            None => self.logic = Some(Logic::new(circuit.logic_refs(), now)),
        }
    }
}

/// The whole state a step starts from. Replaced, never mutated, on accept. A retry, a
/// rejected step or a probe only reads it.
struct Accepted {
    at: Time,
    /// After a restart, the restart solve at `at`, not the row at `at`.
    iterate: Iterate,
    /// The row at `at`, read before any restart there.
    row: Vec<Finite>,
    /// After a restart, `iterate` read on the interval that starts at `at`.
    restart_row: Option<Vec<Finite>>,
    segment: Segment,
    /// Each chip's state, by slot.
    chips: Vec<chip::State>,
    /// Each watch's level at `at`.
    levels: Levels,
}

/// A step from an accepted one that converged, passed its error check, is located at its
/// first watch flip, and is not yet accepted.
struct Landing {
    /// The step attempted, before a watch cut it.
    h: u64,
    at: Time,
    /// Where the step attempted ends.
    step_end: Time,
    converged: Converged,
    /// Each watch's level at `at`.
    levels: Levels,
    /// Whether a watch cut the step.
    cut: bool,
    error: lte::Error,
    /// The row at `at`, read on `converged`.
    row: Vec<Finite>,
    /// The plan the step was attempted with, after any reject or stall.
    plan: Plan,
    /// The accepted segment with the capacitors a stiffen holds on backward Euler, when one did.
    stiffened: Option<Segment>,
}

/// A transient run held between calls. `advance_to` accepts every step that lands by its time.
/// The step after the last one it accepted is solved and waits.
struct Session<'n> {
    circuit: Circuit<'n>,
    columns: Columns,
    transient: Transient,
    budget: Budget,
    charges: Charges,
    breakpoints: Breakpoints<'n>,
    actives: Actives,
    /// The plan for the step after the last accepted one, before any reject or stall.
    plan: Plan,
    stats: RunStats,
    record: Record,
    state: State,
}

#[allow(clippy::large_enum_variant)]
enum State {
    /// `accepted.at <= now < next.at`. `next` is solved from `accepted` and not accepted.
    Running {
        accepted: Accepted,
        now: Time,
        next: Landing,
    },
    /// At `stop`, or at the last step the trace holds when a fault ended the run. After a failed
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

enum End {
    Stop,
    /// A solver error, the step limit's included, or `trace.too_large`.
    Fault(Diagnostic),
}

impl<'n> Session<'n> {
    /// Solves and records t = 0, then solves the step after it. Fails only when t = 0 fails.
    fn open_recorded(
        netlist: &'n Netlist,
        transient: &Transient,
        actives: Vec<active::Chip>,
        budget: Budget,
        columns: Columns,
        rows: Rows,
    ) -> Result<Session<'n>, Diagnostic> {
        let circuit = Circuit::with_actives(netlist, actives)?;
        let charges = Charges::new(netlist, &circuit.layout);
        let start = Iterate::initial(&circuit);
        let start = op::settle(
            &circuit,
            Conditions::NOMINAL,
            start,
            circuit.power_on(),
            &Actives::power_on(&circuit),
            None,
            op::OP_BUDGET,
        )?;
        let row = observe::observe(
            &circuit,
            &columns,
            &start.iterate.solution,
            Time::ZERO,
            Side::Before,
        )?;
        let mut record = Record::start(netlist, transient, &columns, rows, &row)?;
        record.logic(Time::ZERO, &circuit, &start.chips, &start.levels);
        let actives = Actives::new(&circuit, &start.levels);
        let breakpoints = Breakpoints::new(netlist, transient.stop);
        let segment = Segment::start(&charges, &start.iterate.solution);
        let accepted = Accepted {
            at: Time::ZERO,
            iterate: start.iterate,
            row,
            restart_row: None,
            segment,
            chips: start.chips,
            levels: start.levels,
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
            record,
            state: State::TAKEN,
        };
        session.state = session.solved(accepted);
        Ok(session)
    }

    /// Accepts every step that lands at or before `to`, and `stop` caps `to`. A running session
    /// then stands at `to`, and a time at or before `now` does nothing. A fault ends the session
    /// at its last accepted step.
    fn advance_to(&mut self, to: Time) {
        let to = to.min(self.transient.stop);
        let mut state = std::mem::replace(&mut self.state, State::TAKEN);
        loop {
            state = match state {
                State::Running { accepted, next, .. } if next.at <= to => {
                    self.accept(accepted, next)
                }
                _ => break,
            };
        }
        if let State::Running { now, .. } = &mut state {
            *now = (*now).max(to);
        }
        self.state = state;
    }

    /// The observations and trace of a session advanced to its end.
    fn finish(self, diagnostics: &mut Vec<Diagnostic>) -> Outcome {
        let State::Ended { at, row, end } = self.state else {
            unreachable!("a run finishes once it has ended");
        };
        let (observations, trace) = self
            .record
            .finish(self.circuit.netlist, at, &row, diagnostics);
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

    /// The step after `accepted`. It writes back only the breakpoint cursor and the cores run
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
        loop {
            let segment = stiffened.as_ref().unwrap_or(&accepted.segment);
            let h = plan.clip(accepted.at, self.breakpoints.next_after(accepted.at));
            let at = self.actives.horizon(Time::from_ps(accepted.at.as_ps() + h));
            let h = at.as_ps() - accepted.at.as_ps();
            let cores = self.actives.cores();
            match attempt(
                circuit,
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
                    });
                }
                Err(Failure::Singular { row }) => {
                    return Err(Diagnostic::SolverSingular {
                        at,
                        unknown: circuit.layout.describe(circuit.netlist, row),
                    });
                }
                Err(Failure::Stalled(stall)) => {
                    // Without a junction the system is linear, and a shorter step does not make it finite.
                    if let Offender::NonFinite(_) = stall.offender
                        && circuit.layout.junctions() == 0
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
    fn accept(&mut self, accepted: Accepted, landing: Landing) -> State {
        let span = Span {
            circuit: &self.circuit,
            columns: &self.columns,
            cores: self.actives.cores(),
            budget: self.budget.row,
            from: &accepted,
            to: &landing,
        };
        if let Err(fault) = self.record.landed(&span) {
            return State::Ended {
                at: accepted.at,
                row: accepted.row,
                end: End::Fault(fault),
            };
        }
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
        self.record.logic(at, circuit, &next.chips, &recorded);
        self.solved(next)
    }
}

/// From an accepted step to the landing after it, which no breakpoint falls inside.
struct Span<'s> {
    circuit: &'s Circuit<'s>,
    columns: &'s Columns,
    cores: &'s [active::Core],
    /// Newton's budget for each solve.
    budget: u32,
    from: &'s Accepted,
    to: &'s Landing,
}

enum Probed {
    Solved(Vec<Finite>),
    /// Both solves failed, so the row is on the line between the span's ends.
    Interpolated(Vec<Finite>),
}

impl Span<'_> {
    /// The row at `at`, inside the span. Solved from `from`, retried from the landing's solve,
    /// and on the line between the span's ends when both fail. It is never accepted.
    fn probe(&self, at: Time) -> Probed {
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
    fn solve(&self, at: Time, start: &Iterate) -> Result<Vec<Finite>, Ref> {
        debug_assert!(
            self.from.at < at && at < self.to.at,
            "a row at {at:?} is not inside the span"
        );
        let h = at.as_ps() - self.from.at.as_ps();
        let segment = self.to.stiffened.as_ref().unwrap_or(&self.from.segment);
        let converged = attempt(
            self.circuit,
            self.from,
            segment,
            self.cores,
            h,
            start,
            self.budget,
        )
        .map_err(|failure| match failure {
            Failure::Singular { row } => self.circuit.layout.describe(self.circuit.netlist, row),
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
            unknown: circuit.layout.describe(circuit.netlist, row),
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
fn attempt(
    circuit: &Circuit,
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
    newton(circuit, &conditions, start.clone(), budget)
}

/// The solve after a switch edge, a source jump or a chip drive change at `at`: every capacitor
/// held at its voltage in `step`, the new contacts, sources and `chips` in force, settled on the operating point
/// ladder with `budget` per rung. `levels` were read on `step`. It is history for the next
/// step, not a row.
fn restart_solve(
    circuit: &Circuit,
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
        conditions,
        step.clone(),
        chips,
        cores,
        Some(levels),
        budget,
    )
}

/// Where a step must land: every contact edge and source jump, 1 ns after t = 0 and after each
/// edge, and `stop`. The deck ramps each contact and source over that nanosecond. A lazy merge
/// of one cursor per part, so no edge list is built and none is capped. Time only moves
/// forward.
struct Breakpoints<'n> {
    stop: Time,
    /// Each cursor's next edge before `stop`, earliest first, with the cursor's index.
    edges: BinaryHeap<Reverse<(Time, usize)>>,
    cursors: Vec<Box<dyn Iterator<Item = Time> + 'n>>,
    /// 1 ns after t = 0 and after each edge passed, before `stop`, ascending.
    ramp_ends: VecDeque<Time>,
}

impl<'n> Breakpoints<'n> {
    fn new(netlist: &'n Netlist, stop: Time) -> Breakpoints<'n> {
        let mut cursors: Vec<Box<dyn Iterator<Item = Time> + 'n>> = Vec::new();
        let mut edges = BinaryHeap::new();
        for (_, part) in netlist.parts() {
            let mut cursor: Box<dyn Iterator<Item = Time> + 'n> = match &part.model {
                Model::Switch(switch) => Box::new(
                    switch
                        .schedule
                        .edges()
                        .iter()
                        .map(|edge| edge.at)
                        .take_while(move |&at| at < stop),
                ),
                model => {
                    let Some(waveform) = model.waveform() else {
                        continue;
                    };
                    Box::new(waveform.jumps_before(stop))
                }
            };
            if let Some(at) = cursor.next() {
                edges.push(Reverse((at, cursors.len())));
                cursors.push(cursor);
            }
        }
        let mut breakpoints = Breakpoints {
            stop,
            edges,
            cursors,
            ramp_ends: VecDeque::new(),
        };
        breakpoints.ramp_after(Time::ZERO);
        breakpoints
    }

    /// The earliest landing after `now`, which is at most `stop`.
    fn next_after(&mut self, now: Time) -> Time {
        while let Some(&Reverse((at, index))) = self.edges.peek()
            && at <= now
        {
            self.edges.pop();
            self.ramp_after(at);
            if let Some(next) = self.cursors[index].next() {
                self.edges.push(Reverse((next, index)));
            }
        }
        while self.ramp_ends.front().is_some_and(|&end| end <= now) {
            self.ramp_ends.pop_front();
        }
        let edge = self.edges.peek().map_or(self.stop, |&Reverse((at, _))| at);
        let ramp_end = self.ramp_ends.front().copied().unwrap_or(self.stop);
        edge.min(ramp_end)
    }

    /// Whether a contact edge or a source jump falls at `at`, a time `next_after` returned.
    fn is_edge(&self, at: Time) -> bool {
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
                    .edges()
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
