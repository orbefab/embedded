//! The active chip seam: a chip whose core runs on the circuit's clock. Its core sees the time,
//! names its own future events, holds state no stamp reads, and samples a voltage at a time it
//! picks. The circuit sees its drive image, one `Drive` per output pin, and a device's draw from
//! its supply. `docs/engine/engine.md`, Active chips, holds the loop's protocol.

use orbe_mcu::{Frame, Phase};
use orbe_model::air::{Ack, AirFrame, Channel, Signal as Heard};
use orbe_model::{Draw, Drawn, Drive, Event, Quantity, Shows, Stop, Time, View};

use crate::netlist::{NetId, PartId};

pub(crate) mod duty;
pub(crate) mod i2c;
pub(crate) mod native;
pub(crate) mod pulse;
pub(crate) mod replay;
mod route;
#[cfg(test)]
pub(crate) mod script;
pub(crate) mod spi;
#[cfg(test)]
pub(crate) mod spi_pins;

use crate::device::{
    bmp280, cst816, dht22, hc595, hd44780, hd44780_i2c, max7219, pulses_replay, sg90, spi_replay,
    ssd1306, st77916, ws2812,
};
use crate::mcu::{self, Mcu, atmega328p, esp32s3};
use native::Hosted;

use crate::device::Declared;
use crate::diagnostic::Diagnostic;
use crate::display::Rectangle;
use crate::i2c::I2cRow;
use crate::link::{Answers, LinkRow, Record, RecordKind};
use crate::name::{Name, Ref};

use crate::serial::SerialByte;

use crate::air::{self, Air, Net};
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
    /// Ohm, greater than 0. Alternate pull-up resistance when the core asserts `strong_pull_up`
    /// alongside [`Drive::PullUp`] (such as the ESP32-S3 USB PHY's D+ pull-up on GPIO20).
    pub(crate) strong_pull_up: Option<f64>,
    /// Ohm, greater than 0, to GND while its core pulls the pin down. `None` on a pin that never
    /// pulls down, where a pull-down its core reports is not stamped.
    pub(crate) pull_down: Option<f64>,
    /// Ohm, greater than 0, to VCC and again to GND in every drive state. `None` on a pin that
    /// leaks nothing.
    pub(crate) leakage: Option<f64>,
}

/// A chip's own draw: a conductance from its supply pin to its ground pin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Supply {
    pub(crate) vcc: NetId,
    pub(crate) gnd: NetId,
    /// Siemens, greater than 0, while it runs.
    pub(crate) conductance: f64,
    /// Siemens, greater than 0, while a reset holds it.
    pub(crate) held: f64,
}

impl Supply {
    /// The conductance in force for `core`.
    pub(crate) fn conductance(&self, core: &Core) -> f64 {
        if core.held() {
            self.held
        } else {
            self.conductance
        }
    }
}

/// An active chip as the circuit sees it. Read-only for the whole analysis.
#[derive(Clone, Debug)]
pub(crate) struct Chip {
    /// `None` only on the network and on a test's script chip, which no netlist part builds.
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
    /// Whether its core sends records, which go to `link.csv`. A chip that also takes `Pulses`
    /// records relays them: it sends on only what it took.
    pub(crate) sends: bool,
    /// Its own draw from its supply, one conductance while it runs and another while a reset
    /// holds it. `None` on a chip that draws nothing but what its outputs pass.
    pub(crate) supply: Option<Supply>,
    /// The supply and ground nets its device's [`Draw`] runs between. `None` on a chip whose
    /// core draws nothing that way.
    pub(crate) draw: Option<(NetId, NetId)>,
    /// The records it takes.
    pub(crate) takes: Takes,
    /// The core at power-on.
    pub(crate) core: Core,
}

impl Chip {
    /// Whether [`stamp`] reads the iterate: a device [`Draw`] does, and so does
    /// [`supply_released`] on a chip with outputs and a minimum supply.
    pub(crate) fn reads_iterate(&self) -> bool {
        self.draw.is_some() || (!self.outputs.is_empty() && self.core.min_supply_v().is_some())
    }
}

/// The nets of a chip's SPI target pins. It takes each `Spi` record whose CS and SCLK are on
/// these nets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpiTarget {
    pub(crate) cs: NetId,
    pub(crate) sclk: NetId,
    /// Package pin indices of `CS`, `SCLK`, and `D0` to `D3` on a netlist part. `None` on a
    /// test-only chip with no netlist part.
    pub(crate) pins: Option<[usize; 6]>,
}

