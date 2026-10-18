use sim_core::diagnostic::Diagnostic;
use sim_core::name::Ref;
use sim_core::netlist::{Chip, Model, Netlist, PartId, Waveform};
use sim_core::time::Time;

use crate::chip;
use crate::element;
use crate::mna::{GMIN, Layout, Probe, Solution, Stamp, System};
use crate::watch::{Levels, Watches};

pub(crate) const RELTOL: f64 = 1e-6;
pub(crate) const VNTOL: f64 = 1e-9;
pub(crate) const ABSTOL: f64 = 1e-12;

/// A netlist with its rows assigned and its chips' watches declared. Read-only for the
/// whole analysis.
pub(crate) struct Circuit<'n> {
    pub(crate) netlist: &'n Netlist,
    pub(crate) layout: Layout,
    pub(crate) watches: Watches,
}

impl<'n> Circuit<'n> {
    /// Fails with `solver.too_large` before the matrix is allocated.
    pub(crate) fn new(netlist: &'n Netlist) -> Result<Circuit<'n>, Diagnostic> {
        let layout = Layout::new(netlist);
        if layout.size() > crate::dense::MAX_UNKNOWNS {
            return Err(Diagnostic::SolverTooLarge {
                at: Time::ZERO,
                unknowns: layout.size(),
                limit: crate::dense::MAX_UNKNOWNS,
            });
        }
        let mut circuit = Circuit {
            netlist,
            layout,
            watches: Watches::new(Vec::new()),
        };
        let watches = circuit
            .chips()
            .map(|(id, chip)| chip::watches(chip, id, &circuit.layout))
            .collect();
        circuit.watches = Watches::new(watches);
        Ok(circuit)
    }

    /// The chips in slot order.
    pub(crate) fn chips(&self) -> impl Iterator<Item = (PartId, &'n Chip)> + '_ {
        let netlist = self.netlist;
        self.layout
            .chips()
            .iter()
            .map(move |&id| match &netlist.part(id).model {
                Model::Chip(chip) => (id, chip),
                _ => unreachable!("the layout lists only chips"),
            })
    }

    /// Every chip's state at power-on, in slot order.
    pub(crate) fn power_on(&self) -> Vec<chip::State> {
        self.chips().map(|(_, chip)| chip::power_on(chip)).collect()
    }

    /// Every chip's next state, from the levels just before and just after one time.
    pub(crate) fn react(
        &self,
        chips: &[chip::State],
        before: &Levels,
        after: &Levels,
    ) -> Vec<chip::State> {
        self.chips()
            .zip(chips)
            .enumerate()
            .map(|(slot, ((_, chip), state))| {
                let watched = self.watches.of(slot);
                chip::react(chip, state, before.of(watched.clone()), after.of(watched))
            })
            .collect()
    }
}

/// What varies between Newton calls on one circuit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Conditions<'h> {
    /// Siemens from every node row to ground.
    pub(crate) gmin: f64,
    /// Multiplies every independent source.
    pub(crate) source_scale: f64,
    /// The time this solve is for. A switch takes its contact, and a source its value, on the
    /// interval ending here, or on the one starting here under `History::Restart`.
    pub(crate) at: Time,
    pub(crate) history: History<'h>,
    /// Each chip's state, by slot. Only an accepted time changes it.
    pub(crate) chips: &'h [chip::State],
}

impl Conditions<'_> {
    /// Whether a timed stamp takes the interval that starts at `at`. Only a restart does.
    pub(crate) fn after_change(&self) -> bool {
        matches!(self.history, History::Restart { .. })
    }

    /// A source's netlist value in this solve, before `source_scale`.
    pub(crate) fn level(&self, waveform: &Waveform) -> f64 {
        if self.after_change() {
            waveform.after(self.at)
        } else {
            waveform.before(self.at)
        }
    }
}

impl Conditions<'static> {
    pub(crate) const NOMINAL: Conditions<'static> = Conditions {
        gmin: GMIN,
        source_scale: 1.0,
        at: Time::ZERO,
        history: History::Start,
        chips: &[],
    };
}

/// How capacitors and contacts see the past.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum History<'h> {
    /// t = 0. A capacitor with `initial` is held there, and any other is open.
    Start,
    /// Just after a switch edge, a source jump or a chip drive change at `at`. Every capacitor is held at its voltage in `previous`,
    /// and every contact and source is after the changes at or before `at`. The solve is algebraic. Its
    /// capacitor currents are the post-edge currents the next step starts from.
    Restart { previous: &'h Solution },
    /// A time step from the last accepted solution.
    Step(Integration<'h>),
}

/// One time step's view of the past. The last accepted solution is the whole history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Integration<'h> {
    pub(crate) method: Method,
    /// Capacitors that take backward Euler whatever `method` is. Ascending.
    pub(crate) stiff: &'h [PartId],
    pub(crate) h_ps: u64,
    pub(crate) previous: &'h Solution,
}

