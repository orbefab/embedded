use std::collections::BTreeMap;

use sim_core::diagnostic::{Diagnostic, Solve};
use sim_core::finite::Finite;
use sim_core::logic::Logic;
use sim_core::name::{Name, Ref};
use sim_core::netlist::{Edge, Model, Netlist, Transient};
use sim_core::run_folder::{Observations, OpPoint};
use sim_core::time::Time;
use sim_core::trace::Trace;

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

/// Newton's budget for each step, each probe locating a chip event, and each rung of a
/// restart's ladder. A test lowers one to make that solve fail.
#[derive(Clone, Copy)]
struct Budget {
    step: u32,
    probe: u32,
    restart: u32,
}

const BUDGET: Budget = Budget {
    step: STEP_BUDGET,
    probe: STEP_BUDGET,
    restart: op::OP_BUDGET,
};

/// After an error `observations` is empty, and the trace keeps the rows before it.
pub(crate) struct Outcome {
    pub(crate) observations: Observations,
    pub(crate) trace: Option<Trace>,
}

/// The operating point, then steps to `stop`. Rating findings and any solver
/// error go to `diagnostics`.
pub(crate) fn run(
    netlist: &Netlist,
    transient: &Transient,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    let columns = Columns::new(netlist);
    run_with(
        netlist,
        transient,
        BUDGET,
        Trace::row_limit(columns.refs().len()),
        &columns,
        diagnostics,
    )
}

fn run_with(
    netlist: &Netlist,
    transient: &Transient,
    budget: Budget,
    max_rows: usize,
    columns: &Columns,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    let mut record = Record {
        trace: Trace::new(columns.refs().to_vec(), transient.stop),
        max_rows,
        ratings: Ratings::new(netlist, columns, Timing::Run),
        columns,
        op: None,
        logic: None,
    };
    let result = steps(netlist, transient, budget, &mut record);
    diagnostics.extend(record.ratings.finish());
    let mut trace = record.trace;
    if let Some(logic) = &record.logic {
        trace.attach_logic(logic.clone());
    }
    match result {
        Ok(()) => Outcome {
            observations: Observations {
                op: record.op,
                switches: Some(switches(netlist, transient.stop)),
                logic: record.logic,
            },
            trace: Some(trace),
        },
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            Outcome {
                observations: Observations::default(),
                trace: (trace.rows() > 0).then_some(trace),
            }
        }
    }
}

/// Everything an accepted row feeds.
struct Record<'c> {
    columns: &'c Columns,
    trace: Trace,
    max_rows: usize,
    ratings: Ratings,
    op: Option<OpPoint>,
    /// `None` without a chip.
    logic: Option<Logic>,
}

impl Record<'_> {
    fn accept(&mut self, at: Time, row: &[Finite]) -> Result<(), Diagnostic> {
        if self.trace.rows() == self.max_rows {
            return Err(Diagnostic::TraceTooLarge {
                at,
                limit: self.max_rows,
                columns: self.columns.refs().len(),
            });
        }
        self.trace.push(at, row);
        if at == Time::ZERO {
            self.op = Some(observe::op_point(self.columns, row));
        }
        self.ratings.sample(at, row);
        Ok(())
    }

    /// Every chip signal's level from `at` on: one `Ref::Logic` per signal, chips in slot order.
    fn logic(&mut self, at: Time, circuit: &Circuit, chips: &[chip::State], levels: &Levels) {
        let mut now = Vec::new();
        for (slot, ((_, chip), state)) in circuit.chips().zip(chips).enumerate() {
            now.extend(chip::levels(
                chip,
                state,
                levels.of(circuit.watches.of(slot)),
            ));
        }
        match &mut self.logic {
            Some(logic) => {
                for (signal, level) in now.into_iter().enumerate() {
                    logic.set(at, signal, level);
                }
            }
            None if now.is_empty() => {}
            None => self.logic = Some(Logic::new(signals(circuit), now)),
        }
    }
}