/// The record pins of a chip: its SPI target's, and the pin that takes `Duty` records. A chip
/// with neither takes no record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Takes {
    pub(crate) spi: Option<SpiTarget>,
    pub(crate) duty: Option<NetId>,
    pub(crate) pulses: Option<NetId>,
    /// Package pin index of the `Pulses` input on a netlist part (`DIN`, `SIGNAL`).
    pub(crate) pulses_pin: Option<usize>,
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
/// - A core makes each display row at the time of the call that closed the write, and holds
///   it until `drain_rows` takes it. A core that takes records makes them only in `take`, at the
///   landing's time. A device on a bus, such as an I2C target, makes one in the call that closes
///   it, which may be an input on a twin, before the landing that commits it.
/// - `input`, `take`, `deliver`, `drain_records` and `drain_rows` read nothing but the core and
///   their arguments, so they take two equal cores to two equal cores. A clone shares only what
///   no call writes: the AVR core's flash and decoded ops behind an `Arc`, a replay's records,
///   a script core's script, and the network's scene data.
/// - A station holds the frame it put on the air from the stop where it ended until the landing
///   there hands it on. What `hear` and `acked` cause comes at least 1 ps later.
#[derive(Debug)]
pub(crate) enum Core {
    /// A device on the `orbe-model` contract.
    Device(Box<dyn Hosted>),
    /// A microcontroller on the `orbe-mcu` contract.
    Mcu(Mcu),
    /// The netlist's network, a station on the air with no part.
    Net(Box<Net>),
    #[cfg(test)]
    Script(Box<script::Core>),
}

impl Clone for Core {
    fn clone(&self) -> Self {
        match self {
            Core::Device(core) => Core::Device(core.boxed()),
            Core::Mcu(mcu) => Core::Mcu(mcu.clone()),
            Core::Net(net) => Core::Net(net.clone()),
            #[cfg(test)]
            Core::Script(core) => Core::Script(core.clone()),
        }
    }

    /// Reuses the buffers of a core of the same kind. Every landing copies each core it commits
    /// over that core's last commit. A copy of an `esp32s3` core starts with an empty table of
    /// decoded instructions, so a landing leaves the running core's table in place and a rewind
    /// starts that core empty.
    fn clone_from(&mut self, source: &Self) {
        match self {
            Core::Device(core) => match source {
                Core::Device(source) => core.copy_from(&**source),
                Core::Mcu(_) | Core::Net(_) => *self = source.clone(),
                #[cfg(test)]
                Core::Script(_) => *self = source.clone(),
            },
            Core::Mcu(mcu) => match source {
                Core::Mcu(source) => mcu.clone_from(source),
                Core::Device(_) | Core::Net(_) => *self = source.clone(),
                #[cfg(test)]
                Core::Script(_) => *self = source.clone(),
            },
            Core::Net(net) => match source {
                Core::Net(source) => net.clone_from(source),
                Core::Device(_) | Core::Mcu(_) => *self = source.clone(),
                #[cfg(test)]
                Core::Script(_) => *self = source.clone(),
            },
            #[cfg(test)]
            Core::Script(_) => *self = source.clone(),
        }
    }
}

impl Core {
    fn run_until(&mut self, world: &World, limit: Time) -> Option<Stop> {
        match *self {
            Core::Device(ref mut core) => core.run_until(world, limit),
            Core::Mcu(ref mut mcu) => mcu.run_until(limit),
            Core::Net(ref mut net) => net.run_until(limit),
            #[cfg(test)]
            Core::Script(ref mut core) => core.run_until(limit),
        }
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        match *self {
            Core::Device(ref core) => core.drive(),
            Core::Mcu(ref mcu) => mcu.drive(),
            Core::Net(_) => &[],
            #[cfg(test)]
            Core::Script(ref core) => core.drive(),
        }
    }

    /// [`orbe_model::Device::MIN_SUPPLY_V`], or `None` for a core that drives at any supply.
    pub(crate) fn min_supply_v(&self) -> Option<f64> {
        match *self {
            Core::Device(ref core) => core.min_supply_v(),
            Core::Mcu(_) | Core::Net(_) => None,
            #[cfg(test)]
            Core::Script(_) => None,
        }
    }

    /// Whether the host samples it at every accepted step.
    pub(crate) fn tracks_supply(&self) -> bool {
        match self {
            Core::Device(core) => core.tracks_supply(),
            Core::Mcu(_) | Core::Net(_) => false,
            #[cfg(test)]
            Core::Script(_) => false,
        }
    }

    /// The shaft angle in millidegrees at `at`, when the device shows one.
    pub(crate) fn angle_at(&self, at: Time) -> Option<i32> {
        match self {
            Core::Device(core) => core.angle_at(at),
            Core::Mcu(_) | Core::Net(_) => None,
            #[cfg(test)]
            Core::Script(_) => None,
        }
    }

    /// What its device draws from its supply, as at its last stop. Only a device draws so.
    pub(crate) fn draw(&self) -> Draw {
        match *self {
            Core::Device(ref core) => core.draw(),
            Core::Mcu(_) | Core::Net(_) => Draw::NONE,
            #[cfg(test)]
            Core::Script(_) => Draw::NONE,
        }
    }

    /// Whether each output pulls down, in the order of its image, or empty when none does. Only a
    /// microcontroller says so, and only at a stop.
    pub(crate) fn pulled_down(&self) -> &[bool] {
        match *self {
            Core::Mcu(ref mcu) => mcu.pulled_down(),
            Core::Device(_) | Core::Net(_) => &[],
            #[cfg(test)]
            Core::Script(ref core) => core.pulled_down(),
        }
    }

    /// Whether output `output`'s pin pulls down to its GND, which its `Output` stamps only when
    /// it has a pull-down.
    pub(crate) fn pulls_down(&self, output: usize) -> bool {
        self.pulled_down().get(output).copied().unwrap_or(false)
    }

