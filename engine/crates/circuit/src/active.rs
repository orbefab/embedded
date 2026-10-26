//! The active chip seam: a chip whose core runs on the circuit's clock. Its core sees the time,
//! names its own future events, holds state no stamp reads, and samples a voltage at a time it
//! picks. The circuit sees only its drive image, one `Drive` per output pin. `docs/engine.md`,
//! Active chips, holds the loop's protocol.

pub(crate) mod avr;
#[cfg(test)]
pub(crate) mod script;

use sim_core::name::Name;
use sim_core::netlist::{NetId, PartId};
use sim_core::serial::SerialByte;
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
    pub(crate) part: Option<PartId>,
    /// In the order of its core's image.
    pub(crate) outputs: Vec<Output>,
    /// The levels its core reads, in the order its core reads them.
    pub(crate) watches: Vec<Watch>,
    /// Its logic signals in name order, each with the index of the watch whose level it shows.
    pub(crate) signals: Vec<(&'static str, usize)>,
    /// The voltages its core reads at a sample, in the order it reads them.
    pub(crate) samples: Vec<Signal>,
    /// The datasheet name of each USART its core sends on, in its core's order.
    pub(crate) usarts: &'static [&'static str],
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
/// - `input` and `deliver` read nothing but the core and their arguments, so they take two
///   equal cores to two equal cores. A clone shares only what no call writes: the AVR
///   core's flash and decoded ops behind an `Arc`, and a script core's script.
#[derive(Debug)]
pub(crate) enum Core {
    Avr(Box<avr::Core>),
    #[cfg(test)]
    Script(script::Core),
}

impl Clone for Core {
    fn clone(&self) -> Self {
        match self {
            Core::Avr(core) => Core::Avr(core.clone()),
            #[cfg(test)]
            Core::Script(core) => Core::Script(core.clone()),
        }
    }

    /// Reuses the buffers of a core of the same kind. Every landing copies each core it commits
    /// over that core's last commit.
    fn clone_from(&mut self, source: &Self) {
        match (self, source) {
            (Core::Avr(core), Core::Avr(source)) => core.clone_from(source),
            #[cfg(test)]
            (core, source) => *core = source.clone(),
        }
    }
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

    /// Takes each byte a USART sent whose stop bit ended at or before `through`, in the order
    /// they ended on each USART, and drops every SPI byte and bus phase, which no output reads.
    fn deliver(&mut self, through: Time, sent: impl FnMut(Frame)) {
        match *self {
            Core::Avr(ref mut core) => core.deliver(through, sent),
            #[cfg(test)]
            Core::Script(_) => {}
        }
    }
}