fn signals(circuit: &Circuit) -> Vec<Ref> {
    circuit
        .chips()
        .flat_map(|(id, chip)| {
            let part = &circuit.netlist.part(id).name;
            chip::signals(chip)
                .iter()
                .map(|signal| Ref::Logic(part.clone(), Name::new(signal).expect("signal name")))
        })
        .collect()
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
    budget: Budget,
    record: &mut Record<'_>,
) -> Result<(), Diagnostic> {
    let circuit = Circuit::new(netlist)?;
    let charges = Charges::new(netlist, &circuit.layout);
    let start = Iterate::initial(&circuit);
    let start = op::settle(
        &circuit,
        Conditions::NOMINAL,
        start,
        circuit.power_on(),
        None,
        op::OP_BUDGET,
    )?;
    let row = observe::observe(
        &circuit,
        record.columns,
        &start.iterate.solution,
        Time::ZERO,
    )?;
    record.accept(Time::ZERO, &row)?;
    record.logic(Time::ZERO, &circuit, &start.chips, &start.levels);
    let mut breakpoints = Breakpoints::new(netlist, transient.stop);
    let mut accepted = Accepted {
        at: Time::ZERO,
        iterate: start.iterate,
        segment: Segment::Start,
        chips: start.chips,
        levels: start.levels,
    };
    let mut plan = Plan::RESTART;
    // Newton iterations over every stalled attempt since the last accepted step. An attempt
    // rejected for its error converged, and does not count.
    let mut iterations = 0;
    while accepted.at < transient.stop {
        let h = plan.clip(accepted.at, breakpoints.next_after(accepted.at));
        let at = Time::from_ps(accepted.at.as_ps() + h);
        match attempt(&circuit, &accepted, h, budget.step) {
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
                let (at, converged, levels) = if levels == accepted.levels {
                    (at, converged, levels)
                } else {
                    let (located, converged) = watch::locate(
                        &circuit.watches,
                        &accepted.levels,
                        (accepted.at, &accepted.iterate.solution),
                        (at, converged),
                        |h| attempt(&circuit, &accepted, h, budget.probe),
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
                record.accept(at, &row)?;
                iterations = 0;
                // A watch cuts the step. Only a changed stamp, a chip's or a contact's, restarts.
                let chips = circuit.react(&accepted.chips, &accepted.levels, &levels);
                let restart =
                    (breakpoints.is_edge(at) || chips != accepted.chips) && at < transient.stop;
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
                        &levels,
                        budget.restart,
                    )?;
                    Accepted {
                        at,
                        iterate: held.iterate,
                        segment: Segment::Start,
                        chips: held.chips,
                        levels: held.levels,
                    }
                } else {
                    Accepted {
                        at,
                        segment: accepted.segment.advance(
                            accepted.at,
                            accepted.iterate.solution,
                            at.as_ps() - accepted.at.as_ps(),
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
fn attempt(circuit: &Circuit, from: &Accepted, h: u64, budget: u32) -> Result<Converged, Failure> {
    let conditions = Conditions {
        at: Time::from_ps(from.at.as_ps() + h),
        history: History::Step(Integration {
            method: from.segment.method(),
            stiff: from.segment.stiff(),
            h_ps: h,
            previous: &from.iterate.solution,
        }),
        chips: &from.chips,
        ..Conditions::NOMINAL
    };
    newton(circuit, &conditions, from.iterate.clone(), budget)
}

/// The solve after a switch edge or a chip drive change at `at`: every capacitor held at its
/// voltage in `step`, the new contacts and `chips` in force, settled on the operating point
/// ladder with `budget` per rung. `levels` were read on `step`. It is history for the next
/// step, not a row.
fn restart_solve(
    circuit: &Circuit,
    at: Time,
    step: Iterate,
    chips: Vec<chip::State>,
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
        Some(levels),
        budget,
    )
}

/// Where a step must land. Contact edges, 1 ns after t = 0, and 1 ns after every
/// edge, then `stop`. The deck ramps each contact over that nanosecond. The cursor
/// only moves forward.
struct Breakpoints {
    /// Contact edges strictly before `stop`, ascending.
    edges: Vec<Time>,
    /// Landing times strictly before `stop`, ascending.
    points: Vec<Time>,
    stop: Time,
    cursor: usize,
}

impl Breakpoints {
    fn new(netlist: &Netlist, stop: Time) -> Breakpoints {
        let mut edges: Vec<Time> = netlist
            .parts()
            .filter_map(|(_, part)| match &part.model {
                Model::Switch(switch) => Some(switch.schedule.edges()),
                _ => None,
            })
            .flatten()
            .map(|edge| edge.at)
            .filter(|&at| at < stop)
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let mut points = Vec::with_capacity(edges.len() * 2 + 1);
        push_point(&mut points, EDGE_STEP, stop);
        for &edge in &edges {
            push_point(&mut points, edge.as_ps(), stop);
            if let Some(end) = edge.as_ps().checked_add(EDGE_STEP) {
                push_point(&mut points, end, stop);
            }
        }
        points.sort_unstable();
        points.dedup();
        Breakpoints {
            edges,
            points,
            stop,
            cursor: 0,
        }
    }

    fn next_after(&mut self, now: Time) -> Time {
        while self
            .points
            .get(self.cursor)
            .is_some_and(|&point| point <= now)
        {
            self.cursor += 1;
        }
        self.points.get(self.cursor).copied().unwrap_or(self.stop)
    }

    fn is_edge(&self, at: Time) -> bool {
        self.edges.binary_search(&at).is_ok()
    }
}

/// A landing time strictly before `stop`. `stop` itself is the cursor's end.
fn push_point(points: &mut Vec<Time>, at: u64, stop: Time) {
    if at > 0 && at < stop.as_ps() {
        points.push(Time::from_ps(at));
    }
}

/// Every contact edge before `stop`, per switch.
fn switches(netlist: &Netlist, stop: Time) -> BTreeMap<Ref, Vec<Edge>> {
    netlist
        .parts()
        .filter_map(|(_, part)| match &part.model {
            Model::Switch(switch) => Some((
                Ref::Part(part.name.clone()),
                switch
                    .schedule
                    .edges()
                    .iter()
                    .filter(|edge| edge.at < stop)
                    .copied()
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
