//! Behavioural models of packaged parts, and the questions the loop asks a `Chip`.
//!
//! A file in this directory is one packaged part. `ne555` is a `Chip`. `relay`, `regulator` and
//! `op_amp` are their own `Model` variants, so none adds a `Passive` variant.
//!
//! This file is the dispatch. `kind` says whether a netlist `Chip` is passive or active, and
//! every `Chip` answers `unknowns` and `dc_paths` here. A passive `Chip` adds a `Passive`
//! variant, a `chip/<kind>.rs`, an arm in `passive`, and an arm in each function that takes
//! `Passive`. An active kind adds an `Active` variant and its arm in `active::chip`. A device in
//! `device/` builds an `active::Core::Device` and adds its row to `Model::device`. A
//! microcontroller builds an `active::Core::Mcu` and adds its row to the table in `mcu/mod.rs`.
//! The relay answers `unknowns` and `dc_paths` as an element.

use crate::device::{
    Bmp280, Cst816, Dht22, Hc595, Hd44780, Hd44780I2c, Max7219, PulsesReplay, Sg90, SpiReplay,
    Ssd1306, St77916, Ws2812,
};
use crate::mcu::{Atmega328p, Esp32s3};
use crate::netlist::PartId;

use crate::netlist::Chip;
use crate::netlist::Model;

pub(crate) mod ne555;
pub(crate) mod op_amp;
pub(crate) mod regulator;
pub(crate) mod relay;

pub use ne555::{Discharge, Ne555};
pub use op_amp::OpAmp;
pub use regulator::LinearRegulator;
pub use relay::Relay;

use crate::logic::Level;

use crate::mna::{Layout, Stamp, Unknowns};
use crate::watch::Watch;

/// A passive chip's discrete state. A value, replaced at an accepted time and never mutated.
/// The loop restarts whenever a state changes, so it holds only what the stamp reads. A chip
/// with memory that leaves its drive alone is an active chip, `active::Chip`, whose restart
/// compares only its drive image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Ne555(ne555::State),
    Relay(relay::State),
}

impl State {
    /// A number that tells this state from every other.
    pub(crate) fn code(self) -> u64 {
        match self {
            State::Ne555(state) => u64::from(state.latch == Level::High),
            State::Relay(state) => 2 + u64::from(state.contact == crate::netlist::Contact::Closed),
        }
    }
}