/// A byte one of a core's USARTs sent, its start bit and the end of its stop bit on the run's
/// clock.
pub(crate) struct Frame {
    /// The index in its chip's `usarts`.
    pub(crate) usart: u8,
    pub(crate) byte: u8,
    pub(crate) start: Time,
    pub(crate) end: Time,
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
    /// The bytes landings delivered since the last `deliver`, in no order.
    staged: Vec<Staged>,
    /// Each USART, chips in part order, then each chip's in its core's order: the part's name
    /// and the USART's. A `SerialByte`'s `usart` indexes this.
    usarts: Vec<(Name, &'static str)>,
}

struct Ahead {
    core: Core,
    reach: Reach,
    delivered: Delivered,
}

/// A delivered byte and the end of its stop bit, which orders it.
struct Staged {
    end: Time,
    byte: SerialByte,
}

/// Every byte of one chip whose stop bit ended at or before `through` was staged, once.
///
/// `through` is never past the session's last accepted step. A commit is at a step being
/// accepted, and the session's delivery from the copies is through the accepted step. Nothing
/// accepted is undone, so a staged byte comes only from accepted history. A re-solve after an
/// input can move steps and watch flips anywhere after the accepted step, and none of that
/// span is delivered yet.
///
/// A byte is staged only when its end is in `(through, to]`, and `through` then becomes `to`,
/// so none is staged twice. `to` is never before `through`, because each delivery is through
/// the step being accepted or the last accepted one, and neither moves back. A byte leaves a
/// core only here, and one at or below `through` was staged from the copy that held it then. The committed core is the copy
/// as it was delivered at its commit. A rewind, a restart's input and a live input all go to a
/// time after the accepted step, so the core runs again from its commit on the accepted inputs
/// and sends the same bytes up to `through`, which are dropped. A twin was delivered through
/// its time by its commit and has not run, and an input ends no frame, so an input on a twin
/// delivers nothing. A stop and a run that idles after its last byte are the session's
/// delivery through the accepted step, which every copy has done.
struct Delivered {
    through: Time,
    /// The index in `Actives::usarts` of the chip's first USART.
    usart: u16,
}

impl Delivered {
    fn deliver(&mut self, core: &mut Core, to: Time, staged: &mut Vec<Staged>) {
        let (through, usart) = (self.through, self.usart);
        core.deliver(to, |frame| {
            if frame.end > through {
                staged.push(Staged {
                    end: frame.end,
                    byte: SerialByte {
                        at: frame.start,
                        usart: usart + u16::from(frame.usart),
                        byte: frame.byte,
                    },
                });
            }
        });
        debug_assert!(through <= to, "a delivery never goes back");
        self.through = to;
    }
}

#[derive(Clone, Copy)]
enum Reach {
    /// Everything up to here is done.
    Done(Time),
    /// Everything before its stop is done, and the stop is not yet landed.
    Stopped(Stop),
    /// Everything up to here is done, and the copy equals the committed core: `commit` made it
    /// here and nothing has run or changed either since. Running the copy leaves this state.
    Twin(Time),
}

impl Ahead {
    /// Puts the copy at `at` with everything there done but the stop's sample and inputs. A
    /// copy that ran past `at` runs again from the committed core.
    fn rewind(&mut self, committed: &Core, at: Time) {
        let reached = match self.reach {
            Reach::Done(reached) | Reach::Twin(reached) => reached,
            Reach::Stopped(stop) => stop.at,
        };
        if reached > at {
            self.core.clone_from(committed);
            let early = self.core.run_until(at);
            assert_eq!(early, None, "a core stopped where its copy did not");
        }
        self.reach = Reach::Done(at);
    }

    /// Delivers the copy through its time and commits it, so no later copy of either core
    /// carries what it delivered.
    fn commit(&mut self, committed: &mut Core, staged: &mut Vec<Staged>) {
        let Reach::Done(at) = self.reach else {
            panic!("a copy commits only where it was rewound");
        };
        self.delivered.deliver(&mut self.core, at, staged);
        committed.clone_from(&self.core);
        self.reach = Reach::Twin(at);
    }

    /// Its watches' levels from `at` on, after the landing at `at`, then committed. A twin of
    /// the committed core takes them on both cores, which leaves the state a fresh copy would.
    fn input(&mut self, committed: &mut Core, at: Time, levels: &[bool], staged: &mut Vec<Staged>) {
        if !matches!(self.reach, Reach::Twin(twin) if twin == at) {
            self.rewind(committed, at);
            self.core.input(at, levels);
            assert_eq!(
                self.core.drive(),
                committed.drive(),
                "an input on the held solve never changes an image"
            );
            self.commit(committed, staged);
            return;
        }
        committed.input(at, levels);
        assert_eq!(
            committed.drive(),
            self.core.drive(),
            "an input on the held solve never changes an image"
        );
        self.core.input(at, levels);
    }

