//! The chip seam: each question the loop asks a chip. `kind` alone says whether a netlist chip
//! is passive or active, and every kind answers `unknowns` and `dc_paths` through it. A passive
//! kind adds a `Passive` variant, a `chip/<kind>.rs`, and an arm in each function that takes
//! `Passive`. An active kind adds an `Active` variant, its arm in `active::chip`, and an
//! `active::Core` variant.

mod ne555;

use sim_core::logic::Level;
use sim_core::netlist::{Atmega328p, Chip, Ne555, PartId};

use crate::mna::{Layout, Stamp, Unknowns};
use crate::watch::Watch;

/// A passive chip's discrete state. A value, replaced at an accepted time and never mutated.
/// The loop restarts whenever a state changes, so it holds only what the stamp reads. A chip
/// with memory that leaves its drive alone is an active chip, `active::Chip`, whose restart
/// compares only its drive image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Ne555(ne555::State),
}

pub(crate) enum Kind<'n> {
    Passive(Passive<'n>),
    Active(Active<'n>),
}

/// A chip whose whole state is what its stamp reads.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Passive<'n> {
    Ne555(&'n Ne555),
}

/// A chip whose core runs on the circuit's clock, as an `active::Chip`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Active<'n> {
    Atmega328p(&'n Atmega328p),
}

pub(crate) fn kind(chip: &Chip) -> Kind<'_> {
    match chip {
        Chip::Ne555(ne555) => Kind::Passive(Passive::Ne555(ne555)),
        Chip::Atmega328p(mcu) => Kind::Active(Active::Atmega328p(mcu)),
    }
}

/// The 555's one internal node is its lower ladder node. An active chip stamps only between
/// its own pins, so it has none.
pub(crate) fn unknowns(chip: &Chip) -> Unknowns {
    match kind(chip) {
        Kind::Passive(Passive::Ne555(_)) => Unknowns {
            internal_nodes: 1,
            ..Unknowns::default()
        },
        Kind::Active(_) => Unknowns::default(),
    }
}

/// Before the operating point settles it.
pub(crate) fn power_on(chip: Passive<'_>) -> State {
    match chip {
        Passive::Ne555(_) => State::Ne555(ne555::POWER_ON),
    }
}

/// Its watches, in an order the chip owns. A watch reads node voltages, never a branch current.
pub(crate) fn watches(chip: Passive<'_>, part: PartId, layout: &Layout) -> Vec<Watch> {
    match chip {
        Passive::Ne555(ne555) => ne555::watches(ne555, layout.internal(part, 0)),
    }
}

pub(crate) fn stamp(chip: Passive<'_>, state: State, stamp: &mut Stamp<'_>) {
    match (chip, state) {
        (Passive::Ne555(ne555), State::Ne555(state)) => ne555::stamp(ne555, state, stamp),
    }
}

/// The next state from its watches' levels just before and just after one time. Pure.
/// `before` is there for an edge-triggered chip. The 555 reads only `after`.
pub(crate) fn react(chip: Passive<'_>, state: &State, _before: &[bool], after: &[bool]) -> State {
    match (chip, state) {
        (Passive::Ne555(_), State::Ne555(state)) => State::Ne555(ne555::react(*state, after)),
    }
}

/// Its logic signals, by name in name order.
pub(crate) fn signals(chip: Passive<'_>) -> &'static [&'static str] {
    match chip {
        Passive::Ne555(_) => &ne555::SIGNALS,
    }
}

/// Its signals' levels, in `signals` order.
pub(crate) fn levels(chip: Passive<'_>, state: &State, watched: &[bool]) -> Vec<Level> {
    match (chip, state) {
        (Passive::Ne555(_), State::Ne555(state)) => ne555::levels(*state, watched).to_vec(),
    }
}

/// The pin pairs it joins at DC, for `net.floating`: the union over every state. A pair that
/// some state opens, such as a high-impedance output, is still listed, so a net reached only
/// through it is not reported floating while it is open.
pub(crate) fn dc_paths(chip: &Chip) -> &'static [(usize, usize)] {
    match kind(chip) {
        Kind::Passive(Passive::Ne555(_)) => &ne555::DC_PATHS,
        Kind::Active(Active::Atmega328p(_)) => &crate::active::avr::DC_PATHS,
    }
}
