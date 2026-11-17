//! The active chip seam: a chip whose core runs on the circuit's clock. Its core sees the time,
//! names its own future events, holds state no stamp reads, and samples a voltage at a time it
//! picks. The circuit sees only its drive image, one `Drive` per output pin. `docs/engine/engine.md`,
//! Active chips, holds the loop's protocol.

use orbe_mcu::{Frame, Phase};
use orbe_model::{Drawn, Drive, Event, Quantity, Shows, Stop, Time};

use crate::netlist::{NetId, PartId};

pub(crate) mod duty;
pub(crate) mod i2c;
pub(crate) mod native;
pub(crate) mod pulse;
pub(crate) mod replay;
#[cfg(test)]
pub(crate) mod script;
pub(crate) mod spi;

use crate::device::{cst816, dht22, pulses_replay, spi_replay, st77916, ws2812};
use crate::mcu::{self, Mcu, atmega328p, esp32s3};
use native::Hosted;

use crate::device::Declared;
use crate::diagnostic::Diagnostic;
use crate::display::Rectangle;
use crate::i2c::I2cRow;
use crate::link::{LinkRow, Record, RecordKind};
use crate::name::{Name, Ref};

use crate::serial::SerialByte;

use crate::chip::Active;
use crate::mna::{Solution, System};
use crate::newton::Circuit;
use crate::watch::{Levels, Signal, Watch};
use crate::world::World;

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
    /// Whether its core has a TWI master, whose phases go to `i2c.csv`.
    pub(crate) twi: bool,
    /// Whether its core sends records, which go to `link.csv`.
    pub(crate) sends: bool,
    /// The records it takes.
    pub(crate) takes: Takes,
    /// The core at power-on.
    pub(crate) core: Core,
}

/// The nets of a chip's SPI target pins. It takes each `Spi` record whose CS and SCLK are on
/// these nets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpiTarget {
    pub(crate) cs: NetId,
    pub(crate) sclk: NetId,
}

/// The record pins of a chip: its SPI target's, and the pin that takes `Duty` records. A chip
/// with neither takes no record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Takes {
    pub(crate) spi: Option<SpiTarget>,
    pub(crate) duty: Option<NetId>,
    pub(crate) pulses: Option<NetId>,
}

impl Takes {
    fn record(self, record: &Record) -> bool {
        match &record.kind {
            RecordKind::Spi { nets, .. } => self
                .spi
                .is_some_and(|target| nets.cs == Some(target.cs) && nets.sclk == Some(target.sclk)),
            RecordKind::Duty { net, .. } => self.duty == Some(*net),
            RecordKind::Pulses { net, .. } => self.pulses == Some(*net),
        }
    }
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
/// - `sample`, `input` and `take` never change the image. What they cause comes at least 1 ps
///   later.
/// - Each record a core sends ends at a stop with `record` set, and the core holds it from that
///   stop until `drain_records` takes it. A core numbers its records from 0, and the count
///   carries in its copies.
/// - A core that takes records makes its display rows only in `take`, at the landing's time,
///   and holds them until `drain_rows` takes them.
/// - `input`, `take`, `deliver`, `drain_records` and `drain_rows` read nothing but the core and
///   their arguments, so they take two equal cores to two equal cores. A clone shares only what
///   no call writes: the AVR core's flash and decoded ops behind an `Arc`, a replay's records,
///   and a script core's script.
#[derive(Debug)]
pub(crate) enum Core {
    /// A device on the `orbe-model` contract.
    Device(Box<dyn Hosted>),
    /// A microcontroller on the `orbe-mcu` contract.
    Mcu(Mcu),
    #[cfg(test)]
    Script(Box<script::Core>),
}

impl Clone for Core {
    fn clone(&self) -> Self {
        match self {
            Core::Device(core) => Core::Device(core.boxed()),
            Core::Mcu(mcu) => Core::Mcu(mcu.clone()),
            #[cfg(test)]
            Core::Script(core) => Core::Script(core.clone()),
        }
    }

