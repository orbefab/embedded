mod dense;
mod element;
mod ladder;
mod mna;
mod newton;
mod observe;
mod op;
mod ratings;
mod step;
mod topology;
mod tran;

use sim_core::netlist::{Analysis, Netlist};
use sim_core::run_folder::{Observations, Results};

/// Simulate a validated netlist.
/// A circuit the solver cannot handle comes back as an error diagnostic.
pub fn simulate(netlist: &Netlist) -> Results {
    let mut diagnostics = topology::floating_nets(netlist);
    match netlist.analysis() {
        Analysis::Op => {
            let observations = Observations {
                op: op::run(netlist, &mut diagnostics),
                switches: None,
            };
            Results::new(observations, None, diagnostics)
        }
        Analysis::Transient(transient) => {
            let outcome = tran::run(netlist, transient, &mut diagnostics);
            Results::new(outcome.observations, outcome.trace, diagnostics)
        }
    }
}
