mod capacitor;
mod current_source;
mod junction;
mod potentiometer;
mod resistor;
mod switch;
mod transistor;
mod voltage_source;

use sim_core::netlist::{Model, NetId, PartId};
use sim_core::time::Time;

use crate::chip;
use crate::mna::{Layout, Node, Probe, Stamp, Unknowns};
use crate::newton::Side;
use transistor::Polarity;

/// What the layout and the stamp read from a part. A netlist `Model` is one. A test
/// passes its own value through the same functions.
pub(crate) trait Device {
    fn unknowns(&self) -> Unknowns;
    fn charges(&self) -> Vec<OwnedCapacitor>;
    fn stamp(&self, stamp: &mut Stamp<'_>);
}

pub(crate) fn unknowns(model: &impl Device) -> Unknowns {
    Device::unknowns(model)
}

/// One end of a capacitor. A net, or internal node `k` of this part.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CapacitorEnd {
    Net(NetId),
    Internal(usize),
}

impl CapacitorEnd {
    pub(crate) fn node(self, layout: &Layout, part: PartId) -> Node {
        match self {
            CapacitorEnd::Net(net) => Node::from(net),
            CapacitorEnd::Internal(k) => layout.internal(part, k),
        }
    }
}

/// One capacitor a part owns. `k` is its index in that part's charge list, and branch
/// `k` carries its current. Capacitors take the part's first branches. An inductor is
/// not this type.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnedCapacitor {
    pub(crate) k: usize,
    pub(crate) ends: [CapacitorEnd; 2],
    pub(crate) farad: f64,
}

/// The capacitors `model` owns, in index order. A part with none returns an empty list.
pub(crate) fn charges(model: &impl Device) -> Vec<OwnedCapacitor> {
    Device::charges(model)
}

pub(crate) fn stamp(model: &impl Device, stamp: &mut Stamp<'_>) {
    Device::stamp(model, stamp);
}

impl Device for Model {
    fn unknowns(&self) -> Unknowns {
        match self {
            Model::Resistor(_)
            | Model::Potentiometer(_)
            | Model::CurrentSource(_)
            | Model::Switch(_) => Unknowns::default(),
            Model::VoltageSource(_) => Unknowns {
                branches: 1,
                ..Unknowns::default()
            },
            Model::Capacitor(_) => Unknowns {
                branches: 1,
                ..Unknowns::default()
            },
            Model::Diode(_) | Model::Led(_) => Unknowns {
                internal_nodes: 1,
                bare_internal: false,
                branches: 0,
                junctions: 1,
            },
            Model::Npn(_) | Model::Pnp(_) => Unknowns {
                internal_nodes: 3,
                bare_internal: true,
                branches: 2,
                junctions: 2,
            },
            Model::Chip(chip) => chip::unknowns(chip),
        }
    }

    fn charges(&self) -> Vec<OwnedCapacitor> {
        match self {
            Model::Capacitor(capacitor) => vec![OwnedCapacitor {
                k: 0,
                ends: capacitor.pins.map(CapacitorEnd::Net),
                farad: capacitor.capacitance.si.get(),
            }],
            Model::Npn(bjt) | Model::Pnp(bjt) => transistor::charges(bjt),
            Model::Resistor(_)
            | Model::Potentiometer(_)
            | Model::VoltageSource(_)
            | Model::CurrentSource(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Switch(_)
            | Model::Chip(_) => Vec::new(),
        }
    }

    fn stamp(&self, stamp: &mut Stamp<'_>) {
        match self {
            Model::Resistor(resistor) => resistor::stamp(resistor, stamp),
            Model::Potentiometer(pot) => potentiometer::stamp(pot, stamp),
            Model::VoltageSource(source) => voltage_source::stamp(source, stamp),
            Model::CurrentSource(source) => current_source::stamp(source, stamp),
            Model::Diode(j) | Model::Led(j) => junction::stamp(j, stamp),
            Model::Npn(bjt) => transistor::stamp(bjt, Polarity::Npn, stamp),
            Model::Pnp(bjt) => transistor::stamp(bjt, Polarity::Pnp, stamp),
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
}

/// Whether this part's stamp reads the Newton iterate. Only a junction's does, a diode's or a
/// transistor's, through
/// `Stamp::junction`, the one reader of the iterate. Every other stamp reads only the call's
/// conditions, so on a circuit where no part reads it every iteration of one call stamps the
/// same system.
pub(crate) fn reads_iterate(model: &Model) -> bool {
    match model {
        Model::Resistor(_)
        | Model::Potentiometer(_)
        | Model::VoltageSource(_)
        | Model::CurrentSource(_)
        | Model::Capacitor(_)
        | Model::Switch(_) => false,
        Model::Diode(_) | Model::Led(_) | Model::Npn(_) | Model::Pnp(_) => true,
        Model::Chip(chip) => chip::reads_iterate(chip),
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
        Model::Npn(bjt) | Model::Pnp(bjt) => transistor::initial(bjt, out),
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
        Model::Npn(bjt) => transistor::update(bjt, Polarity::Npn, probe, prior, next),
        Model::Pnp(bjt) => transistor::update(bjt, Polarity::Pnp, probe, prior, next),
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
/// A current source joins nothing. A transistor joins its base to each other pin, because each
/// junction conducts.
pub(crate) fn dc_paths(model: &Model) -> &'static [(usize, usize)] {
    match model {
        Model::Resistor(_)
        | Model::VoltageSource(_)
        | Model::Diode(_)
        | Model::Led(_)
        | Model::Switch(_) => &[(0, 1)],
        Model::Potentiometer(_) | Model::Npn(_) | Model::Pnp(_) => &[(0, 1), (1, 2)],
        Model::Capacitor(capacitor) if capacitor.initial.is_some() => &[(0, 1)],
        Model::Capacitor(_) | Model::CurrentSource(_) => &[],
        Model::Chip(chip) => chip::dc_paths(chip),
    }
}

/// `model.quantities()` at `at`, in that order. `None` for a part with none.
/// Not checked for finiteness.
pub(crate) fn observe(model: &Model, probe: &Probe<'_>, at: Time, side: Side) -> Option<[f64; 3]> {
    let (pins, i) = match model {
        Model::Chip(_) | Model::Potentiometer(_) => return None,
        Model::Npn(bjt) | Model::Pnp(bjt) => return Some(transistor::observe(bjt, probe)),
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

/// Two capacitors on one part. Tests only. Not a netlist kind. `branches` is the count
/// `unknowns` reserves, so a port that leaves it at 0 fails when the layout is built.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pair {
    pub(crate) ends: [[CapacitorEnd; 2]; 2],
    pub(crate) farad: [f64; 2],
    pub(crate) internal_nodes: usize,
    pub(crate) branches: usize,
}

#[cfg(test)]
impl Device for Pair {
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: self.internal_nodes,
            bare_internal: false,
            branches: self.branches,
            junctions: 0,
        }
    }

    fn charges(&self) -> Vec<OwnedCapacitor> {
        self.ends
            .iter()
            .zip(self.farad)
            .enumerate()
            .map(|(k, (ends, farad))| OwnedCapacitor {
                k,
                ends: *ends,
                farad,
            })
            .collect()
    }

    fn stamp(&self, stamp: &mut Stamp<'_>) {
        for cap in self.charges() {
            capacitor::stamp_capacitor(stamp, cap.k, cap.ends, cap.farad, None);
        }
    }
}
