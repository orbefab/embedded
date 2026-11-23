use orbe_model::Time;

use crate::netlist::{Contact, Model, Netlist, PartId, Schedule, Waveform};

use crate::diagnostic::Diagnostic;
use crate::logic::Level;
use crate::name::{Name, Ref};

use crate::active;
use crate::chip;
use crate::element;
use crate::mna::{GMIN, Layout, Memo, Probe, Solution, Stamp, System};
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
    /// Fails with `solver.too_large` before the matrix is allocated.
    pub(crate) fn new(netlist: Netlist) -> Result<Circuit, Diagnostic> {
        Circuit::with_actives(netlist, Vec::new())
    }

    /// The netlist's active chips, then `extra`. Only a test passes `extra`, a script chip with
    /// no netlist kind.
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
        let layout = Layout::new(&netlist, passives);
        if layout.size() > crate::dense::MAX_UNKNOWNS {
            return Err(Diagnostic::SolverTooLarge {
                at: Time::ZERO,
                unknowns: layout.size(),
                limit: crate::dense::MAX_UNKNOWNS,
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
        // The pattern at t = 0 with every chip at power-on, from which the solve takes its
        // elimination order.
        // Each chip stamps with the core it holds, so no core is copied.
        let chips = circuit.power_on();
        let conditions = Conditions {
            chips: &chips,
            ..Conditions::NOMINAL
        };
        let mut system = System::new(&circuit.layout);
        system.reset(&circuit.layout, conditions.gmin);
        let initial = Iterate::initial(&circuit);
        stamp_parts(&circuit, &mut system, &conditions, &initial);
        for chip in &circuit.actives {
            active::stamp(
                chip,
                &chip.core,
                &mut system,
                &initial.solution,
                conditions.source_scale,
            );
        }
        circuit.layout.order_by(&system);
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
    /// Open every capacitor and ignore `initial`. A rating window's DC point uses this.
    /// The run's t = 0 solve leaves it clear, so a held capacitor keeps its voltage there.
    pub(crate) capacitors_open: bool,
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
        capacitors_open: false,
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

/// One capacitor in the circuit's charge list. Stiffness keys on this, so two capacitors of
/// one part can take different rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ChargeIndex(usize);

impl ChargeIndex {
    pub(crate) const fn new(index: usize) -> ChargeIndex {
        ChargeIndex(index)
    }
}

/// One inductor in the circuit's flux list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FluxIndex(usize);

impl FluxIndex {
    pub(crate) const fn new(index: usize) -> FluxIndex {
        FluxIndex(index)
    }
}

/// A capacitor or an inductor. Stiffness keys on this. Every capacitor orders before every
/// inductor, which is the order of every per-store list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Store {
    Charge(ChargeIndex),
    Flux(FluxIndex),
}

/// Backward Euler when `store` is in `stiff`, which is ascending. Otherwise `method`.
pub(crate) fn method_on(stiff: &[Store], method: Method, store: Store) -> Method {
    if stiff.binary_search(&store).is_ok() {
        Method::BackwardEuler
    } else {
        method
    }
}

/// One time step's view of the past. The last accepted solution is the whole history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Integration<'h> {
    pub(crate) method: Method,
    /// Stores that take backward Euler whatever `method` is. Ascending.
    pub(crate) stiff: &'h [Store],
    pub(crate) h_ps: u64,
    pub(crate) previous: &'h Solution,
}

impl Integration<'_> {
    /// The step in seconds.
    pub(crate) fn h(&self) -> f64 {
        self.h_ps as f64 / 1e12
    }

    pub(crate) fn method_of(&self, store: Store) -> Method {
        method_on(self.stiff, self.method, store)
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
                circuit.layout.describe(&circuit.netlist, row)
            }
        }
    }
}

/// Newton with junction limiting, at most `budget` solves. On a linear circuit `memo`, when
/// there is one, answers a system it kept.
///
/// Converged means three checks pass in one iteration: no unknown moved more
/// than its tolerance, every junction's current check passed, and no junction
/// was limited. A held capacitor's current is not one of those unknowns, as `worst_row` says.
///
/// On a linear circuit the second iteration would stamp the first one's system, solve it to the
/// same bits and pass every check, so the call returns after one solve with that iteration's
/// count.
pub(crate) fn newton(
    circuit: &Circuit,
    mut memo: Option<&mut Memo>,
    conditions: &Conditions,
    start: Iterate,
    budget: u32,
) -> Result<Converged, Failure> {
    let layout = &circuit.layout;
    let mut iterate = start;
    let mut offender = None;
    let mut system = System::new(layout);
    let held = !matches!(conditions.history, History::Step(_));
    for iteration in 1..=budget {
        system.reset(layout, conditions.gmin);
        stamp(circuit, &mut system, conditions, &iterate);
        // A linear circuit's systems repeat whenever its sources, contacts, drives, step and
        // held voltages do. With a junction they seldom repeat, and a miss copies the system.
        let solved = match memo.as_deref_mut() {
            Some(memo) if circuit.linear => memo.solve(&mut system),
            _ => system.solve(),
        };
        let solution = solved.map_err(|singular| Failure::Singular { row: singular.row })?;
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
            let update = part.model.update(
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
        let (worst_row, row_ratio) =
            worst_row(layout, held, iterate.solution.components(), &solution);

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
            None if circuit.linear && iteration < budget => {
                return Ok(Converged {
                    iterate,
                    iterations: iteration + 1,
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

/// The row whose step is largest against its tolerance, and that ratio. When `held`, every
/// capacitor's current is left out. A held capacitor's current is the voltage across its hold
/// times C / 1 fs, 1e8 S for 100 nF, and the voltages at its ends are checked. Its own 1 pA
/// floor would need them to agree to 1e-20 V, far below one ulp of a volt, so only a solve
/// that repeats to the bit passed it.
fn worst_row(layout: &Layout, held: bool, old: &[f64], new: &Solution) -> (usize, f64) {
    let mut worst = (0, 0.0);
    for (row, (&before, &after)) in old.iter().zip(new.components()).enumerate() {
        if held && layout.is_charge_row(row) {
            continue;
        }
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
