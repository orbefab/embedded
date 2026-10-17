mod dense;
mod element;
mod ladder;
mod mna;
mod newton;
mod op;
mod ratings;
mod topology;

use sim_core::netlist::{Analysis, Netlist};
use sim_core::run_folder::{Observations, Results};

/// Simulate a validated netlist.
/// A circuit the solver cannot handle comes back as an error diagnostic.
pub fn simulate(netlist: &Netlist) -> Results {
    let mut diagnostics = topology::floating_nets(netlist);
    let mut observations = Observations::default();
    match netlist.analysis() {
        Analysis::Op => observations.op = op::run(netlist, &mut diagnostics),
    }
    Results::new(observations, diagnostics)
}
