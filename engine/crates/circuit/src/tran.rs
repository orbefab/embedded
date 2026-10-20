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

use crate::active::{self, Actives};
use crate::chip;
use crate::lte::{self, Charges, Segment, Verdict};
use crate::newton::{
    Circuit, Conditions, Converged, Failure, History, Integration, Iterate, Offender, newton,
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

/// Newton's budget for each step, each probe locating a chip event, and each rung of a
/// restart's ladder, plus the accepted steps one run may take. A test lowers one to make
/// that solve fail, or `steps` to end a dense schedule at the step limit.
#[derive(Clone, Copy)]
struct Budget {
    step: u32,
    probe: u32,
    restart: u32,
    steps: u64,
}

const BUDGET: Budget = Budget {
    step: STEP_BUDGET,
    probe: STEP_BUDGET,
    restart: op::OP_BUDGET,
    steps: MAX_STEPS,
};

/// After an error `observations` hold what the run reached, and the trace keeps the rows
/// before it.
pub(crate) struct Outcome {
    pub(crate) observations: Observations,
    pub(crate) trace: Option<Trace>,
}

/// Which accepted steps the trace keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rows {
    /// t = 0, every multiple of the grid, and `stop`.
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
        };
    }
    run_with(
        netlist,
        transient,
        Vec::new(),
        BUDGET,
        rows,
        &columns,
        diagnostics,
    )
}

fn run_with(
    netlist: &Netlist,
    transient: &Transient,
    actives: Vec<active::Chip>,
    budget: Budget,
    rows: Rows,
    columns: &Columns,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    let mut record = Record {
        trace: Trace::new(columns.refs().to_vec(), transient),
        transient: *transient,
        rows,
        last_step: None,
        ratings: Ratings::new(netlist, columns, Timing::Run),
        columns,
        op: None,
        logic: None,
    };
    let result = steps(netlist, transient, actives, budget, &mut record);
    let outcome = record.finish(netlist, diagnostics);
    diagnostics.extend(result.err());
    outcome
}

/// Everything an accepted step feeds.
struct Record<'c> {
    columns: &'c Columns,
    trace: Trace,
    transient: Transient,
    rows: Rows,
    /// The last accepted step, when it is not a row. A run that fails stores it as its last
    /// row.
    last_step: Option<(Time, Vec<Finite>)>,
    ratings: Ratings,
    op: Option<OpPoint>,
    /// `None` without a chip.
    logic: Option<Logic>,
}

