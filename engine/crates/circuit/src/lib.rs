mod active;
mod chip;
mod dense;
mod element;
mod ladder;
mod lte;
mod mna;
mod newton;
mod observe;
mod op;
mod ratings;
mod step;
mod topology;
mod tran;
mod watch;

use sim_core::netlist::{Analysis, Netlist};
use sim_core::run_folder::{Observations, Results};

use crate::tran::Rows;

/// Simulate a validated netlist. A transient writes its rows on the grid it sets.
/// A circuit the solver cannot handle comes back as an error diagnostic.
pub fn simulate(netlist: &Netlist) -> Results {
    simulate_with(netlist, Rows::Grid)
}

/// Tests only: the same steps as `simulate`, with a row at every accepted step. The solver's
/// own tests read every step through this. `Cargo.toml` turns the feature on through a
/// dev-dependency on this crate, so no build of `sim` has it.
#[cfg(feature = "step-rows")]
pub fn simulate_steps(netlist: &Netlist) -> Results {
    simulate_with(netlist, Rows::Steps)
}

fn simulate_with(netlist: &Netlist, rows: Rows) -> Results {
    let mut diagnostics = topology::floating_nets(netlist);
    match netlist.analysis() {
        Analysis::Op => {
            let observations = match op::run(netlist, &mut diagnostics) {
                Some(op) => Observations::Op(op),
                None => Observations::Nothing,
            };
            Results::new(observations, None, diagnostics)
        }
        Analysis::Transient(transient) => {
            let outcome = tran::run(netlist, transient, rows, &mut diagnostics);
            Results::new(outcome.observations, outcome.trace, diagnostics)
        }
    }
}
