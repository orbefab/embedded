//! A relay as its coil and an SPDT contact that follows the coil current with hysteresis. The
//! contact moves at the located crossing, with no armature travel and no bounce.

use orbe_engine_core::logic::Level;
use orbe_engine_core::netlist::{Contact, Relay};

use crate::element::{self, OwnedInductor};
use crate::mna::{Node, Stamp};
use crate::watch::{Signal, Watch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) contact: Contact,
}

/// The operating point closes it when the coil starts above pull-in.
pub(crate) const POWER_ON: State = State {
    contact: Contact::Open,
};

pub(crate) const SIGNALS: [&str; 1] = ["contact"];

/// `coil_a` through the coil resistance to internal node 0, then the inductance to `coil_b`.
pub(crate) fn coil(relay: &Relay) -> OwnedInductor {
    element::coil(
        relay.pins[Relay::COIL_B],
        relay.inductance.si.get(),
        relay.coil_resistance.si.get(),
    )
}

/// One watch per polarity of the coil current, read across the coil resistance from `coil_a`
/// to internal node `x`, which carries the inductor's current exactly. Each rises above
/// pull-in and falls at drop-out or below.
pub(crate) fn watches(relay: &Relay, x: Node) -> Vec<Watch> {
    let g = 1.0 / relay.coil_resistance.si.get();
    let a = Node::from(relay.pins[Relay::COIL_A]);
    let current = |sign: f64, offset: f64| Signal {
        terms: vec![(a, sign * g), (x, -sign * g)],
        offset,
    };
    let (pull_in, drop_out) = (
        relay.pull_in_current.si.get(),
        relay.drop_out_current.si.get(),
    );
    [1.0, -1.0]
        .map(|sign| Watch {
            rise: current(sign, pull_in),
            fall: current(sign, drop_out),
        })
        .to_vec()
}

pub(crate) fn stamp(relay: &Relay, state: State, stamp: &mut Stamp<'_>) {
    let pin = |k: usize| relay.pins[k];
    element::stamp_coil(stamp, pin(Relay::COIL_A), coil(relay));
    let on = 1.0 / relay.on_resistance.si.get();
    let off = 1.0 / relay.off_resistance.si.get();
    let (no, nc) = match state.contact {
        Contact::Closed => (on, off),
        Contact::Open => (off, on),
    };
    stamp.conductance(pin(Relay::COM), pin(Relay::NO), no);
    stamp.conductance(pin(Relay::COM), pin(Relay::NC), nc);
}

/// Closed while either polarity's watch is high. Level-sensitive.
pub(crate) fn react(levels: &[bool]) -> State {
    let contact = if levels.iter().any(|&high| high) {
        Contact::Closed
    } else {
        Contact::Open
    };
    State { contact }
}

pub(crate) fn levels(state: State) -> [Level; 1] {
    [Level::from_bool(state.contact == Contact::Closed)]
}
