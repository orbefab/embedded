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
mod session;
mod step;
mod topology;
mod tran;
mod watch;

use sim_core::netlist::{Analysis, Netlist};
use sim_core::run_folder::{Observations, Results};

use crate::tran::Rows;

pub use session::{Probe, Refused, Session, Status};

/// Accepted steps after t = 0, and chip stops on those steps, summed over chips.
///
/// t = 0 is the operating point and is not a landing. A stop is counted on the accepted
/// step that reaches it. Two chips stopped on one landing count as two stops. Neither
/// number is written into the run folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunStats {
    pub landings: u64,
    pub stops: u64,
}

/// Simulate a validated netlist. A transient writes its rows on the grid it sets.
/// A circuit the solver cannot handle comes back as an error diagnostic.
pub fn simulate(netlist: &Netlist) -> Results {
    simulate_stats(netlist).0
}

/// [`simulate`] plus the landing and stop counts. The `Results` are the same bytes.
pub fn simulate_stats(netlist: &Netlist) -> (Results, RunStats) {
    simulate_with(netlist, Rows::Grid)
}

/// Tests only: the same steps as `simulate`, with a row at every accepted step. The solver's
/// own tests read every step through this. `Cargo.toml` turns the feature on through a
/// dev-dependency on this crate, so no build of `sim` has it.
#[cfg(feature = "step-rows")]
pub fn simulate_steps(netlist: &Netlist) -> Results {
    simulate_with(netlist, Rows::Steps).0
}

fn simulate_with(netlist: &Netlist, rows: Rows) -> (Results, RunStats) {
    let mut diagnostics = topology::floating_nets(netlist);
    match netlist.analysis() {
        Analysis::Op => {
            let observations = match op::run(netlist, &mut diagnostics) {
                Some(op) => Observations::Op(op),
                None => Observations::Nothing,
            };
            (
                Results::new(observations, None, diagnostics),
                RunStats::default(),
            )
        }
        Analysis::Transient(transient) => {
            let outcome = tran::run(netlist, transient, rows, &mut diagnostics);
            (
                Results::new(outcome.observations, outcome.trace, diagnostics),
                outcome.stats,
            )
        }
    }
}
