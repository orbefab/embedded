//! The chip seam: each question the loop asks a chip, dispatched over `Chip`. A new chip kind
//! adds a `Chip` variant, a `chip/<kind>.rs`, and one arm per function here.

mod ne555;

use sim_core::logic::Level;
use sim_core::netlist::{Chip, PartId};

use crate::mna::{Layout, Stamp, Unknowns};
use crate::watch::Watch;

/// A chip's discrete state. A value, replaced at an accepted time and never mutated. The loop
/// restarts whenever a state changes, so it holds only what the stamp reads. A chip with memory
/// that leaves its drive alone, such as a register, needs a drive view for the restart to
/// compare instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Ne555(ne555::State),
}

/// The 555's one internal node is its lower ladder node.
pub(crate) fn unknowns(chip: &Chip) -> Unknowns {
    match chip {
        Chip::Ne555(_) => Unknowns {
            internal_nodes: 1,
            ..Unknowns::default()
        },
    }
}

/// Before the operating point settles it.
pub(crate) fn power_on(chip: &Chip) -> State {
    match chip {
        Chip::Ne555(_) => State::Ne555(ne555::POWER_ON),
    }
}

/// Its watches, in an order the chip owns. A watch reads node voltages, never a branch current.
pub(crate) fn watches(chip: &Chip, part: PartId, layout: &Layout) -> Vec<Watch> {
    match chip {
        Chip::Ne555(ne555) => ne555::watches(ne555, layout.internal(part, 0)),
    }
}

pub(crate) fn stamp(chip: &Chip, state: State, stamp: &mut Stamp<'_>) {
    match (chip, state) {
        (Chip::Ne555(ne555), State::Ne555(state)) => ne555::stamp(ne555, state, stamp),
    }
}

/// The next state from its watches' levels just before and just after one time. Pure.
/// `before` is there for an edge-triggered chip. The 555 reads only `after`.
pub(crate) fn react(chip: &Chip, state: &State, _before: &[bool], after: &[bool]) -> State {
    match (chip, state) {
        (Chip::Ne555(_), State::Ne555(state)) => State::Ne555(ne555::react(*state, after)),
    }
}

/// Its logic signals, by name in name order.
pub(crate) fn signals(chip: &Chip) -> &'static [&'static str] {
    match chip {
        Chip::Ne555(_) => &ne555::SIGNALS,
    }
}

/// Its signals' levels, in `signals` order.
pub(crate) fn levels(chip: &Chip, state: &State, watched: &[bool]) -> Vec<Level> {
    match (chip, state) {
        (Chip::Ne555(_), State::Ne555(state)) => ne555::levels(*state, watched).to_vec(),
    }
}

/// The pin pairs it joins at DC, for `net.floating`: the union over every state. A pair that
/// some state opens, such as a high-impedance output, is still listed, so a net reached only
/// through it is not reported floating while it is open.
pub(crate) fn dc_paths(chip: &Chip) -> &'static [(usize, usize)] {
    match chip {
        Chip::Ne555(_) => &ne555::DC_PATHS,
    }
}
