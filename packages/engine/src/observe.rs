use orbe_model::Time;

use crate::netlist::{NetId, Netlist};

use crate::diagnostic::Diagnostic;
use crate::finite::Finite;
use crate::name::Ref;

use crate::run_folder::OpPoint;

use crate::mna::{Probe, Solution};
use crate::newton::{Circuit, Side};

/// Every observed ref in `Ref` order: each net, then each part's `quantities`, then each world
/// quantity the netlist declares. Fixed by the netlist. The `op` keys, trace columns and rating
/// watches all index it.
pub(crate) struct Columns {
    refs: Vec<Ref>,
    /// The nets in the order of their columns.
    nets: Vec<NetId>,
}

impl Columns {
    pub(crate) fn new(netlist: &Netlist) -> Columns {
        // Ground comes first in the netlist, but `Ref` order sorts every net by name.
        let mut nets: Vec<_> = netlist.nets().collect();
        nets.sort_by(|left, right| left.1.cmp(right.1));
        let mut refs: Vec<Ref> = nets
            .iter()
            .map(|(_, name)| Ref::Net((*name).clone()))
            .collect();
        for (_, part) in netlist.parts() {
            for &quantity in part.model.quantities() {
                refs.push(Ref::Quantity(part.name.clone(), quantity));
            }
        }
        refs.extend(netlist.world().declared().map(|(q, _)| Ref::World(q)));
        debug_assert!(refs.is_sorted());
        Columns {
            refs,
            nets: nets.into_iter().map(|(id, _)| id).collect(),
        }
    }

    pub(crate) fn refs(&self) -> &[Ref] {
        &self.refs
    }

    pub(crate) fn index(&self, reference: &Ref) -> Option<usize> {
        self.refs.binary_search(reference).ok()
    }
}

/// One solution as a row aligned with `columns`. A part quantity that is not
/// finite is `solver.nonfinite` at `at`, naming the part.
pub(crate) fn observe(
    circuit: &Circuit,
    columns: &Columns,
    x: &Solution,
    at: Time,
    side: Side,
) -> Result<Vec<Finite>, Diagnostic> {
    row(circuit, columns, x, at, side)
        .map_err(|unknown| Diagnostic::SolverNonFinite { at, unknown })
}

/// `observe`, with the part whose quantity is not finite as the error.
pub(crate) fn row(
    circuit: &Circuit,
    columns: &Columns,
    x: &Solution,
    at: Time,
    side: Side,
) -> Result<Vec<Finite>, Ref> {
    let mut row = Vec::with_capacity(columns.refs.len());
    for &net in &columns.nets {
        row.push(Finite::new(x.voltage(net)).expect("solution components are finite"));
    }
    for (id, part) in circuit.netlist.parts() {
        let probe = Probe {
            layout: &circuit.layout,
            part: id,
            x,
        };
        for value in part.model.observe(&probe, at, side).into_iter().flatten() {
            row.push(Finite::new(value).ok_or_else(|| Ref::Part(part.name.clone()))?);
        }
    }
    let world = circuit.netlist.world();
    for (_, schedule) in world.declared() {
        row.push(side.read(schedule, at).si);
    }
    Ok(row)
}

pub(crate) fn op_point(columns: &Columns, row: &[Finite]) -> OpPoint {
    OpPoint {
        values: columns
            .refs()
            .iter()
            .cloned()
            .zip(row.iter().copied())
            .collect(),
    }
}
