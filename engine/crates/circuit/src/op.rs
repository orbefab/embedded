use std::collections::BTreeMap;

use sim_core::diagnostic::Diagnostic;
use sim_core::finite::Finite;
use sim_core::name::Ref;
use sim_core::netlist::Netlist;
use sim_core::run_folder::OpPoint;
use sim_core::time::Time;

use crate::element;
use crate::mna::{GMIN, Layout, Probe, Stamp, System};

pub(crate) fn solve(netlist: &Netlist) -> Result<OpPoint, Diagnostic> {
    let layout = Layout::new(netlist);
    if layout.size() > crate::dense::MAX_UNKNOWNS {
        return Err(Diagnostic::SolverTooLarge {
            at: Time::ZERO,
            unknowns: layout.size(),
            limit: crate::dense::MAX_UNKNOWNS,
        });
    }
    let mut system = System::new(&layout, GMIN);
    for (id, part) in netlist.parts() {
        element::stamp(
            &part.model,
            &mut Stamp {
                sys: &mut system,
                layout: &layout,
                part: id,
            },
        );
    }
    let solution = system
        .solve()
        .map_err(|singular| Diagnostic::SolverSingular {
            at: Time::ZERO,
            unknown: layout.describe(netlist, singular.row),
        })?;
    if let Some(row) = solution
        .components()
        .iter()
        .position(|value| !value.is_finite())
    {
        return Err(Diagnostic::SolverNonFinite {
            at: Time::ZERO,
            unknown: layout.describe(netlist, row),
        });
    }

    let mut values = BTreeMap::new();
    for (id, name) in netlist.nets() {
        let finite = Finite::new(solution.voltage(id)).expect("solution components are finite");
        values.insert(Ref::Net(name.clone()), finite);
    }
    for (id, part) in netlist.parts() {
        let probe = Probe {
            layout: &layout,
            part: id,
            x: &solution,
        };
        element::observe(&part.model, &part.name, &probe, &mut values).map_err(|_| {
            Diagnostic::SolverNonFinite {
                at: Time::ZERO,
                unknown: Ref::Part(part.name.clone()),
            }
        })?;
    }
    Ok(OpPoint { values })
}
