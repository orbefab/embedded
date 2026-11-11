use crate::name::Quantity;
use crate::netlist::{NetId, PartId};

use crate::mna::{Layout, Node, Probe, Stamp, Unknowns};
pub(crate) use crate::passive::inductor::{coil, stamp_coil};
#[cfg(test)]
use crate::passive::{capacitor, inductor};

/// What the layout and the stamp read from a part. A netlist `Model` is one. A test
/// passes its own value through the same calls.
pub(crate) trait Device {
    fn unknowns(&self) -> Unknowns;
    fn charges(&self) -> Vec<OwnedCapacitor>;
    fn inductors(&self) -> Vec<OwnedInductor>;
    fn stamp(&self, stamp: &mut Stamp<'_>);
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

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Update {
    pub(crate) limited: bool,
    /// Worst current mismatch over its tolerance. At most 1 passes.
    pub(crate) mismatch: f64,
}

/// The quantities a two-pin part has.
pub(crate) const TWO_PIN: &[Quantity] = &[Quantity::I, Quantity::V, Quantity::P];

/// The values `observe` contributes, in `quantities` order.
/// `len` is `model.quantities().len()`. The third slot pads an `nmos`.
pub(crate) struct Observed {
    values: [f64; 3],
    len: usize,
}

impl Observed {
    /// [`TWO_PIN`] for current `i` from `pins[0]` to `pins[1]`.
    pub(crate) fn two_pin(pins: [NetId; 2], i: f64, probe: &Probe<'_>) -> Self {
        let v = probe.v(pins[0]) - probe.v(pins[1]);
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