    /// Reuses the buffers of a core of the same kind. Every landing copies each core it commits
    /// over that core's last commit. A copy of an `esp32s3` core starts with an empty table of
    /// decoded instructions, so a landing leaves the running core's table in place and a rewind
    /// starts that core empty.
    fn clone_from(&mut self, source: &Self) {
        match (self, source) {
            (Core::Device(core), Core::Device(source)) => core.copy_from(&**source),
            (Core::Mcu(mcu), Core::Mcu(source)) => mcu.clone_from(source),
            (core, source) => *core = source.clone(),
        }
    }
}

impl Core {
    fn run_until(&mut self, world: &World, limit: Time) -> Option<Stop> {
        match *self {
            Core::Device(ref mut core) => core.run_until(world, limit),
            Core::Mcu(ref mut mcu) => mcu.run_until(limit),
            #[cfg(test)]
            Core::Script(ref mut core) => core.run_until(limit),
        }
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        match *self {
            Core::Device(ref core) => core.drive(),
            Core::Mcu(ref mcu) => mcu.drive(),
            #[cfg(test)]
            Core::Script(ref core) => core.drive(),
        }
    }

    /// The volts of each of its samples at its stop at `at`.
    fn sample(&mut self, world: &World, at: Time, volts: &[f64]) {
        match *self {
            Core::Device(ref mut core) => core.sample(world, at, volts),
            Core::Mcu(ref mut mcu) => mcu.sample(at, volts),
            #[cfg(test)]
            Core::Script(ref mut core) => core.sample(at, volts),
        }
    }

    /// Its watches' levels from `at` on.
    fn input(&mut self, world: &World, at: Time, levels: &[bool]) {
        match *self {
            Core::Device(ref mut core) => core.input(world, at, levels),
            Core::Mcu(ref mut mcu) => mcu.input(at, levels),
            #[cfg(test)]
            Core::Script(ref mut core) => core.input(at, levels),
        }
    }

    /// Takes each byte a USART sent whose stop bit ended at or before `through`, in the order
    /// they ended on each USART, and each TWI phase that ended by then, in the order they
    /// ended. Drops every SPI byte, which no output reads.
    fn deliver(&mut self, through: Time, sent: impl FnMut(Frame), saw: impl FnMut(Phase)) {
        match *self {
            Core::Mcu(ref mut mcu) => mcu.deliver(through, sent, saw),
            Core::Device(_) => {}
            #[cfg(test)]
            Core::Script(ref mut core) => core.deliver(through, sent),
        }
    }

    /// The records it sent that it still holds, in the order it sent them.
    fn records(&self) -> &[Record] {
        match *self {
            Core::Device(ref core) => core.records(),
            Core::Mcu(ref mcu) => mcu.records(),
            #[cfg(test)]
            Core::Script(ref core) => core.records(),
        }
    }

    /// Hands over and drops each record it holds that ended at or before `through`, in the
    /// order it sent them.
    fn drain_records(&mut self, through: Time, mut sent: impl FnMut(Record)) {
        match *self {
            Core::Device(ref mut core) => core.drain_records(through, &mut sent),
            Core::Mcu(ref mut mcu) => mcu.drain_records(through, sent),
            #[cfg(test)]
            Core::Script(ref mut core) => core.drain_records(through, sent),
        }
    }

    /// Takes the records that end at `at` on its record pins, ordered by end, then by sender name,
    /// then by sequence.
    fn take(&mut self, world: &World, at: Time, records: &[LinkRow]) {
        match *self {
            Core::Device(ref mut core) => core.take(world, at, records),
            Core::Mcu(_) => {
                unreachable!(
                    "a chip with no record pins took {} records at {at:?}",
                    records.len()
                )
            }
            #[cfg(test)]
            Core::Script(ref mut core) => core.take(at, records),
        }
    }

