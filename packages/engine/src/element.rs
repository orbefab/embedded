use orbe_model::Time;

use crate::netlist::{Model, NetId, PartId};

use crate::mna::{Layout, Node, Probe, Stamp, Unknowns};
use crate::newton::Side;
pub(crate) use crate::passive::inductor::{coil, stamp_coil};
#[cfg(test)]
use crate::passive::{capacitor, inductor};

/// What the layout and the stamp read from a part. A netlist `Model` is one. A test
/// passes its own value through the same functions.
pub(crate) trait Device {
    fn unknowns(&self) -> Unknowns;
    fn charges(&self) -> Vec<OwnedCapacitor>;
    fn inductors(&self) -> Vec<OwnedInductor>;
    fn stamp(&self, stamp: &mut Stamp<'_>);
}

pub(crate) fn unknowns(model: &impl Device) -> Unknowns {
    Device::unknowns(model)
}

/// One end of a capacitor or an inductor. A net, or internal node `k` of this part.
#[derive(Clone, Copy, Debug)]
pub(crate) enum StoreEnd {
    Net(NetId),
    Internal(usize),
}

impl StoreEnd {
    pub(crate) fn node(self, layout: &Layout, part: PartId) -> Node {
        match self {
            StoreEnd::Net(net) => Node::from(net),
            StoreEnd::Internal(k) => layout.internal(part, k),
        }
    }
}

/// One capacitor a part owns. `k` is its index in that part's charge list, and branch
/// `k` carries its current. Capacitors take the part's first branches. An inductor is
/// not this type.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnedCapacitor {
    pub(crate) k: usize,
    pub(crate) ends: [StoreEnd; 2],
    pub(crate) farad: f64,
}

/// The capacitors `model` owns, in index order. A part with none returns an empty list.
pub(crate) fn charges(model: &impl Device) -> Vec<OwnedCapacitor> {
    Device::charges(model)
}

/// One inductor a part owns. `k` is its index in that part's inductor list, and its current is
/// branch `charges + k`, after the part's capacitors. `ohm` is the resistance in series with
/// it, which bounds the current it can carry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnedInductor {
    pub(crate) k: usize,
    pub(crate) ends: [StoreEnd; 2],
    pub(crate) henry: f64,
    pub(crate) ohm: f64,
}

/// The inductors `model` owns, in index order. A part with none returns an empty list.
pub(crate) fn inductors(model: &impl Device) -> Vec<OwnedInductor> {
    Device::inductors(model)
}

pub(crate) fn stamp(model: &impl Device, stamp: &mut Stamp<'_>) {
    Device::stamp(model, stamp);
}

impl Device for Model {
    #[inline]
    fn unknowns(&self) -> Unknowns {
        Model::unknowns(self)
    }

    #[inline]
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Model::charges(self)
    }

    #[inline]
    fn inductors(&self) -> Vec<OwnedInductor> {
        Model::inductors(self)
    }

    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        Model::stamp(self, stamp);
    }
}

/// Whether this part's stamp reads the Newton iterate. A junction reads it through
/// `Stamp::junction`. A linear regulator reads its last branch current and input voltage
/// through `Stamp::prior`. Every other stamp reads only the call's conditions, so on a
/// circuit where no part reads the iterate every iteration of one call stamps the same system.
#[inline]
pub(crate) fn reads_iterate(model: &Model) -> bool {
    model.reads_iterate()
}

/// Start voltages for this part's junction slots.
pub(crate) fn initial(model: &Model, out: &mut [f64]) {
    model.initial(out);
}

/// After a solve: limit this part's junctions from `prior` into `next`, and check their currents.
pub(crate) fn update(model: &Model, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
    model.update(probe, prior, next)
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Update {
    pub(crate) limited: bool,
    /// Worst current mismatch over its tolerance. At most 1 passes.
    pub(crate) mismatch: f64,
}

/// The pin pairs a part joins at DC. A chain of them is a DC path for `net.floating`.
/// A capacitor is open unless the operating point holds it at `initial`. An inductor is a short
/// behind its resistance. A switch is a resistor either way.
/// A current source joins nothing. A BJT joins its base to each other pin, because each junction
/// conducts. An `nmos` joins its drain to its source. Its gate does not. A relay joins its coil's
/// ends, and `com` to `no` and to `nc` in either state.
pub(crate) fn dc_paths(model: &Model) -> &'static [(usize, usize)] {
    model.dc_paths()
}

/// The values `observe` contributes, in `quantities` order.
/// `len` is `model.quantities().len()`. The third slot pads an `nmos`.
pub(crate) struct Observed {
    values: [f64; 3],
    len: usize,
}

impl Observed {
    pub(crate) fn ivp(i: f64, v: f64) -> Self {
        Self {
            values: [i, v, v * i],
            len: 3,
        }
    }

    pub(crate) fn values(values: [f64; 3], len: usize) -> Self {
        Self { values, len }
    }
}

impl IntoIterator for Observed {
    type Item = f64;
    type IntoIter = std::iter::Take<std::array::IntoIter<f64, 3>>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter().take(self.len)
    }
}

/// `model.quantities()` at `at`, in that order. `None` for a part with none.
/// Not checked for finiteness.
pub(crate) fn observe(model: &Model, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed> {
    model.observe(probe, at, side)
}

/// Two capacitors on one part. Tests only. Not a netlist kind. `branches` is the count
/// `unknowns` reserves, so a port that leaves it at 0 fails when the layout is built.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pair {
    pub(crate) ends: [[StoreEnd; 2]; 2],
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

    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }

    fn stamp(&self, stamp: &mut Stamp<'_>) {
        for cap in self.charges() {
            capacitor::stamp_capacitor(stamp, cap.k, cap.ends, cap.farad, None);
        }
    }
}

/// One capacitor and one inductor on one part, so its charge 0 and its flux 0 share a part.
/// Tests only. Not a netlist kind. The capacitor takes branch 0 and the inductor branch 1, and
/// `branches` is the count `unknowns` reserves.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mixed {
    pub(crate) capacitor: [StoreEnd; 2],
    pub(crate) farad: f64,
    pub(crate) inductor: [StoreEnd; 2],
    pub(crate) henry: f64,
    pub(crate) ohm: f64,
    pub(crate) internal_nodes: usize,
    pub(crate) branches: usize,
}

#[cfg(test)]
impl Device for Mixed {
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: self.internal_nodes,
            bare_internal: false,
            branches: self.branches,
            junctions: 0,
        }
    }

    fn charges(&self) -> Vec<OwnedCapacitor> {
        vec![OwnedCapacitor {
            k: 0,
            ends: self.capacitor,
            farad: self.farad,
        }]
    }

    fn inductors(&self) -> Vec<OwnedInductor> {
        vec![OwnedInductor {
            k: 0,
            ends: self.inductor,
            henry: self.henry,
            ohm: self.ohm,
        }]
    }

    fn stamp(&self, stamp: &mut Stamp<'_>) {
        capacitor::stamp_capacitor(stamp, 0, self.capacitor, self.farad, None);
        inductor::stamp_inductor(stamp, 0, self.inductor, self.henry);
    }
}
