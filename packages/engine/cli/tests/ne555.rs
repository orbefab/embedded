//! The A3 astable, packages/engine/tests/circuits/ne555-led, against SLFS022K's formula and against the closed form
//! of the behaviour the engine models.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{read_vcd, summary};
use orbe_engine_cli::Completed;
use orbe_engine_core::logic::Level;
use orbe_engine_core::run_folder::Results;
use orbe_engine_core::timestamp::Timestamp;

/// The engine's conductance from every node to ground.
const GMIN: f64 = 1e-12;

fn netlist() -> serde_json::Value {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/circuits/ne555-led/netlist.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn run(netlist: &serde_json::Value) -> Completed {
    let done = orbe_engine_cli::run(
        &serde_json::to_vec(netlist).unwrap(),
        Timestamp::from_unix_seconds(0),
    );
    assert!(
        done.results.diagnostics().is_empty(),
        "{:?}",
        done.results.diagnostics()
    );
    done
}

/// Like `run`, with a row at every accepted step, for the tests that read the trace at an edge.
fn run_steps(netlist: &serde_json::Value) -> Results {
    let netlist = orbe_engine_core::netlist::parse(&serde_json::to_vec(netlist).unwrap()).unwrap();
    let results = orbe_circuit::simulate_steps(&netlist);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    results
}

fn part<'a>(netlist: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == name)
        .unwrap()
}

fn value(field: &serde_json::Value) -> f64 {
    field["value"].as_f64().unwrap()
}

/// The timing network, and the RC solution of each half period. TRIG and THRES are tied to the
/// capacitor, CONT holds 2/3 of the supply and the lower ladder node 1/3.
struct Astable {
    supply: f64,
    ra: f64,
    rb: f64,
    c: f64,
    on: f64,
    off: f64,
}

impl Astable {
    fn of(netlist: &serde_json::Value) -> Astable {
        let chip = part(netlist, "u1");
        Astable {
            supply: value(&part(netlist, "v1")["voltage"]),
            ra: value(&part(netlist, "ra")["resistance"]),
            rb: value(&part(netlist, "rb")["resistance"]),
            c: value(&part(netlist, "c1")["capacitance"]),
            on: value(&chip["discharge"]["on_resistance"]),
            off: value(&chip["discharge"]["off_resistance"]),
        }
    }

    fn upper(&self) -> f64 {
        2.0 * self.supply / 3.0
    }

    fn lower(&self) -> f64 {
        self.supply / 3.0
    }

    /// The capacitor's asymptote and time constant while DISCH has `discharge` to GND.
    fn phase(&self, discharge: f64) -> (f64, f64) {
        let asymptote = self.supply * discharge / (self.ra + discharge);
        let resistance = self.rb + self.ra * discharge / (self.ra + discharge);
        (asymptote, resistance * self.c)
    }

    /// OUT high, DISCH off. The capacitor charges from `from` to `to`.
    fn high(&self, from: f64, to: f64) -> f64 {
        let (asymptote, tau) = self.phase(self.off);
        tau * libm::log((asymptote - from) / (asymptote - to))
    }

    /// OUT low, DISCH on. The capacitor discharges from the upper to the lower threshold.
    fn low(&self) -> f64 {
        let (asymptote, tau) = self.phase(self.on);
        tau * libm::log((self.upper() - asymptote) / (self.lower() - asymptote))
    }

    fn period(&self) -> f64 {
        self.high(self.lower(), self.upper()) + self.low()
    }
}

/// `(t_ps, level)` of every `#u1.logic.out` edge.
fn out_edges(results: &Results) -> Vec<(u64, Level)> {
    let logic = results.trace().unwrap().logic().unwrap();
    let out = logic
        .signals()
        .iter()
        .position(|signal| signal.to_string() == "#u1.logic.out")
        .unwrap();
    logic
        .changes()
        .iter()
        .filter(|change| change.signal == out)
        .map(|change| (change.at.as_ps(), change.level))
        .collect()
}

