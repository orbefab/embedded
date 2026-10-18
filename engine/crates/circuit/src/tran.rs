use std::collections::BTreeMap;

use sim_core::diagnostic::Diagnostic;
use sim_core::finite::Finite;
use sim_core::name::Ref;
use sim_core::netlist::{Edge, Model, Netlist, Transient};
use sim_core::run_folder::{Observations, OpPoint};
use sim_core::time::Time;
use sim_core::trace::Trace;

use crate::lte::{self, Charges, Segment, Verdict};
use crate::newton::{
    Circuit, Conditions, Converged, Failure, History, Integration, Iterate, Offender, newton,
};
use crate::observe::{self, Columns};
use crate::op;
use crate::ratings::{Ratings, Timing};
use crate::step::{EDGE_STEP, Plan};

pub(crate) const STEP_BUDGET: u32 = 50;

/// Newton's budget for each step, and for each rung of a restart's ladder.
/// A test lowers one to make that solve fail.
#[derive(Clone, Copy)]
struct Budget {
    step: u32,
    restart: u32,
}

const BUDGET: Budget = Budget {
    step: STEP_BUDGET,
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
    };
    match steps(netlist, transient, budget, &mut record) {
        Ok(()) => {
            diagnostics.extend(record.ratings.finish());
            Outcome {
                observations: Observations {
                    op: record.op,
                    switches: Some(switches(netlist, transient.stop)),
                },
                trace: Some(record.trace),
            }
        }
        Err(diagnostic) => {
            diagnostics.extend(record.ratings.finish());
            diagnostics.push(diagnostic);
            Outcome {
                observations: Observations::default(),
                trace: (record.trace.rows() > 0).then_some(record.trace),
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
}

/// The whole state a step starts from. Replaced, never mutated, on accept.
struct Accepted {
    at: Time,
    /// At a switch edge, the restart solve at `at`, not the row at `at`.
    iterate: Iterate,
    segment: Segment,
}

fn steps(
    netlist: &Netlist,
    transient: &Transient,
    budget: Budget,
    record: &mut Record<'_>,
) -> Result<(), Diagnostic> {
    let circuit = Circuit::new(netlist)?;
    let charges = Charges::new(netlist, &circuit.layout);
    let start = op::operating_point(&circuit, Conditions::NOMINAL, Iterate::initial(&circuit))?;
    let row = observe::observe(&circuit, record.columns, &start.solution, Time::ZERO)?;
    record.accept(Time::ZERO, &row)?;
    let mut breakpoints = Breakpoints::new(netlist, transient.stop);
    let mut accepted = Accepted {
        at: Time::ZERO,
        iterate: start,
        segment: Segment::Start,
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
                let row =
                    observe::observe(&circuit, record.columns, &converged.iterate.solution, at)?;
                record.accept(at, &row)?;
                iterations = 0;
                let edge = breakpoints.is_edge(at);
                // A step cut short by a breakpoint keeps the step it wanted.
                plan = match (edge, h == plan.h) {
                    (true, _) => Plan::RESTART,
                    (false, true) => Plan::accepted(h, ratio, method, transient.max_step),
                    (false, false) => plan,
                };
                accepted = if edge {
                    Accepted {
                        at,
                        iterate: restart(&circuit, at, converged.iterate, budget.restart)?,
                        segment: Segment::Start,
                    }
                } else {
                    Accepted {
                        at,
                        segment: accepted.segment.advance(
                            accepted.at,
                            accepted.iterate.solution,
                            h,
                        ),
                        iterate: converged.iterate,
                    }
                };
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
                    step_ps: Some(h),
                })?;
            }
        }
    }
    Ok(())
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
        ..Conditions::NOMINAL
    };
    newton(circuit, &conditions, from.iterate.clone(), budget)
}

/// The solve after a switch edge at `at`: every capacitor held at its voltage in `step`, the
/// new contacts in force, on the operating point ladder with `budget` per rung. It is history
/// for the next step, not a row.
fn restart(circuit: &Circuit, at: Time, step: Iterate, budget: u32) -> Result<Iterate, Diagnostic> {
    let conditions = Conditions {
        at,
        history: History::Restart {
            previous: &step.solution,
        },
        ..Conditions::NOMINAL
    };
    op::climb(circuit, conditions, step.clone(), |_| budget)
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
