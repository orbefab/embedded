mod junction;
mod resistor;
mod voltage_source;

use std::collections::BTreeMap;

use sim_core::finite::Finite;
use sim_core::name::{Name, Quantity, Ref};
use sim_core::netlist::{Model, NetId};

use crate::mna::{Probe, Stamp, Unknowns};

pub(crate) fn unknowns(model: &Model) -> Unknowns {
    match model {
        Model::Resistor(_) => Unknowns::default(),
        Model::VoltageSource(_) => Unknowns {
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
    }
}

/// Start voltages for this part's junction slots.
pub(crate) fn initial(model: &Model, out: &mut [f64]) {
    match model {
        Model::Resistor(_) | Model::VoltageSource(_) => {}
        Model::Diode(j) | Model::Led(j) => out[0] = junction::initial(j),
    }
}

/// After a solve: limit this part's junctions from `prior` into `next`, and check their currents.
pub(crate) fn update(model: &Model, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
    match model {
        Model::Resistor(_) | Model::VoltageSource(_) => Update::default(),
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

/// Each part joins its two pins. A chain of them is a DC path for `net.floating`.
pub(crate) fn dc_paths(model: &Model) -> &'static [(usize, usize)] {
    match model {
        Model::Resistor(_) | Model::VoltageSource(_) | Model::Diode(_) | Model::Led(_) => &[(0, 1)],
    }
}

pub(crate) fn observe(
    model: &Model,
    name: &Name,
    probe: &Probe<'_>,
    out: &mut BTreeMap<Ref, Finite>,
) -> Result<(), NonFinite> {
    let (pins, i) = match model {
        Model::Resistor(resistor) => (resistor.pins, resistor::current(resistor, probe)),
        Model::VoltageSource(source) => (source.pins, voltage_source::current(probe)),
        Model::Diode(j) | Model::Led(j) => (j.pins, junction::current(j, probe)),
    };
    two_terminal(name, pins, i, probe, out)
}

#[derive(Debug)]
pub(crate) struct NonFinite;

fn two_terminal(
    name: &Name,
    pins: [NetId; 2],
    i: f64,
    probe: &Probe<'_>,
    out: &mut BTreeMap<Ref, Finite>,
) -> Result<(), NonFinite> {
    let v = probe.v(pins[0]) - probe.v(pins[1]);
    let p = v * i;
    for (quantity, value) in [(Quantity::I, i), (Quantity::V, v), (Quantity::P, p)] {
        let finite = Finite::new(value).ok_or(NonFinite)?;
        out.insert(Ref::Quantity(name.clone(), quantity), finite);
    }
    Ok(())
}
