use crate::name::Quantity;
use crate::netlist::{NetId, PartId};

use crate::mna::{Layout, Node, Probe, Stamp, Unknowns};
pub(crate) use crate::passive::inductor::{coil, stamp_coil};

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
/// `len` is `model.quantities().len()`. The third slot pads a MOSFET.
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