    /// Whether each `Drive::PullUp` output uses its strong pull-up resistor, or empty when none
    /// does. Only a microcontroller says so, and only at a stop.
    pub(crate) fn strong_pull_up(&self) -> &[bool] {
        match *self {
            Core::Mcu(ref mcu) => mcu.strong_pull_up(),
            Core::Device(_) | Core::Net(_) => &[],
            #[cfg(test)]
            Core::Script(_) => &[],
        }
    }

    /// Whether output `output`'s [`Drive::PullUp`] stamps `Output::strong_pull_up`.
    pub(crate) fn pulls_up_strong(&self, output: usize) -> bool {
        self.strong_pull_up().get(output).copied().unwrap_or(false)
    }

    /// Whether a reset holds it, which changes its own draw as a drive change does. Only a
    /// microcontroller says so, and only at a stop.
    pub(crate) fn held(&self) -> bool {
        match *self {
            Core::Mcu(ref mcu) => mcu.held(),
            Core::Device(_) | Core::Net(_) => false,
            #[cfg(test)]
            Core::Script(_) => false,
        }
    }

    /// When its firmware first read output `output`'s pin as a digital input since the pin's
    /// drive last turned off, as far as it has run. Only a microcontroller says so.
    fn read(&self, output: usize) -> Option<Time> {
        match *self {
            Core::Mcu(ref mcu) => mcu.read(output),
            Core::Device(_) | Core::Net(_) => None,
            #[cfg(test)]
            Core::Script(_) => None,
        }
    }

    /// The volts of each of its samples at its stop at `at`.
    fn sample(&mut self, world: &World, at: Time, volts: &[f64]) {
        match *self {
            Core::Device(ref mut core) => core.sample(world, at, volts),
            Core::Mcu(ref mut mcu) => mcu.sample(world, at, volts),
            Core::Net(_) => unreachable!("the network samples nothing"),
            #[cfg(test)]
            Core::Script(ref mut core) => core.sample(at, volts),
        }
    }

    /// Its watches' levels from `at` on.
    fn input(&mut self, world: &World, at: Time, levels: &[bool]) {
        match *self {
            Core::Device(ref mut core) => core.input(world, at, levels),
            Core::Mcu(ref mut mcu) => mcu.input(at, levels),
            Core::Net(_) => debug_assert!(levels.is_empty(), "the network watches nothing"),
            #[cfg(test)]
            Core::Script(ref mut core) => core.input(at, levels),
        }
    }

    /// Takes each byte a USART sent whose stop bit ended at or before `through`, in the order
    /// they ended on each USART, and each TWI phase that ended by then, in the order they
    /// ended. Drops every SPI byte, which no output reads. The network instead hands over and
    /// drops each row it made by then, in the order it made them.
    fn deliver(
        &mut self,
        through: Time,
        sent: impl FnMut(Frame),
        saw: impl FnMut(Phase),
        made: impl FnMut(orbe_net::Row),
    ) {
        match *self {
            Core::Mcu(ref mut mcu) => mcu.deliver(through, sent, saw),
            Core::Net(ref mut net) => net.drain_rows(through, made),
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
            Core::Net(_) => &[],
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
            Core::Net(_) => {}
            #[cfg(test)]
            Core::Script(ref mut core) => core.drain_records(through, sent),
        }
    }

    /// Takes the records that end at `at` on its record pins, ordered by end, then by sender name,
    /// then by sequence.
    fn take(&mut self, world: &World, at: Time, records: &[LinkRow], answers: &mut Answers) {
        match *self {
            Core::Device(ref mut core) => core.take(world, at, records, answers),
            Core::Mcu(_) | Core::Net(_) => {
                unreachable!(
                    "a chip with no record pins took {} records at {at:?}",
                    records.len()
                )
            }
            #[cfg(test)]
            Core::Script(ref mut core) => core.take(at, records),
        }
    }

    /// The bytes the targets answered on MISO to the SPI record it sent that ended at `at`.
    fn answered(&mut self, at: Time, miso: &[u8]) {
        match *self {
            Core::Mcu(ref mut mcu) => mcu.answered(at, miso),
            Core::Device(_) => unreachable!("a device sends no record that reads"),
            Core::Net(_) => unreachable!("the network sends no record"),
            #[cfg(test)]
            Core::Script(_) => unreachable!("a script sends no record that reads"),
        }
    }

    /// Hands over and drops each display row it made, with its time, in the order it made them.
    fn drain_rows(&mut self, mut sent: impl FnMut(Time, Drawn)) {
        match *self {
            Core::Device(ref mut core) => core.drain_rows(&mut sent),
            Core::Mcu(_) | Core::Net(_) => {}
            #[cfg(test)]
            Core::Script(_) => {}
        }
    }

    /// The part of one `Pulses` record sent by `at`, where the run ends inside it, and the
    /// part sent by `at` of the record it relays on from it.
    fn cut(&mut self, world: &World, at: Time, record: &Record) -> Option<Record> {
        match self {
            Core::Device(core) => core.cut(world, at, record),
            Core::Mcu(_) | Core::Net(_) => None,
            #[cfg(test)]
            Core::Script(_) => None,
        }
    }

    /// Keeps every change of what an LED chain shows, for a run that writes them.
    fn keep_shown(&mut self) {
        if let Core::Device(core) = self {
            core.keep_shown();
        }
    }