    /// Hands over and drops each display row it made, with its time, in the order it made them.
    fn drain_rows(&mut self, mut sent: impl FnMut(Time, Drawn)) {
        match *self {
            Core::Device(ref mut core) => core.drain_rows(&mut sent),
            Core::Mcu(_) => {}
            #[cfg(test)]
            Core::Script(_) => {}
        }
    }

    /// The part of one `Pulses` record sent by `at`, where the run ends inside it.
    fn cut(&mut self, world: &World, at: Time, record: &Record) {
        match self {
            Core::Device(core) => core.cut(world, at, record),
            Core::Mcu(_) => {}
            #[cfg(test)]
            Core::Script(_) => {}
        }
    }

    /// What it shows besides its drives, as its device declares.
    fn shows(&self) -> Shows {
        match self {
            Core::Device(core) => core.shows(),
            Core::Mcu(_) => Shows::Nothing,
            #[cfg(test)]
            Core::Script(_) => Shows::Nothing,
        }
    }

    /// The world quantities its device reads. A chip reads none.
    fn reads(&self) -> &'static [Quantity] {
        match self {
            Core::Device(core) => core.reads(),
            Core::Mcu(_) => &[],
            #[cfg(test)]
            Core::Script(_) => &[],
        }
    }

    /// An event its device declares, at `at`. The image stays as it is.
    fn event(&mut self, world: &World, at: Time, event: Event<'_>) {
        match self {
            Core::Device(core) => core.event(world, at, event),
            Core::Mcu(_) => unreachable!("only a device takes an event"),
            #[cfg(test)]
            Core::Script(_) => unreachable!("only a device takes an event"),
        }
    }
}

