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

use sim_core::blob::Blobs;
use sim_core::netlist::{self, Analysis, Netlist};
use sim_core::run_folder::{self, Observations, Results, RunFolder};
use sim_core::timestamp::Timestamp;

use crate::tran::Rows;

pub use active::st77916;
pub use session::{
    Applied, Display, EdgeCursor, Input, Leds, Probe, Refused, Rejected, Serial, SerialCursor,
    Session, Setting, Status,
};
pub use sim_core::serial::SerialByte;

/// Accepted steps after t = 0, chip stops on those steps, and stiffen retries.
///
/// t = 0 is the operating point and is not a landing. A stop is counted on the accepted
/// step that reaches it. Two chips stopped on one landing count as two stops. A stiffen
/// is a step retried with a store on backward Euler, a failed try included. None of these
/// numbers is written into the run folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunStats {
    pub landings: u64,
    pub stops: u64,
    pub stiffens: u64,
}

/// What `sim run` makes of netlist.json bytes: the results and the folder they render to.
pub struct Completed {
    pub results: Results,
    pub folder: RunFolder,
}

/// netlist.json bytes to a run folder, as `sim run` writes it. Never fails. A rejected netlist is
/// a folder whose diagnostics say why.
pub fn run(input: &[u8], created_at: Timestamp) -> Completed {
    run_with(input, &Blobs::new(), created_at)
}

/// [`run`], finding each blob the netlist names in `blobs`.
pub fn run_with(input: &[u8], blobs: &Blobs, created_at: Timestamp) -> Completed {
    let (netlist, results) = match netlist::parse_with(input, blobs) {
        Ok(n) => {
            let r = simulate(&n);
            (Some(n), r)
        }
        Err(d) => (None, Results::rejected(d)),
    };
    let folder = run_folder::render(input, netlist.as_ref(), &results, created_at);
    Completed { results, folder }
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
    match netlist.analysis() {
        Analysis::Op => {
            let mut diagnostics = topology::floating_nets(netlist);
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
            let mut diagnostics = Vec::new();
            let outcome = tran::run(netlist, transient, rows, &mut diagnostics);
            (
                Results::new(outcome.observations, outcome.trace, diagnostics),
                outcome.stats,
            )
        }
    }
}