    /// What it shows besides its drives, as its device declares.
    fn shows(&self) -> Shows {
        match self {
            Core::Device(core) => core.shows(),
            Core::Mcu(_) | Core::Net(_) => Shows::Nothing,
            #[cfg(test)]
            Core::Script(_) => Shows::Nothing,
        }
    }

    /// What the device shows now. A chip shows nothing.
    fn view(&self) -> View<'_> {
        match self {
            Core::Device(core) => core.view(),
            Core::Mcu(_) | Core::Net(_) => View::Nothing,
            #[cfg(test)]
            Core::Script(_) => View::Nothing,
        }
    }

    /// The world quantities its device reads. A chip reads none.
    fn reads(&self) -> &'static [Quantity] {
        match self {
            Core::Device(core) => core.reads(),
            Core::Mcu(_) | Core::Net(_) => &[],
            #[cfg(test)]
            Core::Script(_) => &[],
        }
    }

    /// An event its device declares, at `at`. The image stays as it is.
    fn event(&mut self, world: &World, at: Time, event: Event<'_>) {
        match self {
            Core::Device(core) => core.event(world, at, event),
            Core::Mcu(_) | Core::Net(_) => unreachable!("only a device takes an event"),
            #[cfg(test)]
            Core::Script(_) => unreachable!("only a device takes an event"),
        }
    }

    /// Whether it is a station on the air: the network, and a chip with a radio, such as an
    /// ESP32-S3 built with its radio.
    pub(crate) fn radio(&self) -> bool {
        match self {
            Core::Net(_) => true,
            Core::Mcu(mcu) => mcu.radio(),
            Core::Device(_) => false,
            #[cfg(test)]
            Core::Script(core) => core.radio(),
        }
    }

    /// The channel a station listens on at its time, or `None` while its RF is off.
    fn tuned(&self) -> Option<Channel> {
        match self {
            Core::Net(net) => net.tuned(),
            Core::Mcu(mcu) => mcu.tuned(),
            Core::Device(_) => unreachable!("only a station listens"),
            #[cfg(test)]
            Core::Script(core) => core.tuned(),
        }
    }

    /// The frame a station put on the air that ended at its stop, until the landing there.
    fn aired(&self) -> Option<&AirFrame> {
        match self {
            Core::Net(net) => net.aired(),
            Core::Mcu(mcu) => mcu.aired(),
            Core::Device(_) => unreachable!("only a station airs"),
            #[cfg(test)]
            Core::Script(core) => core.aired(),
        }
    }

    /// Drops the frame its landing handed on.
    fn drop_aired(&mut self) {
        match self {
            Core::Net(net) => net.drop_aired(),
            Core::Mcu(mcu) => mcu.drop_aired(),
            Core::Device(_) => unreachable!("only a station airs"),
            #[cfg(test)]
            Core::Script(core) => core.drop_aired(),
        }
    }

    /// Another station's frame on its channel, which ended at `at`, and whether it acknowledges
    /// it.
    fn hear(&mut self, at: Time, frame: &AirFrame, signal: Heard) -> Ack {
        match self {
            Core::Net(net) => net.hear(at, frame, signal),
            Core::Mcu(mcu) => mcu.hear(at, frame, signal),
            Core::Device(_) => unreachable!("only a station hears"),
            #[cfg(test)]
            Core::Script(core) => core.hear(at, frame),
        }
    }

    /// Whether any station acknowledged its frame that ended at `at`.
    fn acked(&mut self, at: Time, ack: Ack) {
        match self {
            Core::Net(net) => net.acked(at, ack),
            Core::Mcu(mcu) => mcu.acked(at, ack),
            Core::Device(_) => unreachable!("only a station airs"),
            #[cfg(test)]
            Core::Script(core) => core.acked(at, ack),
        }
    }
}

/// The chip of `kind`. With `radio`, the netlist has a network, and a kind with a radio is built
/// with it.
pub(crate) fn chip(part: PartId, kind: Active<'_>, radio: bool) -> Chip {
    match kind {
        Active::Atmega328p(mcu) => atmega328p::chip(part, mcu),
        Active::SpiReplay(replay) => spi_replay::chip(part, replay),
        Active::PulsesReplay(replay) => pulses_replay::chip(part, replay),
        Active::St77916(panel) => st77916::chip(part, panel),
        Active::Ssd1306(panel) => ssd1306::chip(part, panel),
        Active::Hd44780I2c(panel) => hd44780_i2c::chip(part, panel),
        Active::Hd44780(panel) => hd44780::chip(part, panel),
        Active::Cst816(dev) => cst816::chip(part, dev),
        Active::Dht22(dev) => dht22::chip(part, dev),
        Active::Bmp280(dev) => bmp280::chip(part, dev),
        Active::Hc595(dev) => hc595::chip(part, dev),
        Active::Max7219(dev) => max7219::chip(part, dev),
        Active::Esp32s3(mcu) => esp32s3::chip(part, mcu, radio),
        Active::Ws2812(chain) => ws2812::chip(part, chain),
        Active::Sg90(dev) => sg90::chip(part, dev),
    }
}

/// Margin inside a device's minimum supply so an exact `min_supply_v` rail does not release
/// outputs on 1-ULP solver rounding.
const SUPPLY_BOUND_INSIDE_V: f64 = 1e-6;

