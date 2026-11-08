//! The chip seam: each question the loop asks a chip. `kind` alone says whether a netlist chip
//! is passive or active, and every `Chip` answers `unknowns` and `dc_paths` here. A passive
//! kind adds a `Passive` variant, a `chip/<kind>.rs`, an arm in `passive`, and an arm in each
//! function that takes `Passive`. An active kind adds an `Active` variant, its arm in
//! `active::chip`, and an `active::Core` variant. A relay is a passive chip that is its own
//! netlist kind, not a `Chip`, so it answers `unknowns` and `dc_paths` as an element.

mod ne555;
mod relay;

pub(crate) use relay::coil as relay_coil;

use orbe_engine_core::logic::Level;
use orbe_engine_core::netlist::{
    Atmega328p, Chip, Cst816, Esp32s3, Model, Ne555, PartId, PulsesReplay, Relay, SpiReplay,
    St77916, Ws2812,
};

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
    Cst816(&'n Cst816),
    Esp32s3(&'n Esp32s3),
    Ws2812(&'n Ws2812),
    PulsesReplay(&'n PulsesReplay),
}

pub(crate) fn kind(chip: &Chip) -> Kind<'_> {
    match chip {
        Chip::Ne555(ne555) => Kind::Passive(Passive::Ne555(ne555)),
        Chip::Atmega328p(mcu) => Kind::Active(Active::Atmega328p(mcu)),
        Chip::SpiReplay(replay) => Kind::Active(Active::SpiReplay(replay)),
        Chip::St77916(panel) => Kind::Active(Active::St77916(panel)),
        Chip::Cst816(dev) => Kind::Active(Active::Cst816(dev)),
        Chip::Esp32s3(mcu) => Kind::Active(Active::Esp32s3(mcu)),
        Chip::Ws2812(chain) => Kind::Active(Active::Ws2812(chain)),
        Chip::PulsesReplay(replay) => Kind::Active(Active::PulsesReplay(replay)),
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
        | Chip::Cst816(_)
        | Chip::Esp32s3(_)
        | Chip::Ws2812(_)
        | Chip::PulsesReplay(_) => Unknowns::default(),
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
        | Chip::Cst816(_)
        | Chip::Esp32s3(_)
        | Chip::Ws2812(_)
        | Chip::PulsesReplay(_) => false,
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
        Chip::Atmega328p(_) => &crate::active::avr::DC_PATHS,
        Chip::SpiReplay(_) => &crate::active::replay::DC_PATHS,
        Chip::St77916(_) => &crate::active::st77916::DC_PATHS,
        Chip::Cst816(_) => &crate::active::cst816::DC_PATHS,
        Chip::Esp32s3(mcu) => crate::active::esp32s3::dc_paths(mcu),
        Chip::Ws2812(_) => &crate::active::ws2812::DC_PATHS,
        Chip::PulsesReplay(_) => &crate::active::pulses::DC_PATHS,
    }
}
