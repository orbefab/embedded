mod capacitor;
mod junction;
mod resistor;
mod switch;
mod voltage_source;

use sim_core::netlist::Model;
use sim_core::time::Time;

use crate::mna::{Probe, Stamp, Unknowns};

pub(crate) fn unknowns(model: &Model) -> Unknowns {
    match model {
        Model::Resistor(_) | Model::Switch(_) => Unknowns::default(),
        Model::VoltageSource(_) | Model::Capacitor(_) => Unknowns {
            branches: 1,
            ..Unknowns::default()
        },
        Model::Diode(_) | Model::Led(_) => Unknowns {
            internal_nodes: 1,
            branches: 0,
            junctions: 1,
        },
    }
}

pub(crate) fn stamp(model: &Model, stamp: &mut Stamp<'_>) {
    match model {
        Model::Resistor(resistor) => resistor::stamp(resistor, stamp),
        Model::VoltageSource(source) => voltage_source::stamp(source, stamp),
        Model::Diode(j) | Model::Led(j) => junction::stamp(j, stamp),
        Model::Capacitor(capacitor) => capacitor::stamp(capacitor, stamp),
        Model::Switch(switch) => switch::stamp(switch, stamp),
    }
}

/// Start voltages for this part's junction slots.
pub(crate) fn initial(model: &Model, out: &mut [f64]) {
    match model {
        Model::Resistor(_) | Model::VoltageSource(_) | Model::Capacitor(_) | Model::Switch(_) => {}
        Model::Diode(j) | Model::Led(j) => out[0] = junction::initial(j),
    }
}

/// After a solve: limit this part's junctions from `prior` into `next`, and check their currents.
pub(crate) fn update(model: &Model, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
    match model {
        Model::Resistor(_) | Model::VoltageSource(_) | Model::Capacitor(_) | Model::Switch(_) => {
            Update::default()
        }
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
/// A capacitor is open. A switch is a resistor either way.
pub(crate) fn dc_paths(model: &Model) -> &'static [(usize, usize)] {
    match model {
        Model::Resistor(_)
        | Model::VoltageSource(_)
        | Model::Diode(_)
        | Model::Led(_)
        | Model::Switch(_) => &[(0, 1)],
        Model::Capacitor(_) => &[],
    }
}

/// `i`, `v` and `p` at `at`, in `Quantity` order. Not checked for finiteness.
pub(crate) fn observe(model: &Model, probe: &Probe<'_>, at: Time) -> [f64; 3] {
    let (pins, i) = match model {
        Model::Resistor(resistor) => (resistor.pins, resistor::current(resistor, probe)),
        Model::VoltageSource(source) => (source.pins, voltage_source::current(probe)),
        Model::Diode(j) | Model::Led(j) => (j.pins, junction::current(j, probe)),
        Model::Capacitor(capacitor) => (capacitor.pins, capacitor::current(probe)),
        Model::Switch(switch) => (switch.pins, switch::current(switch, probe, at)),
    };
    let v = probe.v(pins[0]) - probe.v(pins[1]);
    [i, v, v * i]
}
