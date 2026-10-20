//! The active chip seam: a chip whose core runs on the circuit's clock. Its core sees the time,
//! names its own future events, holds state no stamp reads, and samples a voltage at a time it
//! picks. The circuit sees only its drive image, one `Drive` per output pin. `docs/engine.md`,
//! Active chips, holds the loop's protocol.

pub(crate) mod avr;
#[cfg(test)]
pub(crate) mod script;

use sim_core::netlist::{NetId, PartId};
use sim_core::time::Time;

use crate::chip::Active;
use crate::mna::{Solution, System};
use crate::newton::Circuit;
use crate::watch::{Levels, Signal, Watch};

/// What one output pin does. The image, one per output pin, is all the stamp reads, and the
/// only thing the restart test compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drive {
    /// Through the high stage from the chip's own VCC pin.
    High,
    /// Through the low stage into the chip's own GND pin.
    Low,
    /// Through the output's pull-up from the chip's own VCC pin.
    PullUp,
    /// No stage, only the leakage every drive state stamps.
    Off,
}

/// One side of an output stage, through `resistance`. The high stage sits `drop` below VCC, and
/// the low stage `drop` above GND.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stage {
    /// Volt.
    pub(crate) drop: f64,
    /// Ohm, greater than 0.
    pub(crate) resistance: f64,
}

/// An output pin and the stages that drive it from the chip's rails.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Output {
    pub(crate) pin: NetId,
    pub(crate) vcc: NetId,
    pub(crate) gnd: NetId,
    pub(crate) high: Stage,
    pub(crate) low: Stage,
    /// Ohm, greater than 0. `None` on a pin whose core never drives [`Drive::PullUp`].
    pub(crate) pull_up: Option<f64>,
    /// Ohm, greater than 0, to VCC and again to GND in every drive state. `None` on a pin that
    /// leaks nothing.
    pub(crate) leakage: Option<f64>,
}

/// An active chip as the circuit sees it. Read-only for the whole analysis.
#[derive(Clone, Debug)]
pub(crate) struct Chip {
    /// `None` only on a test's script chip, which no netlist part builds.
    #[expect(dead_code, reason = "the logic signals that name the part come next")]
    pub(crate) part: Option<PartId>,
    /// In the order of its core's image.
    pub(crate) outputs: Vec<Output>,
    /// The levels its core reads, in the order its core reads them.
    pub(crate) watches: Vec<Watch>,
    /// The voltages its core reads at a sample, in the order it reads them.
    pub(crate) samples: Vec<Signal>,
    /// The core at power-on.
    pub(crate) core: Core,
}

/// Where a core stopped before its limit: at `at` its image may differ, or it samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stop {
    pub(crate) at: Time,
    /// Whether it samples at `at`. It reads every one of its samples on the step's solution
    /// there, before any change at `at`.
    pub(crate) sample: bool,
}

/// A core's state, everything it remembers. `Clone` is the checkpoint.
///
/// Every kind keeps this contract:
/// - `run_until(limit)` runs from the last time it reached and stops at the first time in
///   `(reached, limit]` where its image may change or it samples, or returns `None` once it has
///   done everything up to `limit`. Two calls to `l1` then `l2` leave the state one call to `l2`
///   does. All of a time's effects are one stop.
/// - A clocked kind runs to cycle `floor(limit_ps * f / 1e12)` and puts a stop at cycle `c` at
///   `ceil(c * 1e12 / f)` ps, counting from its clock domain's last frequency change.
/// - `sample` and `input` never change the image. What they cause comes at least 1 ps later.
#[derive(Clone, Debug)]
pub(crate) enum Core {
    Avr(Box<avr::Core>),
    #[cfg(test)]
    Script(script::Core),
}

impl Core {
    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        match *self {
            Core::Avr(ref mut core) => core.run_until(limit),
            #[cfg(test)]
            Core::Script(ref mut core) => core.run_until(limit),
        }
    }

    fn drive(&self) -> &[Drive] {
        match *self {
            Core::Avr(ref core) => core.drive(),
            #[cfg(test)]
            Core::Script(ref core) => core.drive(),
        }
    }

    /// The volts of each of its samples at its stop at `at`.
    fn sample(&mut self, at: Time, volts: &[f64]) {
        match *self {
            Core::Avr(ref mut core) => core.sample(at, volts),
            #[cfg(test)]
            Core::Script(ref mut core) => core.sample(at, volts),
        }
    }

    /// Its watches' levels from `at` on.
    fn input(&mut self, at: Time, levels: &[bool]) {
        match *self {
            Core::Avr(ref mut core) => core.input(at, levels),
            #[cfg(test)]
            Core::Script(ref mut core) => core.input(at, levels),
        }
    }
}

pub(crate) fn chip(part: PartId, kind: Active<'_>) -> Chip {
    match kind {
        Active::Atmega328p(mcu) => avr::chip(part, mcu),
    }
}

/// The drops are sources, so they scale.
pub(crate) fn stamp(chip: &Chip, core: &Core, sys: &mut System, source_scale: f64) {
    let drive = core.drive();
    debug_assert_eq!(drive.len(), chip.outputs.len());
    for (output, drive) in chip.outputs.iter().zip(drive) {
        if let Some(ohm) = output.leakage {
            sys.conductance(output.pin, output.vcc, 1.0 / ohm);
            sys.conductance(output.pin, output.gnd, 1.0 / ohm);
        }
        match drive {
            Drive::High => {
                let g = 1.0 / output.high.resistance;
                sys.conductance(output.pin, output.vcc, g);
                sys.current(output.pin, output.vcc, output.high.drop * g * source_scale);
            }
            Drive::Low => {
                let g = 1.0 / output.low.resistance;
                sys.conductance(output.pin, output.gnd, g);
                sys.current(output.gnd, output.pin, output.low.drop * g * source_scale);
            }
            Drive::PullUp => {
                let ohm = output
                    .pull_up
                    .expect("a pull-up drive on a pin that has one");
                sys.conductance(output.pin, output.vcc, 1.0 / ohm);
            }
            Drive::Off => {}
        }
    }
}

