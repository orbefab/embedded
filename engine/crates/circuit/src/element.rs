mod capacitor;
mod current_source;
mod junction;
mod potentiometer;
mod resistor;
mod switch;
mod voltage_source;

use sim_core::netlist::Model;
use sim_core::time::Time;

use crate::chip;
use crate::mna::{Probe, Stamp, Unknowns};
use crate::newton::Side;

pub(crate) fn unknowns(model: &Model) -> Unknowns {
    match model {
        Model::Resistor(_)
        | Model::Potentiometer(_)
        | Model::CurrentSource(_)
        | Model::Switch(_) => Unknowns::default(),
        Model::VoltageSource(_) | Model::Capacitor(_) => Unknowns {
            branches: 1,
            ..Unknowns::default()
        },
        Model::Diode(_) | Model::Led(_) => Unknowns {
            internal_nodes: 1,
            branches: 0,
            junctions: 1,
        },
        Model::Chip(chip) => chip::unknowns(chip),
    }
}

pub(crate) fn stamp(model: &Model, stamp: &mut Stamp<'_>) {
    match model {
        Model::Resistor(resistor) => resistor::stamp(resistor, stamp),
        Model::Potentiometer(pot) => potentiometer::stamp(pot, stamp),
        Model::VoltageSource(source) => voltage_source::stamp(source, stamp),
        Model::CurrentSource(source) => current_source::stamp(source, stamp),
        Model::Diode(j) | Model::Led(j) => junction::stamp(j, stamp),
        Model::Capacitor(capacitor) => capacitor::stamp(capacitor, stamp),
        Model::Switch(switch) => switch::stamp(switch, stamp),
        // An active chip stamps its core's drive image, in `active::stamp`.
        Model::Chip(chip) => {
            if let chip::Kind::Passive(passive) = chip::kind(chip) {
                chip::stamp(passive, *stamp.chip(), stamp);
            }
        }
    }
}

/// Start voltages for this part's junction slots.
pub(crate) fn initial(model: &Model, out: &mut [f64]) {
    match model {
        Model::Resistor(_)
        | Model::Potentiometer(_)
        | Model::VoltageSource(_)
        | Model::CurrentSource(_)
        | Model::Capacitor(_)
        | Model::Switch(_)
        | Model::Chip(_) => {}
        Model::Diode(j) | Model::Led(j) => out[0] = junction::initial(j),
    }
}

/// After a solve: limit this part's junctions from `prior` into `next`, and check their currents.
pub(crate) fn update(model: &Model, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
    match model {
        Model::Resistor(_)
        | Model::Potentiometer(_)
        | Model::VoltageSource(_)
        | Model::CurrentSource(_)
        | Model::Capacitor(_)
        | Model::Switch(_)
        | Model::Chip(_) => Update::default(),
        Model::Diode(j) | Model::Led(j) => {
            let (limited, update) = junction::update(j, probe, prior[0]);
            next[0] = limited;
            update
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Update {
    pub(crate) limited: bool,
    /// Worst current mismatch over its tolerance. At most 1 passes.
    pub(crate) mismatch: f64,
}

/// The pin pairs a part joins at DC. A chain of them is a DC path for `net.floating`.
/// A capacitor is open unless the operating point holds it at `initial`. A switch is a resistor either way.
/// A current source joins nothing.
pub(crate) fn dc_paths(model: &Model) -> &'static [(usize, usize)] {
    match model {
        Model::Resistor(_)
        | Model::VoltageSource(_)
        | Model::Diode(_)
        | Model::Led(_)
        | Model::Switch(_) => &[(0, 1)],
        Model::Potentiometer(_) => &[(0, 1), (1, 2)],
        Model::Capacitor(capacitor) if capacitor.initial.is_some() => &[(0, 1)],
        Model::Capacitor(_) | Model::CurrentSource(_) => &[],
        Model::Chip(chip) => chip::dc_paths(chip),
    }
}

/// `i`, `v` and `p` at `at`, in `Quantity` order, for a part that `has_quantities`.
/// Not checked for finiteness.
pub(crate) fn observe(model: &Model, probe: &Probe<'_>, at: Time, side: Side) -> Option<[f64; 3]> {
    if !model.has_quantities() {
        return None;
    }
    let (pins, i) = match model {
        Model::Chip(_) | Model::Potentiometer(_) => unreachable!("has_quantities returned true"),
        Model::Resistor(resistor) => (resistor.pins, resistor::current(resistor, probe)),
        Model::VoltageSource(source) => (source.pins, voltage_source::current(probe)),
        Model::CurrentSource(source) => (source.pins, current_source::current(source, at, side)),
        Model::Diode(j) | Model::Led(j) => (j.pins, junction::current(j, probe)),
        Model::Capacitor(capacitor) => (capacitor.pins, capacitor::current(probe)),
        Model::Switch(switch) => (switch.pins, switch::current(switch, probe, at, side)),
    };
    let v = probe.v(pins[0]) - probe.v(pins[1]);
    Some([i, v, v * i])
}
