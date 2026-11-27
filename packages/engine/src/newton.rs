use orbe_model::Time;

use crate::netlist::{Contact, Model, Netlist, PartId, Schedule, Waveform};

use crate::diagnostic::Diagnostic;
use crate::logic::Level;
use crate::name::{Name, Ref};

use crate::active;
use crate::chip;
use crate::element;
use crate::mna::{GMIN, Layout, Probe, Solution, Solver, Stamp, System};
use crate::watch::{Levels, Watches};

pub(crate) const RELTOL: f64 = 1e-6;
pub(crate) const VNTOL: f64 = 1e-9;
pub(crate) const ABSTOL: f64 = 1e-12;

/// A netlist with its rows assigned and its chips' watches declared. Its schedules only grow,
/// and only past the furthest time a step attempt has reached.
pub(crate) struct Circuit {
    pub(crate) netlist: Netlist,
    pub(crate) layout: Layout,
    /// The netlist's chips' watches by slot, then each active chip's.
    pub(crate) watches: Watches,
    /// By active slot.
    pub(crate) actives: Vec<active::Chip>,
    /// Every chip a netlist part builds, sorted by part, which is the order of the logic refs.
    logic: Vec<(PartId, Slot)>,
    /// No part's stamp reads the iterate, and no active chip draws through a [`Draw`], so one
    /// Newton call stamps one system.
    ///
    /// [`Draw`]: orbe_model::Draw
    pub(crate) linear: bool,
}

#[derive(Clone, Copy, Debug)]
enum Slot {
    Passive(usize),
    Active(usize),
}

impl Circuit {
    /// Fails with `solver.too_large` before any system is allocated.
    pub(crate) fn new(netlist: Netlist) -> Result<Circuit, Diagnostic> {
        Circuit::with_actives(netlist, Vec::new())
    }

    /// The netlist's active chips, then `extra`, then the network when the netlist has one. Only
    /// a test passes `extra`, a script chip with no netlist kind.
    pub(crate) fn with_actives(
        netlist: Netlist,
        extra: Vec<active::Chip>,
    ) -> Result<Circuit, Diagnostic> {
        let mut passives = Vec::new();
        let mut actives = Vec::new();
        for (id, part) in netlist.parts() {
            if chip::passive(&part.model).is_some() {
                passives.push(id);
            } else if let Model::Chip(chip) = &part.model
                && let chip::Kind::Active(kind) = chip::kind(chip)
            {
                actives.push(active::chip(id, kind));
            }
        }
        actives.extend(extra);
        actives.extend(netlist.network().map(crate::air::chip));
        let layout = Layout::new(&netlist, passives);
        if layout.size() > crate::sparse::MAX_UNKNOWNS {
            return Err(Diagnostic::SolverTooLarge {
                at: Time::ZERO,
                unknowns: layout.size(),
                limit: crate::sparse::MAX_UNKNOWNS,
            });
        }
        let mut logic: Vec<(PartId, Slot)> = layout
            .chips()
            .iter()
            .enumerate()
            .map(|(slot, &id)| (id, Slot::Passive(slot)))
            .chain(
                actives
                    .iter()
                    .enumerate()
                    .filter_map(|(k, chip)| Some((chip.part?, Slot::Active(k)))),
            )
            .collect();
        logic.sort_unstable_by_key(|&(id, _)| id);
        let linear = netlist.parts().all(|(_, part)| !part.model.reads_iterate())
            && actives.iter().all(|chip| chip.draw.is_none());
        let mut circuit = Circuit {
            netlist,
            layout,
            watches: Watches::new(Vec::new()),
            actives,
            logic,
            linear,
        };
        let watches = circuit
            .chips()
            .map(|(id, chip)| chip::watches(chip, id, &circuit.layout))
            .chain(circuit.actives.iter().map(|chip| chip.watches.clone()))
            .collect();
        circuit.watches = Watches::new(watches);
        Ok(circuit)
    }

    pub(crate) fn active_watches(&self, k: usize) -> std::ops::Range<usize> {
        self.watches.of(self.layout.chips().len() + k)
    }

    pub(crate) fn logic_refs(&self) -> Vec<Ref> {
        let mut refs = Vec::new();
        for &(id, slot) in &self.logic {
            let part = &self.netlist.part(id).name;
            let signal =
                |name: &str| Ref::Logic(part.clone(), Name::new(name).expect("signal name"));
            match slot {
                Slot::Passive(slot) => {
                    let id = self.layout.chips()[slot];
                    refs.extend(
                        chip::signals(self.passive(id))
                            .iter()
                            .map(|name| signal(name)),
                    );
                }
                Slot::Active(k) => {
                    refs.extend(
                        self.actives[k]
                            .signals
                            .iter()
                            .map(|&(name, _)| signal(name)),
                    );
                }
            }
        }
        refs
    }