impl Record<'_> {
    /// Ratings sample every accepted step. A non-row step folds into the open interval, and
    /// a row closes it. A row past the trace's limit is `trace.too_large`, and the step is
    /// neither stored nor folded.
    fn accept(&mut self, at: Time, row: Vec<Finite>) -> Result<(), Diagnostic> {
        let kept = self.rows.keeps(&self.transient, at);
        if kept {
            let columns = self.columns.refs().len();
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
        self.ratings.sample(at, &row);
        if at == Time::ZERO {
            self.op = Some(observe::op_point(self.columns, &row));
        }
        if kept {
            self.trace.push(at, &row);
            self.last_step = None;
        } else {
            self.trace.sample(&row);
            self.last_step = Some((at, row));
        }
        Ok(())
    }

    /// The one constructor of a transient's observations, after success and after an error.
    /// `Nothing` when t = 0 never solved. Otherwise the last accepted step is the last row,
    /// `end` is its time, the switches are the edges before it, and the logic summary comes
    /// from the `Logic` the trace keeps.
    fn finish(self, netlist: &Netlist, diagnostics: &mut Vec<Diagnostic>) -> Outcome {
        diagnostics.extend(self.ratings.finish());
        let (mut trace, Some(op)) = (self.trace, self.op) else {
            return Outcome {
                observations: Observations::Nothing,
                trace: None,
            };
        };
        if let Some((at, row)) = self.last_step {
            trace.push(at, &row);
        }
        let end = trace.end().expect("the operating point is row 0");
        let summary = self.logic.as_ref().map(Logic::summary);
        if let Some(logic) = self.logic {
            trace.attach_logic(logic);
        }
        Outcome {
            observations: Observations::Transient {
                end,
                op,
                switches: switches(netlist, end),
                logic: summary,
            },
            trace: Some(trace),
        }
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
    segment: Segment,
    /// Each chip's state, by slot.
    chips: Vec<chip::State>,
    /// Each watch's level at `at`.
    levels: Levels,
}

fn steps(
    netlist: &Netlist,
    transient: &Transient,
    actives: Vec<active::Chip>,
    budget: Budget,
    record: &mut Record<'_>,
) -> Result<(), Diagnostic> {
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
        record.columns,
        &start.iterate.solution,
        Time::ZERO,
    )?;
    record.accept(Time::ZERO, row)?;
    record.logic(Time::ZERO, &circuit, &start.chips, &start.levels);
    let mut actives = Actives::new(&circuit, &start.levels);
    let mut breakpoints = Breakpoints::new(netlist, *transient);
    let segment = Segment::start(&charges, &start.iterate.solution);
    let mut accepted = Accepted {
        at: Time::ZERO,
        iterate: start.iterate,
        segment,
        chips: start.chips,
        levels: start.levels,
    };
    let mut plan = Plan::RESTART;
    // Newton iterations over every stalled attempt since the last accepted step. An attempt
    // rejected for its error converged, and does not count.
    let mut iterations = 0;
    let mut accepted_steps = 0u64;
    while accepted.at < transient.stop {
        if accepted_steps >= budget.steps {
            return Err(Diagnostic::SolverStepLimit {
                at: accepted.at,
                limit: budget.steps,
            });
        }
        let h = plan.clip(accepted.at, breakpoints.next_after(accepted.at));
        let at = actives.horizon(Time::from_ps(accepted.at.as_ps() + h));
        let h = at.as_ps() - accepted.at.as_ps();
        match attempt(&circuit, &accepted, actives.cores(), h, budget.step) {
            Ok(converged) => {
                let error = lte::error(
                    &charges,
                    &accepted.segment,
                    accepted.at,
                    &accepted.iterate.solution,
                    h,
                    &converged.iterate.solution,
                );
                let (ratio, method) = (error.ratio, error.method);
                match error.verdict(h) {
                    Verdict::Accept => {}
                    Verdict::Reject => {
                        plan = Plan::rejected(h, ratio, method);
                        continue;
                    }
                    Verdict::Stiffen(more) => {
                        accepted = Accepted {
                            segment: accepted.segment.stiffen(&more, h),
                            ..accepted
                        };
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
                        |h| attempt(&circuit, &accepted, actives.cores(), h, budget.probe),
                    )
                    .map_err(|(probe, failure)| {
                        probe_failure(&circuit, accepted.at, probe, failure)
                    })?;
                    let levels = circuit
                        .watches
                        .levels(&converged.iterate.solution, Some(&accepted.levels));
                    (located, converged, levels)
                };
                // The row holds the chip states it was solved with.
                let row =
                    observe::observe(&circuit, record.columns, &converged.iterate.solution, at)?;
                record.accept(at, row)?;
                accepted_steps += 1;
                iterations = 0;
                // A watch cuts the step. Only a changed stamp, a chip's, an active chip's image, a
                // contact's or a source's, restarts.
                let chips = circuit.react(&accepted.chips, &accepted.levels, &levels);
                let driven = actives.land(
                    &circuit,
                    at,
                    &converged.iterate.solution,
                    &accepted.levels,
                    &levels,
                );
                let restart = (breakpoints.is_edge(at) || chips != accepted.chips || driven)
                    && at < transient.stop;
                // A step cut short by a breakpoint or a watch keeps the step it wanted.
                plan = match (restart, h == plan.h && at == step_end) {
                    (true, _) => Plan::RESTART,
                    (false, true) => Plan::accepted(h, ratio, method, transient.max_step),
                    (false, false) => plan,
                };
                let next = if restart {
                    let held = restart_solve(
                        &circuit,
                        at,
                        converged.iterate,
                        chips,
                        actives.cores(),
                        &levels,
                        budget.restart,
                    )?;
                    actives.input(&circuit, at, &levels, &held.levels);
                    let segment = Segment::start(&charges, &held.iterate.solution);
                    Accepted {
                        at,
                        iterate: held.iterate,
                        segment,
                        chips: held.chips,
                        levels: held.levels,
                    }
                } else {
                    let landed = at.as_ps() - accepted.at.as_ps();
                    let swings = if cut {
                        accepted.segment.swings(
                            &charges,
                            &accepted.iterate.solution,
                            landed,
                            &converged.iterate.solution,
                        )
                    } else {
                        error.swings
                    };
                    Accepted {
                        at,
                        segment: accepted.segment.advance(
                            accepted.at,
                            accepted.iterate.solution,
                            landed,
                            swings,
                        ),
                        iterate: converged.iterate,
                        chips,
                        levels: levels.clone(),
                    }
                };
                // A flip the chips reacted to is recorded at `at`, even when the restart's
                // solve is back across the threshold. The restart's level is the next step's
                // prior, so the return is recorded on the next row.
                let recorded = next.levels.with_flips(&accepted.levels, &levels);
                record.logic(at, &circuit, &next.chips, &recorded);
                accepted = next;
            }
            Err(Failure::Singular { row }) => {
                return Err(Diagnostic::SolverSingular {
                    at,
                    unknown: circuit.layout.describe(netlist, row),
                });
            }
            Err(Failure::Stalled(stall)) => {
                // Without a junction the system is linear, and a shorter step does not make it finite.
                if let Offender::NonFinite(_) = stall.offender
                    && circuit.layout.junctions() == 0
                {
                    return Err(Diagnostic::SolverNonFinite {
                        at,
                        unknown: stall.offender.describe(&circuit),
                    });
                }
                iterations += stall.iterations;
                plan = Plan::halve(h).ok_or_else(|| Diagnostic::SolverConvergence {
                    at: accepted.at,
                    unknown: stall.offender.describe(&circuit),
                    iterations,
                    gmin: None,
                    source_scale: 1.0,
                    solve: Solve::Step { step_ps: h },
                })?;
            }
        }
    }
    Ok(())
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

/// One step of `h` ps from `from`. `from` is only read, so a retry calls this again.
fn attempt(
    circuit: &Circuit,
    from: &Accepted,
    cores: &[active::Core],
    h: u64,
    budget: u32,
) -> Result<Converged, Failure> {
    let conditions = Conditions {
        at: Time::from_ps(from.at.as_ps() + h),
        history: History::Step(Integration {
            method: from.segment.method(),
            stiff: from.segment.stiff(),
            h_ps: h,
            previous: &from.iterate.solution,
        }),
        chips: &from.chips,
        cores,
        ..Conditions::NOMINAL
    };
    newton(circuit, &conditions, from.iterate.clone(), budget)
}

/// The solve after a switch edge, a source jump or a chip drive change at `at`: every capacitor
/// held at its voltage in `step`, the new contacts, sources and `chips` in force, settled on the operating point
/// ladder with `budget` per rung. `levels` were read on `step`. It is history for the next
/// step, not a row.
fn restart_solve(
    circuit: &Circuit,
    at: Time,
    step: Iterate,
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

/// Where a step must land: every row time, every contact edge and source jump, and 1 ns after
/// t = 0 and after each edge. The deck ramps each contact and source over that nanosecond. A
/// lazy merge of one cursor per part, so no edge list is built and none is capped. Time only
/// moves forward.
struct Breakpoints<'n> {
    transient: Transient,
    /// Each cursor's next edge before `stop`, earliest first, with the cursor's index.
    edges: BinaryHeap<Reverse<(Time, usize)>>,
    cursors: Vec<Box<dyn Iterator<Item = Time> + 'n>>,
    /// 1 ns after t = 0 and after each edge passed, before `stop`, ascending.
    ramp_ends: VecDeque<Time>,
}

impl<'n> Breakpoints<'n> {
    fn new(netlist: &'n Netlist, transient: Transient) -> Breakpoints<'n> {
        let stop = transient.stop;
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
            transient,
            edges,
            cursors,
            ramp_ends: VecDeque::new(),
        };
        breakpoints.ramp_after(Time::ZERO);
        breakpoints
    }

    /// The earliest landing after `now`, which is before `stop`.
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
        let row = self.transient.next_row_after(now);
        let edge = self.edges.peek().map_or(row, |&Reverse((at, _))| at);
        let ramp_end = self.ramp_ends.front().copied().unwrap_or(row);
        row.min(edge).min(ramp_end)
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
        if end < self.transient.stop.as_ps()
            && self.ramp_ends.back().is_none_or(|back| back.as_ps() < end)
        {
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