impl Integration<'_> {
    /// The step in seconds.
    pub(crate) fn h(&self) -> f64 {
        self.h_ps as f64 / 1e12
    }

    pub(crate) fn method_of(&self, part: PartId) -> Method {
        if self.stiff.binary_search(&part).is_ok() {
            Method::BackwardEuler
        } else {
            self.method
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
    BackwardEuler,
    Trapezoidal,
}

/// All of Newton's state. A value: passed in, returned, or dropped.
#[derive(Clone, Debug)]
pub(crate) struct Iterate {
    pub(crate) solution: Solution,
    /// The limited voltage per junction slot, which the next step limits against.
    pub(crate) junctions: Vec<f64>,
}

impl Iterate {
    /// Every unknown at 0 and every junction at its element's start voltage.
    pub(crate) fn initial(circuit: &Circuit) -> Iterate {
        let layout = &circuit.layout;
        let mut junctions = vec![0.0; layout.junctions()];
        for (id, part) in circuit.netlist.parts() {
            element::initial(&part.model, &mut junctions[layout.junction_slots(id)]);
        }
        Iterate {
            solution: Solution::zeros(layout.size()),
            junctions,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Converged {
    pub(crate) iterate: Iterate,
    pub(crate) iterations: u32,
}

#[derive(Debug)]
pub(crate) enum Failure {
    /// Zero pivot. No change of conditions fixes it.
    Singular {
        row: usize,
    },
    Stalled(Stall),
}

#[derive(Debug)]
pub(crate) struct Stall {
    pub(crate) iterations: u32,
    pub(crate) offender: Offender,
}

/// What failed worst in the last iteration of a stalled call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Offender {
    /// Limited, or its current check failed.
    Junction(PartId),
    /// Moved more than its tolerance.
    Row(usize),
    /// Not finite after the solve.
    NonFinite(usize),
}

impl Offender {
    pub(crate) fn describe(self, circuit: &Circuit) -> Ref {
        match self {
            Offender::Junction(part) => Ref::Part(circuit.netlist.part(part).name.clone()),
            Offender::Row(row) | Offender::NonFinite(row) => {
                circuit.layout.describe(circuit.netlist, row)
            }
        }
    }
}

/// Newton with junction limiting, at most `budget` solves.
///
/// Converged means three checks pass in one iteration: no unknown moved more
/// than its tolerance, every junction's current check passed, and no junction
/// was limited.
pub(crate) fn newton(
    circuit: &Circuit,
    conditions: &Conditions,
    start: Iterate,
    budget: u32,
) -> Result<Converged, Failure> {
    let layout = &circuit.layout;
    let mut iterate = start;
    let mut offender = None;
    let mut system = System::new(layout);
    for iteration in 1..=budget {
        system.reset(layout, conditions.gmin);
        for (id, part) in circuit.netlist.parts() {
            element::stamp(
                &part.model,
                &mut Stamp {
                    sys: &mut system,
                    layout,
                    part: id,
                    iterate: &iterate,
                    conditions,
                },
            );
        }
        let solution = system
            .solve()
            .map_err(|singular| Failure::Singular { row: singular.row })?;
        if let Some(row) = solution.components().iter().position(|x| !x.is_finite()) {
            return Err(Failure::Stalled(Stall {
                iterations: iteration,
                offender: Offender::NonFinite(row),
            }));
        }

        let mut junctions = vec![0.0; layout.junctions()];
        let mut worst_junction: Option<(f64, PartId)> = None;
        for (id, part) in circuit.netlist.parts() {
            let slots = layout.junction_slots(id);
            let probe = Probe {
                layout,
                part: id,
                x: &solution,
            };
            let update = element::update(
                &part.model,
                &probe,
                &iterate.junctions[slots.clone()],
                &mut junctions[slots],
            );
            let score = if update.limited {
                f64::INFINITY
            } else {
                update.mismatch
            };
            if score > 1.0 && worst_junction.is_none_or(|(worst, _)| score > worst) {
                worst_junction = Some((score, id));
            }
        }
        let (worst_row, row_ratio) = worst_row(layout, iterate.solution.components(), &solution);

        iterate = Iterate {
            solution,
            junctions,
        };
        offender = Some(match worst_junction {
            Some((_, part)) => Offender::Junction(part),
            None if row_ratio <= 1.0 => {
                return Ok(Converged {
                    iterate,
                    iterations: iteration,
                });
            }
            None => Offender::Row(worst_row),
        });
    }
    Err(Failure::Stalled(Stall {
        iterations: budget,
        offender: offender.expect("budget is at least one iteration"),
    }))
}

/// The row whose step is largest against its tolerance, and that ratio.
fn worst_row(layout: &Layout, old: &[f64], new: &Solution) -> (usize, f64) {
    let mut worst = (0, 0.0);
    for (row, (&before, &after)) in old.iter().zip(new.components()).enumerate() {
        let floor = if layout.is_node_row(row) {
            VNTOL
        } else {
            ABSTOL
        };
        let tolerance = RELTOL * before.abs().max(after.abs()) + floor;
        let ratio = (after - before).abs() / tolerance;
        if ratio > worst.1 {
            worst = (row, ratio);
        }
    }
    worst
}