/// The mean time between falling edges, in seconds.
fn steady_period(edges: &[(u64, Level)]) -> f64 {
    let falls: Vec<u64> = edges
        .iter()
        .filter(|(_, level)| *level == Level::Low)
        .map(|&(at, _)| at)
        .collect();
    assert!(falls.len() >= 2, "{edges:?}");
    (falls[falls.len() - 1] - falls[0]) as f64 * 1e-12 / (falls.len() - 1) as f64
}

fn column<'a>(results: &'a Results, key: &str) -> &'a [f64] {
    let trace = results.trace().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == key)
        .unwrap_or_else(|| panic!("no column {key}"));
    trace.column(index)
}

fn rows(results: &Results) -> Vec<u64> {
    results
        .trace()
        .unwrap()
        .time()
        .iter()
        .map(|t| t.as_ps())
        .collect()
}

#[test]
fn the_period_is_within_the_formula_budget_and_its_own_closed_form() {
    let netlist = netlist();
    let astable = Astable::of(&netlist);
    let edges = out_edges(&run(&netlist).results);
    let period = steady_period(&edges);
    let Astable { ra, rb, c, .. } = astable;

    // SLFS022K eq. 3 and 4, and what it leaves out that the model has, each as a share of it.
    let formula = (ra + 2.0 * rb) * c / 1.44;
    let ln2 = std::f64::consts::LN_2;
    let rounding = (ra + 2.0 * rb) * c * ln2 / formula - 1.0;
    let (asymptote, tau) = astable.phase(astable.on);
    let ratio = libm::log((astable.upper() - asymptote) / (astable.lower() - asymptote));
    let discharge_asymptote = (rb * c * ratio - rb * c * ln2) / formula;
    let discharge_resistance = (tau - rb * c) * ratio / formula;
    let leakage = (astable.high(astable.lower(), astable.upper()) - (ra + rb) * c * ln2) / formula;
    let terms = [rounding, discharge_asymptote, discharge_resistance, leakage];
    let bound: f64 = terms.iter().map(|term| term.abs()).sum();
    let from_formula = period / formula - 1.0;
    assert!(
        from_formula.abs() <= bound,
        "{period} s is {from_formula:e} from the formula; the budget {terms:?} sums to {bound:e}"
    );

    let closed = period / astable.period() - 1.0;
    assert!(
        closed.abs() <= 1e-4,
        "{period} s is {closed:e} from the closed form"
    );
    let first = edges[0].0 as f64 * 1e-12;
    let first_closed = astable.high(0.0, astable.upper());
    assert_eq!(edges[0].1, Level::Low);
    assert!(
        (first / first_closed - 1.0).abs() <= 1e-4,
        "first high {first} s vs {first_closed} s"
    );
}

/// For a fall, THRES above CONT. For a rise, TRIG at or under the lower ladder node, which is
/// CONT over the two lower resistors and gmin.
fn past(ladder: f64, level: Level, thr: f64, cont: f64) -> f64 {
    match level {
        Level::Low => thr - cont,
        Level::High => cont / (2.0 + GMIN * ladder) - thr,
    }
}

#[test]
fn every_output_edge_is_a_located_crossing_and_never_a_step_late() {
    let netlist = netlist();
    let results = run_steps(&netlist);
    let ladder = value(&part(&netlist, "u1")["ladder_resistance"]);
    let (time, thr, cont) = (
        rows(&results),
        column(&results, "#net.thr"),
        column(&results, "#net.cont"),
    );
    let edges = out_edges(&results);
    assert_eq!(edges.len(), 10);
    for (at, level) in edges {
        let row = time
            .binary_search(&at)
            .unwrap_or_else(|_| panic!("no row at the edge at {at} ps"));
        let gap = past(ladder, level, thr[row], cont[row]);
        assert!(
            gap > 0.0 && gap <= 1e-8,
            "{level:?} at {at} ps: {gap} V past"
        );
        let before = past(ladder, level, thr[row - 1], cont[row - 1]);
        assert!(
            before < 0.0,
            "{level:?} at {at} ps: the row before is {before} V past"
        );
    }
}

