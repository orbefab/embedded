use sim_core::diagnostic::{Diagnostic, Solve};
use sim_core::finite::Finite;
use sim_core::logic::Level;
use sim_core::name::Ref;
use sim_core::netlist::{Analysis, Document, Netlist, Transient};
use sim_core::time::Time;

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
use crate::step::Plan;
use crate::tran::{Breakpoints, Outcome, Record, Rows};
use crate::watch::{self, Levels};

mod input;

pub use input::{Applied, Input, Rejected, Setting};

const STEP_BUDGET: u32 = 50;

/// Accepted steps one transient run may take. The operating point at t = 0 is not one of
/// them. A dense switch or source schedule spends them on landings, and reaching the limit
/// before `stop` ends the run with `solver.step_limit`.
///
/// 100000000 sits far above the steps a run whose rows fit needs on ordinary circuits.
/// A release build reaches it in about 40 s on a resistive pulse and about 70 s on a 1 MHz
/// pin into a 10 ns load. At this step policy that pin costs about 171000 steps per
/// simulated ms, so the limit allows about 580 ms of it. The step policy, not this limit,
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
    pub(crate) record: Option<Record>,
    /// The JSON the session opened from, which `export` writes back.
    document: Option<Document>,
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
    /// that is not finite.
    Failed(Diagnostic),
    /// The document `open_document` read is not a valid netlist.
    Invalid(Vec<Diagnostic>),
}

/// Every analog column at one time, read in one call: each net's voltage, then each part's `i`,
/// `v` and `p`. The columns and their order are the trace's.
pub struct Probe<'s> {
    at: Time,
    columns: &'s Columns,
    values: Vec<Finite>,
    interpolated: bool,
}

impl std::fmt::Debug for Probe<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe")
            .field("at", &self.at)
            .field("values", &self.values)
            .field("interpolated", &self.interpolated)
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
}

impl Session {
    /// Solves the operating point at t = 0 and the step after it, and keeps no trace. The step
    /// after t = 0 can fail, and then the session it returns has faulted.
    pub fn open(netlist: Netlist) -> Result<Session, Refused> {
        Session::open_with(netlist, None)
    }

    /// `open` on the netlist `json` parses to, keeping `json` so `export` can write it back.
    pub fn open_document(json: &[u8]) -> Result<Session, Refused> {
        let (document, netlist) = Document::parse(json).map_err(Refused::Invalid)?;
        Session::open_with(netlist, Some(document))
    }

    fn open_with(netlist: Netlist, document: Option<Document>) -> Result<Session, Refused> {
        let transient = match netlist.analysis() {
            Analysis::Transient(transient) => *transient,
            Analysis::Op => return Err(Refused::Op),
        };
        let columns = Columns::new(&netlist);
        Session::start(
            netlist,
            &transient,
            Vec::new(),
            BUDGET,
            columns,
            None,
            document,
        )
        .map_err(Refused::Failed)
    }

    pub(crate) fn open_recorded(
        netlist: Netlist,
        transient: &Transient,
        actives: Vec<active::Chip>,
        budget: Budget,
        columns: Columns,
        rows: Rows,
        document: Option<Document>,
    ) -> Result<Session, Diagnostic> {
        Session::start(
            netlist,
            transient,
            actives,
            budget,
            columns,
            Some(rows),
            document,
        )
    }

    /// Solves t = 0 and records it when `rows` is set, then solves the step after it. Fails only
    /// when t = 0 fails.
    fn start(
        netlist: Netlist,
        transient: &Transient,
        actives: Vec<active::Chip>,
        budget: Budget,
        columns: Columns,
        rows: Option<Rows>,
        document: Option<Document>,
    ) -> Result<Session, Diagnostic> {
        let circuit = Circuit::with_actives(netlist, actives)?;
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
        )?;
        let row = observe::observe(
            &circuit,
            &columns,
            &start.iterate.solution,
            Time::ZERO,
            Side::Before,
        )?;
        let signals = circuit.logic_refs();
        let logic = circuit.logic_levels(&start.chips, &start.levels);
        let record = rows
            .map(|rows| {
                Record::start(
                    &circuit.netlist,
                    transient,
                    &columns,
                    rows,
                    &row,
                    &signals,
                    &logic,
                )
            })
            .transpose()?;
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
            record,
            document,
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

    pub fn status(&self) -> Status {
        match &self.state {
            State::Running { .. } => Status::Running,
            State::Ended { end: End::Stop, .. } => Status::Stopped,
            State::Ended {
                end: End::Fault(_), ..
            } => Status::Faulted,
        }
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
    /// at its last accepted step.
    pub fn advance_to(&mut self, to: Time) -> Status {
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
        self.status()
    }

    /// Every analog column at `now`. On an accepted step it is that step's row. Between steps it
    /// is solved from the last accepted step the way a row is, so at a row time it is that row.
    pub fn probe(&self) -> Probe<'_> {
        let (at, values, interpolated) = match &self.state {
            State::Running {
                accepted,
                now,
                next,
            } if *now > accepted.at => match self.span(accepted, next).probe(*now) {
                Probed::Solved(values) => (*now, values, false),
                Probed::Interpolated(values) => (*now, values, true),
            },
            State::Running { accepted, .. } => (accepted.at, accepted.row.clone(), false),
            State::Ended { at, row, .. } => (*at, row.clone(), false),
        };
        Probe {
            at,
            columns: &self.columns,
            values,
            interpolated,
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

    /// The observations and trace of a recorded session advanced to its end.
    pub(crate) fn finish(self, diagnostics: &mut Vec<Diagnostic>) -> Outcome {
        let (State::Ended { at, row, end }, Some(record)) = (self.state, self.record) else {
            unreachable!("a recorded run finishes once it has ended");
        };
        let (observations, trace) = record.finish(&self.circuit.netlist, at, &row, diagnostics);
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
    fn accept(&mut self, accepted: Accepted, landing: Landing) -> State {
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
        self.logic = circuit.logic_levels(&next.chips, &recorded);
        if let Some(record) = &mut self.record {
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