pub(crate) enum Kind<'n> {
    Passive(Passive<'n>),
    Active(Active<'n>),
}

/// A chip whose whole state is what its stamp reads.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Passive<'n> {
    Ne555(&'n Ne555),
    Relay(&'n Relay),
}

/// A chip whose core runs on the circuit's clock, as an `active::Chip`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Active<'n> {
    Atmega328p(&'n Atmega328p),
    SpiReplay(&'n SpiReplay),
    St77916(&'n St77916),
    Ssd1306(&'n Ssd1306),
    Hd44780I2c(&'n Hd44780I2c),
    Hd44780(&'n Hd44780),
    Cst816(&'n Cst816),
    Dht22(&'n Dht22),
    Bmp280(&'n Bmp280),
    Hc595(&'n Hc595),
    Max7219(&'n Max7219),
    Esp32s3(&'n Esp32s3),
    Ws2812(&'n Ws2812),
    PulsesReplay(&'n PulsesReplay),
    Sg90(&'n Sg90),
}

pub(crate) fn kind(chip: &Chip) -> Kind<'_> {
    match chip {
        Chip::Ne555(ne555) => Kind::Passive(Passive::Ne555(ne555)),
        Chip::Atmega328p(mcu) => Kind::Active(Active::Atmega328p(mcu)),
        Chip::SpiReplay(replay) => Kind::Active(Active::SpiReplay(replay)),
        Chip::St77916(panel) => Kind::Active(Active::St77916(panel)),
        Chip::Ssd1306(panel) => Kind::Active(Active::Ssd1306(panel)),
        Chip::Hd44780I2c(panel) => Kind::Active(Active::Hd44780I2c(panel)),
        Chip::Hd44780(panel) => Kind::Active(Active::Hd44780(panel)),
        Chip::Cst816(dev) => Kind::Active(Active::Cst816(dev)),
        Chip::Dht22(dev) => Kind::Active(Active::Dht22(dev)),
        Chip::Bmp280(dev) => Kind::Active(Active::Bmp280(dev)),
        Chip::Hc595(dev) => Kind::Active(Active::Hc595(dev)),
        Chip::Max7219(dev) => Kind::Active(Active::Max7219(dev)),
        Chip::Esp32s3(mcu) => Kind::Active(Active::Esp32s3(mcu)),
        Chip::Ws2812(chain) => Kind::Active(Active::Ws2812(chain)),
        Chip::PulsesReplay(replay) => Kind::Active(Active::PulsesReplay(replay)),
        Chip::Sg90(dev) => Kind::Active(Active::Sg90(dev)),
    }
}

/// The passive chip a part is, a passive `Chip` or a relay. The layout gives each one a slot.
pub(crate) fn passive(model: &Model) -> Option<Passive<'_>> {
    match model {
        Model::Chip(chip) => match kind(chip) {
            Kind::Passive(passive) => Some(passive),
            Kind::Active(_) => None,
        },
        Model::Relay(relay) => Some(Passive::Relay(relay)),
        _ => None,
    }
}

/// The 555's one internal node is its lower ladder node. An active chip stamps only between
/// its own pins, so it has none.
pub(crate) fn unknowns(chip: &Chip) -> Unknowns {
    match chip {
        Chip::Ne555(_) => Unknowns {
            internal_nodes: 1,
            ..Unknowns::default()
        },
        Chip::Atmega328p(_)
        | Chip::SpiReplay(_)
        | Chip::St77916(_)
        | Chip::Ssd1306(_)
        | Chip::Hd44780I2c(_)
        | Chip::Hd44780(_)
        | Chip::Cst816(_)
        | Chip::Dht22(_)
        | Chip::Bmp280(_)
        | Chip::Hc595(_)
        | Chip::Max7219(_)
        | Chip::Esp32s3(_)
        | Chip::Ws2812(_)
        | Chip::PulsesReplay(_)
        | Chip::Sg90(_) => Unknowns::default(),
    }
}

/// Whether its stamp reads the Newton iterate. The NE555 stamps from its held state, and an
/// active chip from its core's drive image. A Newton call changes neither.
pub(crate) fn reads_iterate(chip: &Chip) -> bool {
    match chip {
        Chip::Ne555(_)
        | Chip::Atmega328p(_)
        | Chip::SpiReplay(_)
        | Chip::St77916(_)
        | Chip::Ssd1306(_)
        | Chip::Hd44780I2c(_)
        | Chip::Hd44780(_)
        | Chip::Cst816(_)
        | Chip::Dht22(_)
        | Chip::Bmp280(_)
        | Chip::Hc595(_)
        | Chip::Max7219(_)
        | Chip::Esp32s3(_)
        | Chip::Ws2812(_)
        | Chip::PulsesReplay(_)
        | Chip::Sg90(_) => false,
    }
}

/// Before the operating point settles it.
pub(crate) fn power_on(chip: Passive<'_>) -> State {
    match chip {
        Passive::Ne555(_) => State::Ne555(ne555::POWER_ON),
        Passive::Relay(_) => State::Relay(relay::POWER_ON),
    }
}

/// Its watches, in an order the chip owns. A watch reads node voltages, never a branch current.
pub(crate) fn watches(chip: Passive<'_>, part: PartId, layout: &Layout) -> Vec<Watch> {
    match chip {
        Passive::Ne555(ne555) => ne555::watches(ne555, layout.internal(part, 0)),
        Passive::Relay(relay) => relay::watches(relay, layout.internal(part, 0)),
    }
}

pub(crate) fn stamp(chip: Passive<'_>, state: State, stamp: &mut Stamp<'_>) {
    match (chip, state) {
        (Passive::Ne555(ne555), State::Ne555(state)) => ne555::stamp(ne555, state, stamp),
        (Passive::Relay(relay), State::Relay(state)) => relay::stamp(relay, state, stamp),
        _ => unreachable!("a chip's state is its own kind's"),
    }
}

/// The next state from its watches' levels just before and just after one time. Pure.
/// `before` is there for an edge-triggered chip. The 555 reads only `after`.
pub(crate) fn react(chip: Passive<'_>, state: &State, _before: &[bool], after: &[bool]) -> State {
    match (chip, state) {
        (Passive::Ne555(_), State::Ne555(state)) => State::Ne555(ne555::react(*state, after)),
        (Passive::Relay(_), State::Relay(_)) => State::Relay(relay::react(after)),
        _ => unreachable!("a chip's state is its own kind's"),
    }
}

/// Its logic signals, by name in name order.
pub(crate) fn signals(chip: Passive<'_>) -> &'static [&'static str] {
    match chip {
        Passive::Ne555(_) => &ne555::SIGNALS,
        Passive::Relay(_) => &relay::SIGNALS,
    }
}

/// Its signals' levels, in `signals` order.
pub(crate) fn levels(chip: Passive<'_>, state: &State, watched: &[bool]) -> Vec<Level> {
    match (chip, state) {
        (Passive::Ne555(_), State::Ne555(state)) => ne555::levels(*state, watched).to_vec(),
        (Passive::Relay(_), State::Relay(state)) => relay::levels(*state).to_vec(),
        _ => unreachable!("a chip's state is its own kind's"),
    }
}

/// The pin pairs it joins at DC, for `net.floating`: the union over every state. A pair that
/// some state opens, such as a high-impedance output, is still listed, so a net reached only
/// through it is not reported floating while it is open.
pub(crate) fn dc_paths(chip: &Chip) -> &'static [(usize, usize)] {
    match chip {
        Chip::Ne555(_) => &ne555::DC_PATHS,
        Chip::Atmega328p(_) => &crate::mcu::atmega328p::DC_PATHS,
        Chip::SpiReplay(_) => &crate::device::spi_replay::DC_PATHS,
        Chip::St77916(_) => &crate::device::st77916::DC_PATHS,
        Chip::Ssd1306(_) => &crate::device::ssd1306::DC_PATHS,
        Chip::Hd44780I2c(_) => &crate::device::hd44780_i2c::DC_PATHS,
        Chip::Hd44780(_) => &crate::device::hd44780::DC_PATHS,
        Chip::Cst816(_) => &crate::device::cst816::DC_PATHS,
        Chip::Dht22(_) => &crate::device::dht22::DC_PATHS,
        Chip::Bmp280(_) => &crate::device::bmp280::DC_PATHS,
        Chip::Hc595(_) => &crate::device::hc595::DC_PATHS,
        Chip::Max7219(_) => &crate::device::max7219::DC_PATHS,
        Chip::Esp32s3(mcu) => crate::mcu::esp32s3::dc_paths(mcu),
        Chip::Ws2812(chain) => crate::device::ws2812::dc_paths(chain),
        Chip::PulsesReplay(_) => &crate::device::pulses_replay::DC_PATHS,
        Chip::Sg90(_) => &crate::device::sg90::DC_PATHS,
    }
}

/// The chip's operating temperature range, `None` when its part rates none.
pub(crate) fn temperature_range(chip: &Chip) -> Option<&crate::netlist::TemperatureRange> {
    match chip {
        Chip::Dht22(dev) => dev.temperature(),
        Chip::Bmp280(dev) => dev.ratings.temperature.as_ref(),
        Chip::Atmega328p(_)
        | Chip::Esp32s3(_)
        | Chip::Hc595(_)
        | Chip::Max7219(_)
        | Chip::Ssd1306(_)
        | Chip::Hd44780I2c(_)
        | Chip::Hd44780(_)
        | Chip::Ws2812(_)
        | Chip::Sg90(_)
        | Chip::Cst816(_)
        | Chip::St77916(_)
        | Chip::Ne555(_)
        | Chip::SpiReplay(_)
        | Chip::PulsesReplay(_) => None,
    }
}

/// The ratings a chip puts on its package pins, or `None` for a chip whose part rates none.
pub(crate) fn pin_ratings(chip: &Chip) -> Option<crate::ratings::PinRatings> {
    match chip {
        Chip::Atmega328p(mcu) => crate::mcu::atmega328p::pin_ratings(mcu),
        Chip::Esp32s3(mcu) => crate::mcu::esp32s3::pin_ratings(mcu),
        Chip::Bmp280(dev) => crate::device::bmp280::pin_ratings(dev),
        Chip::Hc595(dev) => crate::device::hc595::pin_ratings(dev),
        Chip::Max7219(dev) => crate::device::max7219::pin_ratings(dev),
        Chip::Ssd1306(dev) => crate::device::ssd1306::pin_ratings(dev),
        Chip::Hd44780I2c(dev) => crate::device::hd44780_i2c::pin_ratings(dev),
        Chip::Hd44780(dev) => crate::device::hd44780::pin_ratings(dev),
        Chip::Ws2812(chain) => crate::device::ws2812::pin_ratings(chain),
        Chip::Dht22(dev) => crate::device::dht22::pin_ratings(dev),
        Chip::Sg90(dev) => crate::device::sg90::pin_ratings(dev),
        Chip::Cst816(dev) => crate::device::cst816::pin_ratings(dev),
        Chip::St77916(panel) => crate::device::st77916::pin_ratings(panel),
        Chip::Ne555(chip) => ne555::pin_ratings(chip),
        Chip::SpiReplay(_) | Chip::PulsesReplay(_) => None,
    }
}