    /// Whether the copy has done everything up to `at`.
    fn has_done(&self, at: Time) -> bool {
        match self.reach {
            Reach::Done(reached) | Reach::Twin(reached) => reached >= at,
            Reach::Stopped(stop) => stop.at > at,
        }
    }
}

impl Actives {
    /// The power-on cores, given their levels on the operating point at t = 0.
    pub(crate) fn new(circuit: &Circuit, levels: &Levels) -> Actives {
        let mut usarts = Vec::new();
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
            .zip(&circuit.actives)
            .map(|(core, chip)| {
                let delivered = Delivered {
                    through: Time::ZERO,
                    usart: u16::try_from(usarts.len()).expect("a run has few USARTs"),
                };
                for &usart in chip.usarts {
                    let part = chip.part.expect("a chip with a USART is a part");
                    usarts.push((circuit.netlist.part(part).name.clone(), usart));
                }
                Ahead {
                    core: core.clone(),
                    reach: Reach::Done(Time::ZERO),
                    delivered,
                }
            })
            .collect();
        Actives {
            committed,
            ahead,
            staged: Vec::new(),
            usarts,
        }
    }

    /// The power-on cores, for the operating point that comes before `new`.
    pub(crate) fn power_on(circuit: &Circuit) -> Vec<Core> {
        circuit
            .actives
            .iter()
            .map(|chip| chip.core.clone())
            .collect()
    }

    pub(crate) fn usarts(&self) -> &[(Name, &'static str)] {
        &self.usarts
    }

    pub(crate) fn cores(&self) -> &[Core] {
        &self.committed
    }

    /// The copies each core runs ahead on. The next kept copy is cloned from these.
    #[cfg(test)]
    pub(crate) fn ahead_cores(&self) -> impl Iterator<Item = &Core> {
        self.ahead.iter().map(|ahead| &ahead.core)
    }

    /// The first stop of any core up to `limit`, or `limit`. A step from the accepted time to
    /// here solves with every image constant. Runs a core only past where it has reached.
    pub(crate) fn horizon(&mut self, limit: Time) -> Time {
        let mut horizon = limit;
        for ahead in &mut self.ahead {
            if let Reach::Done(reached) | Reach::Twin(reached) = ahead.reach
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
    /// With `keep`, so is every other core, and a later rewind runs again from `at` at the
    /// earliest. A copy that ran past `at`, as a retry shorter than its first try leaves it, runs
    /// again from its last commit to `at` first. Returns whether an image changed, and how many
    /// cores stopped at `at`.
    pub(crate) fn land(
        &mut self,
        circuit: &Circuit,
        at: Time,
        x: &Solution,
        before: &Levels,
        after: &Levels,
        keep: bool,
    ) -> (bool, u64) {
        let mut changed = false;
        let mut stops = 0u64;
        for (k, (committed, ahead)) in self.committed.iter_mut().zip(&mut self.ahead).enumerate() {
            let watched = circuit.active_watches(k);
            let levels = after.of(watched.clone());
            let flipped = before.of(watched) != levels;
            let stop = match ahead.reach {
                Reach::Stopped(stop) if stop.at == at => Some(stop),
                _ => None,
            };
            if stop.is_none() && !flipped && !keep {
                continue;
            }
            if stop.is_some() {
                stops += 1;
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
            ahead.commit(committed, &mut self.staged);
        }
        (changed, stops)
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
            ahead.input(committed, at, levels, &mut self.staged);
        }
    }

    /// Delivers each copy through `through`, which every copy has done everything up to. Returns
    /// every byte delivered since the last call, in the order stop bits ended, then in `usarts`
    /// order.
    pub(crate) fn deliver(&mut self, through: Time) -> impl Iterator<Item = SerialByte> {
        for ahead in &mut self.ahead {
            assert!(ahead.has_done(through), "a copy delivers only what it did");
            ahead
                .delivered
                .deliver(&mut ahead.core, through, &mut self.staged);
        }
        self.staged
            .sort_unstable_by_key(|staged| (staged.end, staged.byte.usart));
        self.staged.drain(..).map(|staged| staged.byte)
    }
}