pub(crate) fn chip(part: PartId, kind: Active<'_>) -> Chip {
    match kind {
        Active::Atmega328p(mcu) => atmega328p::chip(part, mcu),
        Active::SpiReplay(replay) => spi_replay::chip(part, replay),
        Active::PulsesReplay(replay) => pulses_replay::chip(part, replay),
        Active::St77916(panel) => st77916::chip(part, panel),
        Active::Cst816(dev) => cst816::chip(part, dev),
        Active::Dht22(dev) => dht22::chip(part, dev),
        Active::Esp32s3(mcu) => esp32s3::chip(part, mcu),
        Active::Ws2812(chain) => ws2812::chip(part, chain),
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
    /// What landings delivered since the last `deliver`, in no order.
    staged: Staging,
    /// Each USART, chips in part order, then each chip's in its core's order: the part's name
    /// and the USART's. A `SerialByte`'s `usart` indexes this.
    usarts: Vec<(Name, &'static str)>,
    /// The name of each part whose core has a TWI, in part order. An `I2cRow`'s `twi` indexes
    /// this.
    twis: Vec<Name>,
    /// The name of each part whose core sends records, in part order. A `LinkRow`'s `sender`
    /// indexes this.
    senders: Vec<Name>,
    /// The name of each display part, in part order. A `Rectangle`'s `part` indexes this.
    displays: Vec<Name>,
    /// The name of each LED chain, in part order. A latch's `part` indexes this.
    leds: Vec<Name>,
}

struct Ahead {
    core: Core,
    reach: Reach,
    delivered: Delivered,
}

/// A delivered byte, the end of its stop bit, and how many bytes its chip staged before it.
struct Staged {
    end: Time,
    byte: SerialByte,
    seq: u64,
}

/// A delivered TWI phase and its end, which orders it.
struct StagedPhase {
    end: Time,
    row: I2cRow,
}

#[derive(Default)]
struct Staging {
    bytes: Vec<Staged>,
    phases: Vec<StagedPhase>,
    records: Vec<LinkRow>,
    rows: Vec<Rectangle>,
}

/// Every byte of one chip whose stop bit ended at or before `through` was staged, once, and
/// so was every TWI phase that ended by then.
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
///
/// A phase can end on an input, at the time the input lands, which a delivery may already have
/// passed: a STOP ends on the SDA rise the TWI reads. So a phase is staged by its count, not its
/// end. `phases` is how many were staged. Every run of a core numbers its phases from its
/// commit, as the first run did, so a phase run again has the count it was staged with.
///
/// Every record that ended at or before `records` was staged, once. Only a commit moves
/// `records`, to the commit's time, and drains each record that ended by then from the copy it
/// commits. A record's end is a stop of its sender, and every stop lands and commits its core,
/// so a record is staged in the landing at its end, by the commit that also hands it to its
/// receivers. The committed core then holds no record, and a run again from it makes only
/// records after its commit.
///
/// A display row is made only where a core takes records, in a landing that commits it at
/// once, and that commit stages it. So the committed core holds no row.
struct Delivered {
    through: Time,
    /// The index in `Actives::usarts` of the chip's first USART.
    usart: u16,
    /// The index in `Actives::twis` of the chip's TWI. A chip with none delivers no phase.
    twi: u16,
    phases: u64,
    /// How many bytes it staged, which orders bytes that end at one time on one USART.
    bytes: u64,
    records: Time,
    /// The index in `Actives::senders` of the chip. A chip with none sends no record.
    sender: u16,
    /// The index in `Actives::displays` of the chip. A chip that is no display makes no row.
    display: u16,
}

impl Delivered {
    fn deliver(&mut self, core: &mut Core, to: Time, staged: &mut Staging) {
        let (through, usart, twi) = (self.through, self.usart, self.twi);
        let staged_phases = &mut self.phases;
        let staged_bytes = &mut self.bytes;
        let Staging { bytes, phases, .. } = staged;
        core.deliver(
            to,
            |frame| {
                if frame.end > through {
                    bytes.push(Staged {
                        end: frame.end,
                        byte: SerialByte {
                            at: frame.start,
                            usart: usart + u16::from(frame.usart),
                            byte: frame.byte,
                        },
                        seq: *staged_bytes,
                    });
                    *staged_bytes += 1;
                }
            },
            |phase| {
                if phase.count >= *staged_phases {
                    *staged_phases = phase.count + 1;
                    phases.push(StagedPhase {
                        end: phase.end,
                        row: I2cRow {
                            at: phase.start,
                            twi,
                            phase: phase.phase,
                        },
                    });
                }
            },
        );
        debug_assert!(through <= to, "a delivery never goes back");
        self.through = to;
    }

    /// Stages each record the copy holds that ended at or before `to`, drops it from the copy,
    /// and moves `records` to `to`. Only `Ahead::commit` calls this, never the session's
    /// delivery.
    fn take_records(&mut self, core: &mut Core, to: Time, staged: &mut Staging) {
        let (after, sender) = (self.records, self.sender);
        core.drain_records(to, |record| {
            debug_assert!(record.end > after, "a record is staged once");
            staged.records.push(LinkRow { sender, record });
        });
        debug_assert!(after <= to, "a record cursor never goes back");
        self.records = to;
    }

    /// Stages each display row the copy made, all at `at`, and drops it from the copy. Only
    /// `Ahead::commit` calls this.
    fn take_rows(&self, core: &mut Core, at: Time, staged: &mut Staging) {
        let part = self.display;
        core.drain_rows(|made, row| {
            debug_assert_eq!(made, at, "a row is made at the landing that commits it");
            staged.rows.push(Rectangle {
                at: made,
                part,
                x0: row.x0,
                y0: row.y0,
                x1: row.x1,
                y1: row.y1,
                bytes: row.bytes,
                hash: row.hash,
            });
        });
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
    fn rewind(&mut self, world: &World, committed: &Core, at: Time) {
        let reached = match self.reach {
            Reach::Done(reached) | Reach::Twin(reached) => reached,
            Reach::Stopped(stop) => stop.at,
        };
        if reached > at {
            self.core.clone_from(committed);
            let early = self.core.run_until(world, at);
            assert_eq!(early, None, "a core stopped where its copy did not");
        }
        self.reach = Reach::Done(at);
    }

    /// Delivers the copy's bytes, phases and records through its time and commits it, so no
    /// later copy of either core carries what it delivered.
    fn commit(&mut self, committed: &mut Core, staged: &mut Staging) {
        let Reach::Done(at) = self.reach else {
            panic!("a copy commits only where it was rewound");
        };
        self.delivered.deliver(&mut self.core, at, staged);
        self.delivered.take_records(&mut self.core, at, staged);
        self.delivered.take_rows(&mut self.core, at, staged);
        committed.clone_from(&self.core);
        self.reach = Reach::Twin(at);
    }

    /// Its watches' levels from `at` on, after the landing at `at`, then committed. A twin of
    /// the committed core takes them on both cores, which leaves the state a fresh copy would.
    fn input(
        &mut self,
        world: &World,
        committed: &mut Core,
        at: Time,
        levels: &[bool],
        staged: &mut Staging,
    ) {
        if !matches!(self.reach, Reach::Twin(twin) if twin == at) {
            self.rewind(world, committed, at);
            self.core.input(world, at, levels);
            assert_eq!(
                self.core.drive(),
                committed.drive(),
                "an input on the held solve never changes an image"
            );
            self.commit(committed, staged);
            return;
        }
        committed.input(world, at, levels);
        assert_eq!(
            committed.drive(),
            self.core.drive(),
            "an input on the held solve never changes an image"
        );
        self.core.input(world, at, levels);
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
        let mut twis = Vec::new();
        let mut senders = Vec::new();
        let mut displays = Vec::new();
        let mut leds = Vec::new();
        let committed: Vec<Core> = circuit
            .actives
            .iter()
            .enumerate()
            .map(|(k, chip)| {
                let mut core = chip.core.clone();
                core.input(
                    circuit.netlist.world(),
                    Time::ZERO,
                    levels.of(circuit.active_watches(k)),
                );
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
                    twi: u16::try_from(twis.len()).expect("a run has few TWIs"),
                    phases: 0,
                    bytes: 0,
                    records: Time::ZERO,
                    sender: u16::try_from(senders.len()).expect("a run has few senders"),
                    display: u16::try_from(displays.len()).expect("a run has few displays"),
                };
                for &usart in chip.usarts {
                    usarts.push((name(circuit, chip), usart));
                }
                if chip.twi {
                    twis.push(name(circuit, chip));
                }
                if chip.sends {
                    senders.push(name(circuit, chip));
                }
                match core.shows() {
                    Shows::Screen { .. } => displays.push(name(circuit, chip)),
                    Shows::Leds { .. } => leds.push(name(circuit, chip)),
                    Shows::Nothing => {}
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
            staged: Staging::default(),
            usarts,
            twis,
            senders,
            displays,
            leds,
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

    pub(crate) fn twis(&self) -> &[Name] {
        &self.twis
    }

    pub(crate) fn senders(&self) -> &[Name] {
        &self.senders
    }

    pub(crate) fn cores(&self) -> &[Core] {
        &self.committed
    }

    pub(crate) fn displays(&self) -> &[Name] {
        &self.displays
    }

    pub(crate) fn led_names(&self) -> &[Name] {
        &self.leds
    }

    /// Each display part's committed core, in `displays` order.
    pub(crate) fn screens(&self) -> impl Iterator<Item = &dyn Hosted> {
        self.shown(|shows| matches!(shows, Shows::Screen { .. }))
    }

    /// Each chain's committed core, in `led_names` order.
    pub(crate) fn chains(&self) -> impl Iterator<Item = &dyn Hosted> {
        self.shown(|shows| matches!(shows, Shows::Leds { .. }))
    }

    /// Each committed device whose declared output passes `is`, in part order.
    fn shown(&self, is: impl Fn(Shows) -> bool) -> impl Iterator<Item = &dyn Hosted> {
        self.committed.iter().filter_map(move |core| match core {
            Core::Device(dev) if is(dev.shows()) => Some(&**dev),
            _ => None,
        })
    }

    /// The protocol and wiring warnings of the committed cores, each at its first time, and the
    /// unmodeled registers each copy touched and the restart it made by `through`, the accepted
    /// time.
    pub(crate) fn warnings(&self, circuit: &Circuit, through: Time) -> Vec<Diagnostic> {
        let mut found = Vec::new();
        for ((core, ahead), chip) in self.committed.iter().zip(&self.ahead).zip(&circuit.actives) {
            if let Core::Mcu(copy) = &ahead.core {
                let symbol = |pc| mcu::symbolize(model(circuit, chip), pc);
                for (at, note) in copy.notes(through) {
                    found.push(mcu::note(Ref::Part(name(circuit, chip)), at, note, symbol));
                }
            }
            match core {
                Core::Device(dev) => {
                    let part = || Ref::Part(name(circuit, chip));
                    for (rule, at) in dev.broke() {
                        found.push(Diagnostic::ChipProtocol {
                            part: part(),
                            at,
                            rule,
                        });
                    }
                    if let Some((at, sender)) = dev.miswired() {
                        found.push(Diagnostic::ChipLinkMiswired {
                            part: part(),
                            at,
                            sender: self.senders[usize::from(sender)].clone(),
                        });
                    }
                }
                Core::Mcu(_) => {}
                #[cfg(test)]
                Core::Script(_) => {}
            }
        }
        found
    }

    /// The first fault a committed core reached, which ends the run there, by time and then
    /// part order.
    pub(crate) fn fault(&self, circuit: &Circuit) -> Option<Diagnostic> {
        self.committed
            .iter()
            .zip(&circuit.actives)
            .filter_map(|(core, chip)| {
                let Core::Mcu(core) = core else {
                    return None;
                };
                let (at, fault) = core.fault()?;
                let symbol = |pc| mcu::symbolize(model(circuit, chip), pc);
                Some((
                    at,
                    mcu::fault(Ref::Part(name(circuit, chip)), at, fault, symbol),
                ))
            })
            .min_by_key(|&(at, _)| at)
            .map(|(_, fault)| fault)
    }

    /// Each core's record cursor: every record it sent that ended by then is staged.
    #[cfg(test)]
    pub(crate) fn record_cursors(&self) -> Vec<Time> {
        self.ahead
            .iter()
            .map(|ahead| ahead.delivered.records)
            .collect()
    }

    /// The copies each core runs ahead on. The next kept copy is cloned from these.
    #[cfg(test)]
    pub(crate) fn ahead_cores(&self) -> impl Iterator<Item = &Core> {
        self.ahead.iter().map(|ahead| &ahead.core)
    }

    /// The first stop of any core up to `limit`, or `limit`. A step from the accepted time to
    /// here solves with every image constant. Runs a core only past where it has reached.
    /// `limit` is never past the next change of a world quantity, so no run crosses one.
    pub(crate) fn horizon(&mut self, world: &World, limit: Time) -> Time {
        let mut horizon = limit;
        for ahead in &mut self.ahead {
            if let Reach::Done(reached) | Reach::Twin(reached) = ahead.reach
                && reached < limit
            {
                ahead.reach = match ahead.core.run_until(world, limit) {
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
    /// each core whose watches flipped takes them, and each core with a record pin on a record
    /// that ends at `at` takes it. So does each core whose device reads a world quantity that
    /// changes at `at`, so a run from its commit never crosses a change. Each such core, and
    /// each sender, is committed at `at`. With
    /// `keep`, so is every other core, and a later rewind runs again from `at` at the earliest.
    /// A copy that ran past `at`, as a retry shorter than its first try leaves it, runs again
    /// from its last commit to `at` first. Returns whether an image changed, and how many cores
    /// stopped at `at`.
    ///
    /// Two passes keep the landing free of part order. The first only reads: each sender stopped
    /// at `at` holds its records there, and they go to their receivers. The second is the only
    /// rewind and the only commit, so a receiver ahead of its sender in part order still takes
    /// the record, and the sender is still committed on its stop.
    pub(crate) fn land(
        &mut self,
        circuit: &Circuit,
        at: Time,
        x: &Solution,
        before: &Levels,
        after: &Levels,
        keep: bool,
    ) -> (bool, u64) {
        let world = circuit.netlist.world();
        let mut incoming = self.incoming(circuit, at);
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
            let records = incoming.get_mut(k).map(std::mem::take).unwrap_or_default();
            let event = Self::due_event(circuit, k, at);
            if stop.is_none()
                && !flipped
                && records.is_empty()
                && event.is_none()
                && !keep
                && !world.changes_at(committed.reads(), at)
            {
                continue;
            }
            if stop.is_some() {
                stops += 1;
            }
            // A sender stopped at `at` has reached `at`, so this leaves its copy and the
            // records the first pass read in place.
            ahead.rewind(world, committed, at);
            let image = ahead.core.drive().to_vec();
            changed |= image != committed.drive();
            if stop.is_some_and(|stop| stop.sample) {
                let volts: Vec<f64> = circuit.actives[k]
                    .samples
                    .iter()
                    .map(|signal| signal.at(x))
                    .collect();
                ahead.core.sample(world, at, &volts);
            }
            if flipped {
                ahead.core.input(world, at, levels);
            }
            if !records.is_empty() {
                ahead.core.take(world, at, &records);
            }
            if let Some(event) = event {
                ahead.core.event(world, at, event.event());
            }
            assert_eq!(
                ahead.core.drive(),
                image,
                "a sample, an input, a record or an event at a landing never changes an image"
            );
            ahead.commit(committed, &mut self.staged);
        }
        (changed, stops)
    }

    /// The run ends at `at`, where every stop up to `at` has landed. Each pulse target takes the
    /// part sent by then of each `Pulses` record still in flight on its line.
    pub(crate) fn cut(&mut self, circuit: &Circuit, at: Time) {
        let flying: Vec<Record> = self
            .committed
            .iter()
            .zip(&self.ahead)
            .flat_map(|(core, ahead)| match core {
                Core::Device(dev) => dev.in_flight(at).into_iter().collect(),
                Core::Mcu(chip) => match &ahead.core {
                    Core::Mcu(copy) => chip.in_flight(copy, at),
                    _ => unreachable!("a copy is of its core's kind"),
                },
                #[cfg(test)]
                Core::Script(_) => Vec::new(),
            })
            .filter(|record| matches!(record.kind, RecordKind::Pulses { .. }))
            .collect();
        for (core, chip) in self.committed.iter_mut().zip(&circuit.actives) {
            if chip.takes.pulses.is_some() {
                for record in flying.iter().filter(|record| chip.takes.record(record)) {
                    core.cut(circuit.netlist.world(), at, record);
                }
            }
        }
    }

    /// The event whose time is this landing, on the device in this slot.
    fn due_event(circuit: &Circuit, slot: usize, at: Time) -> Option<&Declared> {
        let part = circuit.actives[slot].part?;
        let (_, events) = circuit.netlist.part(part).model.device()?;
        events.at(at)
    }

    /// The records that end at `at`, read from each sender stopped there and listed for each
    /// chip that takes them, ordered by end, then by sender name, then by sequence. Reads only:
    /// no rewind, no commit, no cursor and no reach moves.
    fn incoming(&self, circuit: &Circuit, at: Time) -> Vec<Vec<LinkRow>> {
        if self.senders.is_empty() {
            return Vec::new();
        }
        let mut incoming = vec![Vec::new(); self.ahead.len()];
        for ahead in &self.ahead {
            let Reach::Stopped(stop) = ahead.reach else {
                continue;
            };
            if stop.at != at || !stop.record {
                debug_assert!(
                    stop.at != at || ahead.core.records().is_empty(),
                    "a core stopped without the record flag holds no records"
                );
                continue;
            }
            let after = ahead.delivered.records;
            for record in ahead.core.records() {
                debug_assert!(
                    record.end > after && record.end == at,
                    "a held record ends at its sender's stop"
                );
                for (list, chip) in incoming.iter_mut().zip(&circuit.actives) {
                    if chip.takes.record(record) {
                        list.push(LinkRow {
                            sender: ahead.delivered.sender,
                            record: record.clone(),
                        });
                    }
                }
            }
        }
        for list in &mut incoming {
            list.sort_by(|a, b| self.record_key(a).cmp(&self.record_key(b)));
        }
        incoming
    }

    fn record_key<'a>(&'a self, row: &LinkRow) -> (Time, &'a Name, u64) {
        let sender = &self.senders[usize::from(row.sender)];
        (row.record.end, sender, row.record.seq)
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
            ahead.input(
                circuit.netlist.world(),
                committed,
                at,
                levels,
                &mut self.staged,
            );
        }
    }

    /// Delivers each copy through `through`, which every copy has done everything up to. Returns
    /// every byte delivered since the last call, in the order stop bits ended, then by part
    /// name, then in its chip's USART order, then in the order its chip sent them. The TWI
    /// phases wait for `phases`, and the records for `records`.
    pub(crate) fn deliver(&mut self, through: Time) -> impl Iterator<Item = SerialByte> {
        for ahead in &mut self.ahead {
            assert!(ahead.has_done(through), "a copy delivers only what it did");
            ahead
                .delivered
                .deliver(&mut ahead.core, through, &mut self.staged);
        }
        let usarts = &self.usarts;
        let key = |staged: &Staged| {
            let usart = staged.byte.usart;
            let part = &usarts[usize::from(usart)].0;
            (staged.end, part, usart, staged.seq)
        };
        let bytes = &mut self.staged.bytes;
        bytes.sort_unstable_by(|a, b| key(a).cmp(&key(b)));
        bytes.drain(..).map(|staged| staged.byte)
    }

    /// Every TWI phase delivered since the last call, in the order phases ended, then in `twis`
    /// order.
    pub(crate) fn phases(&mut self) -> impl Iterator<Item = I2cRow> {
        let phases = &mut self.staged.phases;
        phases.sort_unstable_by_key(|staged| (staged.end, staged.row.twi));
        phases.drain(..).map(|staged| staged.row)
    }

    /// Every record staged since the last call, ordered by end, then by sender name, then by
    /// sequence.
    pub(crate) fn records(&mut self) -> Vec<LinkRow> {
        let mut records = std::mem::take(&mut self.staged.records);
        records.sort_by(|a, b| self.record_key(a).cmp(&self.record_key(b)));
        records
    }

    /// Every display row staged since the last call, ordered by time, then by part name, then
    /// in the order its part made them.
    pub(crate) fn rows(&mut self) -> Vec<Rectangle> {
        let mut rows = std::mem::take(&mut self.staged.rows);
        let displays = &self.displays;
        rows.sort_by(|a, b| {
            let key = |row: &Rectangle| (row.at, &displays[usize::from(row.part)]);
            key(a).cmp(&key(b))
        });
        rows
    }
}

/// The netlist chip of a microcontroller's part.
fn model<'c>(circuit: &'c Circuit, chip: &Chip) -> &'c crate::netlist::Chip {
    let part = chip.part.expect("a microcontroller is a part");
    match &circuit.netlist.part(part).model {
        crate::netlist::Model::Chip(model) => model,
        _ => unreachable!("a microcontroller's core belongs to a chip part"),
    }
}

/// The name of a chip's part. A test's script chip is no part and carries its own.
fn name(circuit: &Circuit, chip: &Chip) -> Name {
    match (chip.part, &chip.core) {
        (Some(part), _) => circuit.netlist.part(part).name.clone(),
        #[cfg(test)]
        (None, Core::Script(core)) => core.name(),
        (None, _) => unreachable!("only a test's script chip is no part"),
    }
}