/// Whether `vcc` over `gnd` is below the supply at which `core` still drives.
pub(crate) fn supply_released(core: &Core, vcc: f64, gnd: f64) -> bool {
    core.min_supply_v().is_some_and(|min| {
        let margin = SUPPLY_BOUND_INSIDE_V.min(0.25 * min);
        vcc - gnd < min - margin
    })
}

/// Whether any output's [`supply_released`] state differs between `before` (the iterate used to
/// stamp) and `after` (the solve's solution).
pub(crate) fn supply_flipped(
    chip: &Chip,
    core: &Core,
    before: &Solution,
    after: &Solution,
) -> bool {
    if core.min_supply_v().is_none() {
        return false;
    }
    chip.outputs.iter().any(|output| {
        supply_released(core, before.voltage(output.vcc), before.voltage(output.gnd))
            != supply_released(core, after.voltage(output.vcc), after.voltage(output.gnd))
    })
}

/// The drops are sources, so they scale. A device's draw is linearised at `x`, the iterate:
/// its current there, and its slope as a conductance. A device below its minimum supply
/// stamps no stage: its [`Core::drive`] image stays, and the pins are released.
pub(crate) fn stamp(chip: &Chip, core: &Core, sys: &mut System, x: &Solution, source_scale: f64) {
    let drive = core.drive();
    debug_assert_eq!(drive.len(), chip.outputs.len());
    if let Some(supply) = chip.supply {
        sys.conductance(supply.vcc, supply.gnd, supply.conductance(core));
    }
    if let Some((vcc, gnd)) = chip.draw {
        let volts = x.voltage(vcc) - x.voltage(gnd);
        let (amps, slope) = core.draw().at(volts);
        sys.conductance(vcc, gnd, slope);
        sys.current(vcc, gnd, amps - slope * volts);
    }
    let pulled_down = core.pulled_down();
    debug_assert!(pulled_down.is_empty() || pulled_down.len() == chip.outputs.len());
    for (n, (output, drive)) in chip.outputs.iter().zip(drive).enumerate() {
        if let Some(ohm) = output.leakage {
            sys.conductance(output.pin, output.vcc, 1.0 / ohm);
            sys.conductance(output.pin, output.gnd, 1.0 / ohm);
        }
        if supply_released(core, x.voltage(output.vcc), x.voltage(output.gnd)) {
            continue;
        }
        if let Some(ohm) = output.pull_down
            && pulled_down.get(n) == Some(&true)
        {
            debug_assert!(
                matches!(drive, Drive::PullUp | Drive::Off),
                "only a pin that does not drive pulls down"
            );
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
                let ohm = if core.pulls_up_strong(n) {
                    output
                        .strong_pull_up
                        .expect("a strong pull-up drive on a pin that has one")
                } else {
                    output
                        .pull_up
                        .expect("a pull-up drive on a pin that has one")
                };
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
    /// Whether a chip sends records of its own, not only relays the `Pulses` records it takes.
    /// A relay sends nothing unless such a chip sent first.
    links: bool,
    /// The name of each display part, in part order. A `Rectangle`'s `part` indexes this.
    displays: Vec<Name>,
    /// The name of each LED chain, in part order. A latch's `part` indexes this.
    leds: Vec<Name>,
    /// The name of each shaft, in part order.
    horns: Vec<Name>,
    /// Precomputed net incidence and targets for checking `Pulses` and `Spi` records at landing.
    routing: route::Routing,
    /// The first record-routing fault reached, which ends the run at its landing.
    routing_fault: Option<(Time, Diagnostic)>,
    /// The stations on the air, or `None` when no core is one.
    air: Option<Air>,
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
    /// The network's rows, in the order it made them.
    net: Vec<orbe_net::Row>,
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
/// A display row is made at or before the landing that commits it, and that commit stages it
/// with its own time. So the committed core holds no row.
///
/// The network makes its rows as it runs, and is delivered as a USART is: a row is staged only
/// when its time is in `(through, to]`. Its run makes the same rows again after a rewind, and
/// one at or before `through` was staged from the copy that made it first. A row made at a
/// landing has the landing's time, which is past every delivery.
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
        let Staging {
            bytes, phases, net, ..
        } = staged;
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
            |row| {
                if row.at > through {
                    net.push(row);
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

    /// Stages each display row the copy made, each at its own time, at or before `at`, and drops
    /// it from the copy. Only `Ahead::commit` calls this.
    fn take_rows(&self, core: &mut Core, at: Time, staged: &mut Staging) {
        let part = self.display;
        core.drain_rows(|made, row| {
            debug_assert!(made <= at, "a row is made by the landing that commits it");
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
                (
                    self.core.drive(),
                    self.core.pulled_down(),
                    self.core.strong_pull_up(),
                ),
                (
                    committed.drive(),
                    committed.pulled_down(),
                    committed.strong_pull_up(),
                ),
                "an input on the held solve never changes an image"
            );
            self.commit(committed, staged);
            return;
        }
        committed.input(world, at, levels);
        assert_eq!(
            (
                committed.drive(),
                committed.pulled_down(),
                committed.strong_pull_up(),
            ),
            (
                self.core.drive(),
                self.core.pulled_down(),
                self.core.strong_pull_up(),
            ),
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
        let mut links = false;
        let mut displays = Vec::new();
        let mut leds = Vec::new();
        let mut horns = Vec::new();
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
                    (core.drive(), core.pulled_down(), core.strong_pull_up()),
                    (
                        chip.core.drive(),
                        chip.core.pulled_down(),
                        chip.core.strong_pull_up(),
                    ),
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
                    links |= chip.takes.pulses.is_none();
                }
                match core.shows() {
                    Shows::Screen { .. } => displays.push(name(circuit, chip)),
                    Shows::Leds { .. } => leds.push(name(circuit, chip)),
                    Shows::Angle => horns.push(name(circuit, chip)),
                    Shows::Nothing => {}
                }
                Ahead {
                    core: core.clone(),
                    reach: Reach::Done(Time::ZERO),
                    delivered,
                }
            })
            .collect();
        let routing = route::Routing::new(circuit, !senders.is_empty());
        let air = Air::new(&committed);
        Actives {
            committed,
            ahead,
            staged: Staging::default(),
            usarts,
            twis,
            senders,
            links,
            displays,
            leds,
            horns,
            routing,
            routing_fault: None,
            air,
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

    /// Whether a chip sends records of its own, so the run can send any.
    pub(crate) fn links(&self) -> bool {
        self.links
    }

    pub(crate) fn cores(&self) -> &[Core] {
        &self.committed
    }

    /// When active chip `k`'s firmware first read output `output`'s pin since its drive last
    /// turned off. It asks the copy run ahead, which is never behind the accepted time: a
    /// landing commits only a core that stops there, so the committed core of a chip that
    /// polls a pin without changing a drive can sit far back. The copy is past no change of
    /// its image but at most the one at its stop, so the stretch it names is the one that
    /// holds the accepted time whenever the committed drive there is off.
    pub(crate) fn read(&self, k: usize, output: usize) -> Option<Time> {
        self.ahead[k].core.read(output)
    }

    pub(crate) fn displays(&self) -> &[Name] {
        &self.displays
    }

    pub(crate) fn led_names(&self) -> &[Name] {
        &self.leds
    }

    /// The shaft angle of horn `name`, in degrees, at `at`.
    pub(crate) fn horn_degrees(&self, name: &Name, at: Time) -> Option<f64> {
        let k = self.horns.iter().position(|horn| horn == name)?;
        self.committed
            .iter()
            .filter_map(|core| {
                if !matches!(core.shows(), Shows::Angle) {
                    return None;
                }
                let mdeg = core.angle_at(at).unwrap_or_else(|| {
                    let View::Degrees(mdeg) = core.view() else {
                        panic!("a shaft shows its angle in millidegrees");
                    };
                    mdeg
                });
                Some(mdeg as f64 / 1_000.0)
            })
            .nth(k)
    }

    /// Has every LED chain keep each latch, for a run that writes `leds.csv`. A session that
    /// writes nothing keeps only each chain's last latch.
    pub(crate) fn keep_shown(&mut self) {
        for core in self.committed.iter_mut() {
            core.keep_shown();
        }
        for ahead in &mut self.ahead {
            ahead.core.keep_shown();
        }
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
            Core::Device(_) | Core::Mcu(_) | Core::Net(_) => None,
            #[cfg(test)]
            Core::Script(_) => None,
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
                Core::Mcu(_) | Core::Net(_) => {}
                #[cfg(test)]
                Core::Script(_) => {}
            }
        }
        found
    }

    /// The first fault a committed core or a record landing reached, which ends the run there, by
    /// time and then part order.
    pub(crate) fn fault(&self, circuit: &Circuit) -> Option<Diagnostic> {
        let core = self
            .committed
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
            .min_by_key(|&(at, _)| at);
        match (core, self.routing_fault.clone()) {
            (Some((a, fa)), Some((b, fb))) => Some(if a <= b { fa } else { fb }),
            (Some((_, f)), None) | (None, Some((_, f))) => Some(f),
            (None, None) => None,
        }
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
    /// the record, and the sender is still committed on its stop. A receiver answers each byte
    /// an SPI record reads as it takes it. After the second pass each sender of such a record
    /// takes the answer, on its kept core and on the copy that twins it, so the answer lands at
    /// the record's end whatever the part order.
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
        let (mut incoming, mut answers, routing_fault) = self.incoming(circuit, at);
        if let Some(fault) = routing_fault {
            self.routing_fault.get_or_insert((at, fault));
        }
        let on_air = self.air.is_some() && self.air(world, at);
        let mut changed = false;
        let mut stops = 0u64;
        for (k, (committed, ahead)) in self.committed.iter_mut().zip(&mut self.ahead).enumerate() {
            let heard = on_air && self.air.as_ref().is_some_and(|air| air.is_station(k));
            let watched = circuit.active_watches(k);
            let levels = after.of(watched.clone());
            let flipped = before.of(watched) != levels;
            let stop = match ahead.reach {
                Reach::Stopped(stop) if stop.at == at => Some(stop),
                _ => None,
            };
            let records = incoming.get_mut(k).map(std::mem::take).unwrap_or_default();
            let event = Self::due_event(circuit, k, at);
            let tracks = committed.tracks_supply();
            if stop.is_none()
                && !flipped
                && records.is_empty()
                && event.is_none()
                && !heard
                && !keep
                && !tracks
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
            let pulled_down = ahead.core.pulled_down().to_vec();
            let strong_pull_up = ahead.core.strong_pull_up().to_vec();
            let draw = ahead.core.draw();
            changed |= image != committed.drive()
                || pulled_down != committed.pulled_down()
                || strong_pull_up != committed.strong_pull_up()
                || (circuit.actives[k]
                    .supply
                    .is_some_and(|s| s.conductance != s.held)
                    && ahead.core.held() != committed.held())
                || draw != committed.draw();
            if stop.is_some_and(|stop| stop.sample) || tracks {
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
                ahead.core.take(world, at, &records, &mut answers);
            }
            if let Some(event) = event {
                ahead.core.event(world, at, event.event());
            }
            assert_eq!(
                (
                    ahead.core.drive(),
                    ahead.core.pulled_down(),
                    ahead.core.strong_pull_up(),
                    ahead.core.draw()
                ),
                (&image[..], &pulled_down[..], &strong_pull_up[..], draw),
                "a sample, an input, a record or an event at a landing never changes an image"
            );
            ahead.commit(committed, &mut self.staged);
        }
        for answer in answers.into_vec() {
            let (committed, ahead) = (
                &mut self.committed[answer.slot],
                &mut self.ahead[answer.slot],
            );
            assert!(
                matches!(ahead.reach, Reach::Twin(twin) if twin == at),
                "a sender is kept at its record's end"
            );
            committed.answered(at, &answer.miso);
            ahead.core.answered(at, &answer.miso);
            assert_eq!(
                (
                    committed.drive(),
                    committed.pulled_down(),
                    committed.strong_pull_up(),
                ),
                (
                    ahead.core.drive(),
                    ahead.core.pulled_down(),
                    ahead.core.strong_pull_up(),
                ),
                "an answer never changes an image"
            );
        }
        (changed, stops)
    }

    /// The air's part of a landing at `at`, before the second pass. Each frame a station's copy
    /// stopped on there goes to every other station tuned to its channel, in station order,
    /// frames in station order too. Each station's copy is put at `at` first, and each sender's
    /// copy takes the OR of the answers before it runs on. Returns whether a frame landed, so
    /// the second pass keeps every station at `at`. Never inlined, so a landing with no air
    /// runs the code it ran before the air.
    #[inline(never)]
    fn air(&mut self, world: &World, at: Time) -> bool {
        let Some(air) = &self.air else {
            return false;
        };
        let frames: Vec<(usize, AirFrame)> = (air.stations().iter())
            .filter_map(|&slot| {
                let ahead = &self.ahead[slot];
                match ahead.reach {
                    Reach::Stopped(stop) if stop.at == at && stop.record => {
                        ahead.core.aired().map(|frame| (slot, frame.clone()))
                    }
                    _ => None,
                }
            })
            .collect();
        if frames.is_empty() {
            return false;
        }
        let mut acks = vec![Ack::None; frames.len()];
        for &slot in air.stations() {
            let ahead = &mut self.ahead[slot];
            // A copy stopped at `at` is there, and keeps its stop for the second pass.
            if !matches!(ahead.reach, Reach::Stopped(stop) if stop.at == at) {
                ahead.rewind(world, &self.committed[slot], at);
            }
            let Some(channel) = ahead.core.tuned() else {
                continue;
            };
            let image = ahead.core.drive().to_vec();
            for ((sender, frame), ack) in frames.iter().zip(&mut acks) {
                if *sender != slot
                    && frame.channel == channel
                    && ahead.core.hear(at, frame, air::SIGNAL) == Ack::Acked
                {
                    *ack = Ack::Acked;
                }
            }
            assert_eq!(ahead.core.drive(), image, "a frame never changes an image");
        }
        #[cfg(test)]
        if let Some(lose) = air.lose_ack {
            for ((_, frame), ack) in frames.iter().zip(&mut acks) {
                if lose(frame) {
                    *ack = Ack::None;
                }
            }
        }
        for ((sender, _), ack) in frames.iter().zip(acks) {
            let core = &mut self.ahead[*sender].core;
            core.drop_aired();
            core.acked(at, ack);
        }
        #[cfg(test)]
        if let Some(air) = &mut self.air {
            air.log.extend(frames);
        }
        true
    }

    /// Loses the ACK of each frame `lose` picks, from the next landing on.
    #[cfg(test)]
    pub(crate) fn lose_acks(&mut self, lose: fn(&AirFrame) -> bool) {
        if let Some(air) = &mut self.air {
            air.lose_ack = Some(lose);
        }
    }

    /// Each frame a landing handed on, with its sender's slot.
    #[cfg(test)]
    pub(crate) fn aired(&self) -> &[(usize, AirFrame)] {
        self.air.as_ref().map_or(&[], |air| &air.log)
    }

    /// The run ends at `at`, where every stop up to `at` has landed. Each pulse target takes the
    /// part sent by then of each `Pulses` record still in flight on its line. A target that
    /// relays what it takes hands the part it relayed by then on to the next, until a part
    /// relays nothing sent by `at`.
    pub(crate) fn cut(&mut self, circuit: &Circuit, at: Time) {
        let mut flying: Vec<Record> = self
            .committed
            .iter()
            .zip(&self.ahead)
            .flat_map(|(core, ahead)| match core {
                Core::Device(dev) => dev.in_flight(at),
                Core::Mcu(chip) => match &ahead.core {
                    Core::Mcu(copy) => chip.in_flight(copy, at),
                    Core::Device(_) | Core::Net(_) => {
                        unreachable!("a copy is of its core's kind")
                    }
                    #[cfg(test)]
                    Core::Script(_) => unreachable!("a copy is of its core's kind"),
                },
                Core::Net(_) => Vec::new(),
                #[cfg(test)]
                Core::Script(_) => Vec::new(),
            })
            .filter(|record| matches!(record.kind, RecordKind::Pulses { .. }))
            .collect();
        while !flying.is_empty() {
            let mut relayed = Vec::new();
            for (core, chip) in self.committed.iter_mut().zip(&circuit.actives) {
                if chip.takes.pulses.is_some() {
                    for record in flying.iter().filter(|record| chip.takes.record(record)) {
                        relayed.extend(core.cut(circuit.netlist.world(), at, record));
                    }
                }
            }
            flying = relayed;
        }
    }

    /// The event whose time is this landing, on the device in this slot.
    fn due_event(circuit: &Circuit, slot: usize, at: Time) -> Option<&Declared> {
        let part = circuit.actives[slot].part?;
        let (_, events) = circuit.netlist.part(part).model.device()?;
        events.at(at)
    }

    /// The records that end at `at`, read from each sender stopped there and listed for each
    /// chip that takes them, ordered by end, then by sender name, then by sequence, and an open
    /// answer for each SPI record that reads, plus the first routing fault if any record cannot
    /// reach its net faithfully. Reads only: no rewind, no commit, no cursor and no reach moves.
    fn incoming(
        &self,
        circuit: &Circuit,
        at: Time,
    ) -> (Vec<Vec<LinkRow>>, Answers, Option<Diagnostic>) {
        let mut answers = Answers::default();
        if self.senders.is_empty() {
            return (Vec::new(), answers, None);
        }
        let mut routing_fault: Option<((&Name, u64), Diagnostic)> = None;
        let mut incoming = vec![Vec::new(); self.ahead.len()];
        for (slot, ahead) in self.ahead.iter().enumerate() {
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
            // `record` is set on a frame's stop too. Only a sender has a slot in `senders`, and
            // only a sender holds records.
            if !circuit.actives[slot].sends {
                debug_assert!(
                    ahead.core.records().is_empty(),
                    "a core that sends no records holds none"
                );
                continue;
            }
            let after = ahead.delivered.records;
            let sender_part = circuit.actives[slot].part;
            let sender_name = &self.senders[usize::from(ahead.delivered.sender)];
            for record in ahead.core.records() {
                debug_assert!(
                    record.end > after && record.end == at,
                    "a held record ends at its sender's stop"
                );
                if let Some(fault) = self
                    .routing
                    .check(&circuit.netlist, sender_part, record, at)
                {
                    let key = (sender_name, record.seq);
                    if routing_fault.as_ref().is_none_or(|(best, _)| key < *best) {
                        routing_fault = Some((key, fault));
                    }
                    continue;
                }
                if let RecordKind::Spi { spi, .. } = &record.kind
                    && spi.reads() > 0
                {
                    answers.open(slot, ahead.delivered.sender, record.seq, spi.reads());
                }
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
        if let Some((_, fault)) = routing_fault {
            return (Vec::new(), Answers::default(), Some(fault));
        }
        for list in &mut incoming {
            list.sort_by(|a, b| self.record_key(a).cmp(&self.record_key(b)));
        }
        (incoming, answers, None)
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

    /// Every network row staged since the last call, in the order the network made them.
    pub(crate) fn net_rows(&mut self) -> Vec<orbe_net::Row> {
        std::mem::take(&mut self.staged.net)
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
    use crate::netlist::Model;
    let part = chip.part.expect("a microcontroller is a part");
    match &circuit.netlist.part(part).model {
        Model::Chip(model) => model,
        Model::Resistor(_)
        | Model::Potentiometer(_)
        | Model::VoltageSource(_)
        | Model::CurrentSource(_)
        | Model::Diode(_)
        | Model::Led(_)
        | Model::Zener(_)
        | Model::LinearRegulator(_)
        | Model::Npn(_)
        | Model::Pnp(_)
        | Model::Nmos(_)
        | Model::Pmos(_)
        | Model::Capacitor(_)
        | Model::Inductor(_)
        | Model::Relay(_)
        | Model::Switch(_)
        | Model::OpAmp(_)
        | Model::AnalogTempSensor(_) => {
            unreachable!("a microcontroller's core belongs to a chip part")
        }
    }
}

/// The name of a chip's part. A test's script chip is no part and carries its own.
fn name(circuit: &Circuit, chip: &Chip) -> Name {
    match chip.part {
        Some(part) => circuit.netlist.part(part).name.clone(),
        None => match &chip.core {
            #[cfg(test)]
            Core::Script(core) => core.name(),
            Core::Device(_) | Core::Mcu(_) | Core::Net(_) => {
                unreachable!("only a test's script chip and the network are no part")
            }
        },
    }
}