    pub(crate) fn logic_levels(&self, chips: &[chip::State], levels: &Levels) -> Vec<Level> {
        let mut now = Vec::new();
        for &(_, slot) in &self.logic {
            match slot {
                Slot::Passive(slot) => {
                    let id = self.layout.chips()[slot];
                    let chip = self.passive(id);
                    let watched = levels.of(self.watches.of(slot));
                    now.extend(chip::levels(chip, &chips[slot], watched));
                }
                Slot::Active(k) => {
                    let watched = levels.of(self.active_watches(k));
                    now.extend(
                        self.actives[k]
                            .signals
                            .iter()
                            .map(|&(_, watch)| Level::from_bool(watched[watch])),
                    );
                }
            }
        }
        now
    }

    /// The passive chips in slot order.
    pub(crate) fn chips(&self) -> impl Iterator<Item = (PartId, chip::Passive<'_>)> + '_ {
        self.layout.chips().iter().map(|&id| (id, self.passive(id)))
    }

    fn passive(&self, id: PartId) -> chip::Passive<'_> {
        chip::passive(&self.netlist.part(id).model).expect("a passive slot is a passive chip")
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
#[derive(Clone, Copy, Debug)]
pub(crate) struct Conditions<'h> {
    /// Siemens to ground from every net row and each internal row that is not bare.
    pub(crate) gmin: f64,
    /// Multiplies every independent source.
    pub(crate) source_scale: f64,
    /// The time this solve is for.
    pub(crate) at: Time,
    pub(crate) history: History<'h>,
    /// Each chip's state, by slot. Only an accepted time changes it.
    pub(crate) chips: &'h [chip::State],
    /// Each active chip's core in force, by active slot.
    pub(crate) cores: &'h [active::Core],
}

impl Conditions<'_> {
    pub(crate) fn side(&self) -> Side {
        match self.history {
            History::Restart { .. } => Side::After,
            History::Start | History::Step(_) => Side::Before,
        }
    }

    /// A source's netlist value in this solve, before `source_scale`.
    pub(crate) fn level(&self, waveform: &Waveform) -> f64 {
        self.side().level(waveform, self.at)
    }
}

/// Which interval at a time a timed part is read on: the one that ends there or the one that
/// starts there.
#[derive(Clone, Copy)]
pub(crate) enum Side {
    Before,
    After,
}

impl Side {
    pub(crate) fn level(self, waveform: &Waveform, at: Time) -> f64 {
        match self {
            Side::Before => waveform.before(at),
            Side::After => waveform.after(at),
        }
    }

    pub(crate) fn contact(self, schedule: &Schedule<Contact>, at: Time) -> Contact {
        *self.read(schedule, at)
    }

    pub(crate) fn read<T>(self, schedule: &Schedule<T>, at: Time) -> &T {
        match self {
            Side::Before => schedule.before(at),
            Side::After => schedule.after(at),
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
        cores: &[],
    };
}

/// How capacitors, inductors and contacts see the past.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum History<'h> {
    /// t = 0. A capacitor with `initial` is held there, and any other is open. An inductor is a
    /// short.
    Start,
    /// Just after a switch edge, a source jump or a chip drive change at `at`. Every capacitor is
    /// held at its voltage in `previous`, every inductor at its current there, and every contact
    /// and source is after the changes at or before `at`. The solve is algebraic. Its capacitor
    /// currents and inductor voltages are the post-edge values the next step starts from.
    Restart { previous: &'h Solution },
    /// A time step from the last accepted solution.
    Step(Integration<'h>),
}

/// One time step's view of the past: the accepted solution it starts from, and for BDF2 the
/// accepted point before that.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Integration<'h> {
    pub(crate) h_ps: u64,
    /// A store's slope at the step's end is `(c[0] * x + c[1] * x0 + c[2] * x1) / h`, with `x`
    /// its state there, `x0` in `previous` and `x1` in `before`.
    pub(crate) coefficients: [f64; 3],
    pub(crate) previous: &'h Solution,
    /// `None` for backward Euler, whose `c[2]` is 0.
    pub(crate) before: Option<&'h Solution>,
}

impl Integration<'_> {
    /// The step in seconds.
    pub(crate) fn h(&self) -> f64 {
        self.h_ps as f64 / 1e12
    }

    /// The companion of a store of `size`, a capacitance or an inductance, whose state is
    /// read from a solution by `state`: its slope times `size` at the step's end is
    /// `g * x + rhs`, as `(g, rhs)`. That is a capacitor's current from its voltage, and an
    /// inductor's voltage from its current.
    pub(crate) fn companion(&self, size: f64, state: impl Fn(&Solution) -> f64) -> (f64, f64) {
        let [c0, c1, c2] = self.coefficients;
        let k = size / self.h();
        let older = self.before.map_or(0.0, &state);
        (k * c0, k * (c1 * state(self.previous) + c2 * older))
    }
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
            part.model
                .initial(&mut junctions[layout.junction_slots(id)]);
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
    /// How far each unknown moved on the last iteration, which is how closely the solve has
    /// settled it. 0 on a linear circuit, which one solve settles exactly.
    pub(crate) moved: Vec<f64>,
}