/// How far each OUT edge of a run can be from the closed form's, in ps. In each half period,
/// the run's THRES differs from the closed form started at the run's own last edge by at most
/// the largest difference on any row. That difference, plus how far the edge row sits past the
/// nominal threshold, over the closed form's slope there, bounds the half period's error. Edge
/// `n` carries the sum over the half periods before it.
fn edge_bounds(results: &Results, astable: &Astable) -> Vec<f64> {
    let (time, thr) = (rows(results), column(results, "#net.thr"));
    let mut start = (0, 0.0);
    let mut sum = 0.0;
    let mut bounds = Vec::new();
    for (at, level) in out_edges(results) {
        let (discharge, threshold) = match level {
            Level::Low => (astable.off, astable.upper()),
            Level::High => (astable.on, astable.lower()),
        };
        let (asymptote, tau) = astable.phase(discharge);
        let (from, v0) = start;
        let closed =
            |t: u64| asymptote + (v0 - asymptote) * libm::exp(-((t - from) as f64) * 1e-12 / tau);
        let first = time.partition_point(|&t| t < from);
        let last = time.binary_search(&at).unwrap();
        let error = (first..=last)
            .map(|row| (thr[row] - closed(time[row])).abs())
            .fold(0.0, f64::max);
        let slope = (asymptote - threshold).abs() / tau;
        sum += (error + (thr[last] - threshold).abs()) / slope * 1e12;
        bounds.push(sum);
        start = (at, threshold);
    }
    bounds
}

#[test]
fn the_edges_do_not_move_with_max_step_beyond_the_measured_trajectory_error() {
    let committed = netlist();
    let astable = Astable::of(&committed);
    let max_step = committed["analysis"]["transient"]["max_step_ps"]
        .as_u64()
        .unwrap();
    let runs: Vec<_> = [max_step / 10, max_step, max_step * 10]
        .into_iter()
        .map(|step| {
            let mut netlist = committed.clone();
            netlist["analysis"]["transient"]["max_step_ps"] = step.into();
            let results = run_steps(&netlist);
            (step, out_edges(&results), edge_bounds(&results, &astable))
        })
        .collect();
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        let ((step_a, edges_a, bounds_a), (step_b, edges_b, bounds_b)) = (&runs[a], &runs[b]);
        let levels = |edges: &[(u64, Level)]| -> Vec<Level> { edges.iter().map(|e| e.1).collect() };
        assert_eq!(
            levels(edges_a),
            levels(edges_b),
            "{step_a} ps vs {step_b} ps"
        );
        for n in 0..edges_a.len() {
            let moved = edges_a[n].0.abs_diff(edges_b[n].0) as f64;
            let bound = bounds_a[n] + bounds_b[n];
            assert!(
                moved <= bound,
                "edge {n}: {moved} ps between {step_a} ps and {step_b} ps steps, bound {bound} ps"
            );
            // A step-end edge would be late by up to a step. The bound must be finer than that.
            assert!(bound < (max_step / 10) as f64, "edge {n}: bound {bound} ps");
        }
    }
}

#[test]
fn logic_vcd_reads_back_as_the_summary_and_two_runs_write_the_same_bytes() {
    let netlist = netlist();
    let first = run(&netlist);
    let vcd = first.folder.file("traces/logic.vcd").unwrap();
    let observations: serde_json::Value =
        serde_json::from_slice(first.folder.file("observations.json").unwrap()).unwrap();
    let edges = read_vcd(std::str::from_utf8(vcd).unwrap());
    assert_eq!(summary(&edges), observations["logic"]);
    assert_eq!(
        edges["#u1.logic.out"]["edges"].as_array().unwrap().len(),
        10
    );
    let signals: Vec<_> = observations["logic"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        signals,
        [
            "#u1.logic.out",
            "#u1.logic.reset",
            "#u1.logic.thres",
            "#u1.logic.trig"
        ]
    );
    let second = orbe_engine_cli::run(
        &serde_json::to_vec(&netlist).unwrap(),
        Timestamp::from_unix_seconds(1),
    );
    assert_eq!(second.folder.file("traces/logic.vcd").unwrap(), vcd);
}