/// Every active chip's core in the loop. `committed` is in force: every solve stamps its image.
/// `ahead` is each committed core run on with the same inputs, never behind the accepted time.
pub(crate) struct Actives {
    committed: Vec<Core>,
    ahead: Vec<Ahead>,
}

struct Ahead {
    core: Core,
    reach: Reach,
}

#[derive(Clone, Copy)]
enum Reach {
    /// Everything up to here is done.
    Done(Time),
    /// Everything before its stop is done, and the stop is not yet landed.
    Stopped(Stop),
}

impl Ahead {
    /// Puts the copy at `at` with everything there done but the stop's sample and inputs. A
    /// copy that ran past a flip at `at` runs again from the committed core.
    fn rewind(&mut self, committed: &Core, at: Time) {
        let reached = match self.reach {
            Reach::Done(reached) => reached,
            Reach::Stopped(stop) => stop.at,
        };
        if reached > at {
            self.core.clone_from(committed);
            let early = self.core.run_until(at);
            assert_eq!(early, None, "a core stopped where its copy did not");
        }
        self.reach = Reach::Done(at);
    }
}

impl Actives {
    /// The power-on cores, given their levels on the operating point at t = 0.
    pub(crate) fn new(circuit: &Circuit, levels: &Levels) -> Actives {
        let committed: Vec<Core> = circuit
            .actives
            .iter()
            .enumerate()
            .map(|(k, chip)| {
                let mut core = chip.core.clone();
                core.input(Time::ZERO, levels.of(circuit.active_watches(k)));
                assert_eq!(
                    core.drive(),
                    chip.core.drive(),
                    "the power-on input never changes an image"
                );
                core
            })
            .collect();
        let ahead = committed
            .iter()
            .map(|core| Ahead {
                core: core.clone(),
                reach: Reach::Done(Time::ZERO),
            })
            .collect();
        Actives { committed, ahead }
    }

    /// The power-on cores, for the operating point that comes before `new`.
    pub(crate) fn power_on(circuit: &Circuit) -> Vec<Core> {
        circuit
            .actives
            .iter()
            .map(|chip| chip.core.clone())
            .collect()
    }

    pub(crate) fn cores(&self) -> &[Core] {
        &self.committed
    }

    /// The first stop of any core up to `limit`, or `limit`. A step from the accepted time to
    /// here solves with every image constant. Runs a core only past where it has reached.
    pub(crate) fn horizon(&mut self, limit: Time) -> Time {
        let mut horizon = limit;
        for ahead in &mut self.ahead {
            if let Reach::Done(reached) = ahead.reach
                && reached < limit
            {
                ahead.reach = match ahead.core.run_until(limit) {
                    Some(stop) => {
                        assert!(
                            stop.at > reached,
                            "a core stops only after where it reached"
                        );
                        Reach::Stopped(stop)
                    }
                    None => Reach::Done(limit),
                };
            }
            if let Reach::Stopped(stop) = ahead.reach {
                horizon = horizon.min(stop.at);
            }
        }
        horizon
    }

    /// A step landed at `at`, no later than the last `horizon`, with `after` read on `x` and
    /// `before` in force until then. Each core that stops at `at` takes its sample from `x`,
    /// each core whose watches flipped takes them, and each such core is committed at `at`.
    /// Whether an image changed, which restarts.
    pub(crate) fn land(
        &mut self,
        circuit: &Circuit,
        at: Time,
        x: &Solution,
        before: &Levels,
        after: &Levels,
    ) -> bool {
        let mut changed = false;
        for (k, (committed, ahead)) in self.committed.iter_mut().zip(&mut self.ahead).enumerate() {
            let watched = circuit.active_watches(k);
            let levels = after.of(watched.clone());
            let flipped = before.of(watched) != levels;
            let stop = match ahead.reach {
                Reach::Stopped(stop) if stop.at == at => Some(stop),
                _ => None,
            };
            if stop.is_none() && !flipped {
                continue;
            }
            ahead.rewind(committed, at);
            let image = ahead.core.drive().to_vec();
            changed |= image != committed.drive();
            if stop.is_some_and(|stop| stop.sample) {
                let volts: Vec<f64> = circuit.actives[k]
                    .samples
                    .iter()
                    .map(|signal| signal.at(x))
                    .collect();
                ahead.core.sample(at, &volts);
            }
            if flipped {
                ahead.core.input(at, levels);
            }
            assert_eq!(
                ahead.core.drive(),
                image,
                "a sample or an input at a landing never changes an image"
            );
            committed.clone_from(&ahead.core);
        }
        changed
    }

    /// After a restart at `at`, with `after` read on the held solve and `before` read on the
    /// step. Every stop at `at` is landed by then, so each core whose watches flipped only takes
    /// them.
    pub(crate) fn input(&mut self, circuit: &Circuit, at: Time, before: &Levels, after: &Levels) {
        for (k, (committed, ahead)) in self.committed.iter_mut().zip(&mut self.ahead).enumerate() {
            let watched = circuit.active_watches(k);
            let levels = after.of(watched.clone());
            if before.of(watched) == levels {
                continue;
            }
            ahead.rewind(committed, at);
            ahead.core.input(at, levels);
            assert_eq!(
                ahead.core.drive(),
                committed.drive(),
                "an input on the held solve never changes an image"
            );
            committed.clone_from(&ahead.core);
        }
    }
}