#[derive(Debug)]
pub(crate) enum Failure {
    /// No pivot for the unknown at `row`: every entry left in its column is zero, or within
    /// the rounding of the sums that made it. No change of conditions fixes it.
    Singular {
        row: usize,
    },
    /// A stamped value, a factor or the solution overflowed, at `row`. The circuit's values are
    /// past what a double holds, and no change of conditions fixes that either.
    NonFinite {
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
}

impl Offender {
    pub(crate) fn describe(self, circuit: &Circuit) -> Ref {
        match self {
            Offender::Junction(part) => Ref::Part(circuit.netlist.part(part).name.clone()),
            Offender::Row(row) => circuit.layout.describe(&circuit.netlist, row),
        }
    }
}

/// Newton with junction limiting, at most `budget` solves, each on `solver`. On a linear
/// circuit the solver's memo answers a system it kept.
///
/// Converged means three checks pass in one iteration: no unknown moved more
/// than its tolerance, every junction's current check passed, and no junction
/// was limited. A capacitor's current is not one of those unknowns, as `worst_row` says.
///
/// On a linear circuit the second iteration would stamp the first one's system, solve it to the
/// same bits and pass every check, so the call returns after one solve with that iteration's
/// count.
pub(crate) fn newton(
    circuit: &Circuit,
    solver: &mut Solver,
    conditions: &Conditions,
    start: Iterate,
    budget: u32,
) -> Result<Converged, Failure> {
    let layout = &circuit.layout;
    let mut iterate = start;
    let mut offender = None;
    // The junction voltages of the iterate before last, whose space the next ones reuse.
    let mut spare = Vec::new();
    for iteration in 1..=budget {
        let system = solver.reset(layout, conditions.gmin);
        stamp(circuit, system, conditions, &iterate);
        // A linear circuit's systems repeat whenever its sources, contacts, drives, step and
        // held voltages do. With a junction they seldom repeat, and a miss copies the system.
        let solution = solver
            .solve(circuit.linear)
            .map_err(|failure| match failure {
                crate::sparse::Failure::Singular { col } => Failure::Singular { row: col },
                crate::sparse::Failure::NonFinite { col } => Failure::NonFinite { row: col },
            })?;

        let mut junctions: Vec<f64> = std::mem::take(&mut spare);
        junctions.clear();
        junctions.resize(layout.junctions(), 0.0);
        let mut worst_junction: Option<(f64, PartId)> = None;
        for (id, part) in circuit.netlist.parts() {
            let slots = layout.junction_slots(id);
            let probe = Probe {
                layout,
                part: id,
                x: &solution,
            };
            let update = part.model.update(
                &probe,
                conditions.at,
                conditions.side(),
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
        let before = std::mem::replace(
            &mut iterate,
            Iterate {
                solution,
                junctions,
            },
        );
        spare = before.junctions;
        offender = Some(match worst_junction {
            Some((_, part)) => Offender::Junction(part),
            None if row_ratio <= 1.0 => {
                let moved = match circuit.linear {
                    true => vec![0.0; layout.size()],
                    false => (before.solution.components().iter())
                        .zip(iterate.solution.components())
                        .map(|(before, after)| (after - before).abs())
                        .collect(),
                };
                return Ok(Converged {
                    iterate,
                    iterations: iteration,
                    moved,
                });
            }
            None if circuit.linear && iteration < budget => {
                return Ok(Converged {
                    iterate,
                    iterations: iteration + 1,
                    moved: vec![0.0; layout.size()],
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

/// Stamps every part and every active chip into `system`, linearised at `iterate`.
fn stamp(circuit: &Circuit, system: &mut System, conditions: &Conditions, iterate: &Iterate) {
    stamp_parts(circuit, system, conditions, iterate);
    for (k, chip) in circuit.actives.iter().enumerate() {
        active::stamp(
            chip,
            &conditions.cores[k],
            system,
            &iterate.solution,
            conditions.source_scale,
        );
    }
}

/// Stamps every netlist part into `system`, linearised at `iterate`.
fn stamp_parts(circuit: &Circuit, system: &mut System, conditions: &Conditions, iterate: &Iterate) {
    for (id, part) in circuit.netlist.parts() {
        element::Device::stamp(
            &part.model,
            &mut Stamp::new(system, &circuit.layout, id, iterate, conditions),
        );
    }
}

/// The row whose step is largest against its tolerance, and that ratio. Every capacitor's
/// current is left out. Each solve gives it as the voltage across the capacitor times a fixed
/// conductance, `C / h` on a step and C / 1 fs on a hold, so it has settled once the voltages
/// at its ends have. Its own 1 pA floor would need them to agree to `1e-12 * h / C` V, 1e-19 V
/// for 10 uF at 1 ps, far below one ulp of a volt.
fn worst_row(layout: &Layout, old: &[f64], new: &Solution) -> (usize, f64) {
    let mut worst = (0, 0.0);
    let rows = old.iter().zip(new.components()).zip(layout.floors());
    for (row, ((&before, &after), &floor)) in rows.enumerate() {
        let Some(floor) = floor else {
            continue;
        };
        let tolerance = RELTOL * before.abs().max(after.abs()) + floor;
        let ratio = (after - before).abs() / tolerance;
        if ratio > worst.1 {
            worst = (row, ratio);
        }
    }
    worst
}
