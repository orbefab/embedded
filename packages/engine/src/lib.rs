//! The simulator: its data contracts and the solver that runs them.
//! `Junction::VT` is the one thermal voltage, shared by the value rule and the solver.

use crate::netlist::Analysis;
use crate::netlist::Netlist;

pub mod blob;
pub mod diagnostic;
pub mod display;
pub mod finite;
pub mod i2c;
pub mod leds;
pub mod link;
pub mod logic;
pub mod name;
pub mod netlist;
pub mod run_folder;
pub mod serial;
pub mod timestamp;
pub mod trace;

mod active;
pub mod chip;
pub mod device;
mod element;
mod inputs;
mod ladder;
mod lte;
pub mod mcu;
mod mna;
mod newton;
mod observe;
mod op;
mod output_stage;
pub mod passive;
mod ratings;
#[cfg(feature = "runner")]
pub mod runner;
pub mod semiconductor;
mod session;
mod sparse;
mod step;
mod topology;
mod tran;
mod watch;
pub mod world;

use crate::blob::Blobs;

use crate::run_folder::{Observations, Results, RunFolder};
use crate::timestamp::Timestamp;

use crate::tran::Rows;

pub use crate::ratings::SupplyMark;
pub use crate::serial::SerialByte;
pub use session::{
    Applied, DeviceEvent, Display, EdgeCursor, Input, Leds, Probe, Refused, Rejected, Serial,
    SerialCursor, Session, Setting, Status, WorldApplied, WorldChange,
};
pub use topology::floating_nets;

/// What a run's steps and solves cost, counted exactly. No number is written into the run
/// folder, and counting changes no result.
///
/// t = 0 is the operating point and is not a landing. A stop is counted on the accepted step
/// that reaches it, and two chips stopped on one landing count as two stops.
///
/// A try is one step attempt from the last accepted step. It is accepted as a landing, rejected
/// for its local truncation error, or stalls in Newton and is tried again at half the step, or
/// once at 1 ps with backward Euler. A stall is not a rejection.
///
/// A Newton iteration is one stamped system solved. `iterations` counts those on the run's
/// solver, and is the sum of the `*_iterations` before `row_iterations` once a run reaches its
/// stop. A row of the grid that no step lands on is solved on a copy of that solver and is
/// counted apart, in `row_probes` and `row_iterations`. `iterations`, the kinds of solve and
/// the two lists of parts are read from the run when it ends.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunStats {
    pub landings: u64,
    pub stops: u64,
    /// Tries rejected for their error.
    pub rejections: u64,
    /// Tries whose Newton solve stalled, `euler_retries` among them.
    pub stalls: u64,
    /// Stalled 1 ps tries tried again with backward Euler.
    pub euler_retries: u64,
    /// Landings whose step the plan set: the error of the step before, a segment's first step
    /// or a stall's half.
    pub set_by_plan: u64,
    /// Landings at the transient's `max_step`.
    pub set_by_max_step: u64,
    /// Landings cut to a breakpoint: a contact's or a source's edge, a potentiometer move, a
    /// world change, 1 ns after any of those or t = 0, or `stop`.
    pub set_by_breakpoint: u64,
    /// Landings cut to an active chip's core's next stop.
    pub set_by_chip: u64,
    /// Landings a watch's threshold crossing cut short.
    pub set_by_watch: u64,
    /// Solves after a landing whose chips, sources or contacts changed.
    pub restarts: u64,
    pub iterations: u64,
    /// The operating point at t = 0.
    pub start_iterations: u64,
    /// The tries that landed.
    pub landing_iterations: u64,
    /// The tries rejected for their error.
    pub rejected_iterations: u64,
    /// The tries whose solve failed: every stall, and a singular or overflowing try that ends
    /// the run.
    pub failed_iterations: u64,
    /// Locating where a watch crossed its threshold inside a try.
    pub locate_iterations: u64,
    pub restart_iterations: u64,
    /// Restarts that started from the change the same image change made before, and those
    /// that converged from there.
    pub guesses: u64,
    pub guessed: u64,
    /// Grid rows solved inside a step, and their iterations.
    pub row_probes: u64,
    pub row_iterations: u64,
    /// How each of `iterations` solved its system: the memo of a linear circuit held it, it
    /// was refactored on kept pivots, factored again with pivoting because a kept pivot failed,
    /// or factored on a fresh column order.
    pub memo_hits: u64,
    pub refactors: u64,
    pub repivots: u64,
    pub reorders: u64,
    /// The parts whose capacitance or inductance had the largest error on the most landings
    /// whose error set the next step, most first, at most five, with those counts.
    pub pacing: Vec<(String, u64)>,
    /// The parts whose capacitance or inductance had the largest error on the most rejected
    /// tries, most first, at most five, with those counts.
    pub rejecting: Vec<(String, u64)>,
}

/// What `orbe-run run` makes of netlist.json bytes: the results and the folder they render to.
pub struct Completed {
    pub results: Results,
    pub folder: RunFolder,
}

/// netlist.json bytes to a run folder, as `orbe-run run` writes it. Never fails. A rejected
/// netlist is a folder whose diagnostics say why.
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

/// [`simulate`] plus what its steps and solves cost. The `Results` are the same bytes.
pub fn simulate_stats(netlist: &Netlist) -> (Results, RunStats) {
    simulate_with(netlist, Rows::Grid)
}

/// Tests only: the same steps as `simulate`, with a row at every accepted step. The solver's
/// own tests read every step through this. `Cargo.toml` turns the feature on through a
/// dev-dependency on this crate, so no build of `orbe-run` has it.
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
