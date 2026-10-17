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
            internal_nodes: 0,
            branches: 1,
        },
    }
}

pub(crate) fn stamp(model: &Model, stamp: &mut Stamp<'_>) {
    match model {
        Model::Resistor(resistor) => resistor::stamp(resistor, stamp),
        Model::VoltageSource(source) => voltage_source::stamp(source, stamp),
    }
}

pub(crate) fn dc_paths(model: &Model) -> &'static [(usize, usize)] {
    match model {
        Model::Resistor(_) | Model::VoltageSource(_) => &[(0, 1)],
    }
}

pub(crate) fn observe(
    model: &Model,
    name: &Name,
    probe: &Probe<'_>,
    out: &mut BTreeMap<Ref, Finite>,
) -> Result<(), NonFinite> {
    match model {
        Model::Resistor(resistor) => two_terminal(
            name,
            resistor.pins,
            resistor::current(resistor, probe),
            probe,
            out,
        ),
        Model::VoltageSource(source) => two_terminal(
            name,
            source.pins,
            voltage_source::current(probe),
            probe,
            out,
        ),
    }
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
