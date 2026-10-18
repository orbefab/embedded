use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sim_cli::Completed;
use sim_core::name::Ref;
use sim_core::netlist::{self, Analysis, Model, bounce};
use sim_core::timestamp::Timestamp;

/// What `scripts/ngspice_refs.py` writes into `ngspice.json`.
const NGSPICE: &str = "ngspice-42";
const OPTIONS: &str = "temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12";
/// The deck's ramp at each contact edge and source jump, which is also the engine's first step
/// after one.
const EDGE_RAMP_PS: u64 = 1000;

fn examples() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut found: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("netlist.json").is_file())
        .collect();
    found.sort();
    found
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// `#net.*` and `*.v` are volts. `*.i` is amperes. `*.p` is watts. `#net.i` is a voltage.
fn floor(reference: &str) -> f64 {
    if reference.starts_with("#net.") || reference.ends_with(".v") {
        1e-9
    } else if reference.ends_with(".i") {
        1e-12
    } else if reference.ends_with(".p") {
        1e-15
    } else {
        panic!("{reference} is neither a voltage, a current, nor a power");
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn floor_is_one_nanovolt_one_picoampere_or_one_femtowatt() {
    assert_eq!(floor("#net.vout"), 1e-9);
    assert_eq!(floor("#net.i"), 1e-9);
    assert_eq!(floor("#r1.v"), 1e-9);
    assert_eq!(floor("#led1.i"), 1e-12);
    assert_eq!(floor("#r1.p"), 1e-15);
}

#[test]
fn every_example_matches_its_ngspice_reference_within_a_tenth_of_a_percent() {
    let names: Vec<_> = examples()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "button-led",
            "current-pulse",
            "divider",
            "i-divider",
            "led-5v",
            "led-9v",
            "ne555-led",
            "rc-switch",
            "v-pulse-rc"
        ]
    );
    for example in examples() {
        let label = example.display();
        let reference_path = example.join("ngspice.json");
        assert!(
            reference_path.is_file(),
            "{label} has no ngspice.json; run `uv run scripts/ngspice_refs.py {label}`"
        );
        let reference = json(&reference_path);
        let deck = fs::read(example.join("deck.cir")).unwrap();
        assert_eq!(
            sha256_hex(&deck),
            reference["deck_sha256"].as_str().unwrap(),
            "{label}/deck.cir does not match deck_sha256"
        );
        assert_eq!(reference["format"].as_u64(), Some(0), "{label} format");
        assert_eq!(
            reference["ngspice"].as_str(),
            Some(NGSPICE),
            "{label} ngspice"
        );
        assert_eq!(
            reference["options"].as_str(),
            Some(OPTIONS),
            "{label} options"
        );
        let input = fs::read(example.join("netlist.json")).unwrap();
        let done = sim_cli::run(&input, Timestamp::from_unix_seconds(0));
        let manifest: serde_json::Value =
            serde_json::from_slice(done.folder.file("manifest.json").unwrap()).unwrap();
        assert_eq!(
            manifest["input"]["sha256"], reference["netlist_sha256"],
            "{label}/netlist.json changed; run `uv run scripts/ngspice_refs.py {label}`"
        );
        match (reference.get("op"), reference.get("tran")) {
            (Some(op), None) => check_op(&label.to_string(), op, &done),
            (None, Some(tran)) => check_tran(&label.to_string(), tran, &input, &done),
            _ => panic!("{label}: ngspice.json needs exactly one of op and tran"),
        }
    }
}

fn check_op(label: &str, reference: &serde_json::Value, done: &Completed) {
    let op = done
        .results
        .observations()
        .op
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: no operating point"));
    let wanted = reference.as_object().unwrap();
    assert!(!wanted.is_empty(), "{label}");
    for (key, want) in wanted {
        let want = want.as_f64().unwrap();
        let got = op.values[&key.parse::<Ref>().unwrap()].get();
        let bound = 1e-3 * want.abs() + floor(key);
        assert!(
            (got - want).abs() <= bound,
            "{label} {key}: {got} vs ngspice {want}"
        );
    }
}

/// The script's bounce expansion must equal the engine's, its edges and every source jump must
/// be rows of the trace, its latch edges must pair with the engine's, and each waveform must be
/// within 1e-3 of its span away from every edge and jump.
fn check_tran(label: &str, reference: &serde_json::Value, input: &[u8], done: &Completed) {
    assert!(
        done.results.diagnostics().is_empty(),
        "{label}: {:?}",
        done.results.diagnostics()
    );
    assert_eq!(
        reference["edge_ramp_ps"].as_u64(),
        Some(EDGE_RAMP_PS),
        "{label}"
    );
    let switches = &reference["switches"];
    let observed = serde_json::to_value(&done.results.observations().switches).unwrap();
    assert_eq!(&observed, switches, "{label}: engine edges vs the script's");
    assert_eq!(
        &expanded(input),
        switches,
        "{label}: bounce::expand vs the script's"
    );

    let trace = done
        .results
        .trace()
        .unwrap_or_else(|| panic!("{label}: no trace"));
    let rows: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    let mut edges: Vec<u64> = switches
        .as_object()
        .unwrap()
        .values()
        .flat_map(|list| list.as_array().unwrap())
        .map(|edge| edge["t_ps"].as_u64().unwrap())
        .chain(jumps(input))
        .collect();
    edges.sort_unstable();
    edges.dedup();
    let stop = *rows.last().unwrap();
    for &edge in &edges {
        for at in [edge, ramp_end(edge, stop)] {
            assert!(
                rows.binary_search(&at).is_ok(),
                "{label}: no row at {at} ps"
            );
        }
    }

    let engine_t: Vec<f64> = rows.iter().map(|&t| t as f64).collect();
    let reference_t: Vec<f64> = reference["t_s"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_f64().unwrap() * 1e12)
        .collect();
    let mut windows = ramps(&edges);
    windows.extend(check_logic(label, reference, done));
    let grid = grid(&engine_t, &reference_t, &windows);
    let values = reference["values"].as_object().unwrap();
    let mut compared = SpanCheck {
        failed: false,
        report: Vec::new(),
        checked: BTreeSet::new(),
        engine_t: &engine_t,
        reference_t: &reference_t,
        grid: &grid,
        held: held(input),
    };
    for (key, series) in values {
        let wanted: Vec<f64> = series
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        compared.record(key, column(trace, label, key), &wanted);
    }
    for quantity in trace.columns() {
        let name = quantity.to_string();
        let Some(stem) = name.strip_suffix(".p") else {
            continue;
        };
        let voltage = format!("{stem}.v");
        let current = format!("{stem}.i");
        let wanted_v = series(values, label, &voltage);
        let wanted_i = series(values, label, &current);
        let got_v = column(trace, label, &voltage);
        let got_i = column(trace, label, &current);
        let got_p = column(trace, label, &name);
        for (row, ((&v, &i), &p)) in got_v.iter().zip(got_i).zip(got_p).enumerate() {
            let derived = v * i;
            assert!(
                (p - derived).abs() <= 1e-15,
                "{label} {name} row {row}: stored {p} vs v*i {derived}"
            );
        }
        let wanted: Vec<f64> = wanted_v.iter().zip(&wanted_i).map(|(v, i)| v * i).collect();
        let derived: Vec<f64> = got_v.iter().zip(got_i).map(|(v, i)| v * i).collect();
        compared.record(&name, &derived, &wanted);
    }
    let engine_columns: BTreeSet<_> = trace.columns().iter().map(|r| r.to_string()).collect();
    assert_eq!(
        compared.checked, engine_columns,
        "{label}: compared columns"
    );
    assert!(
        !compared.failed,
        "{label} span error:\n{}",
        compared.report.join("\n")
    );
}

/// Each latch signal ngspice records against the engine's: the same levels in order, the mean
/// period between falling edges within 1e-3, and each edge within 1e-3 of that period of its
/// partner. Returns the window around each pair, from the earlier minus the reference grid to
/// the later plus it, where the linearized waveform still straddles the edge.
fn check_logic(label: &str, reference: &serde_json::Value, done: &Completed) -> Vec<(f64, f64)> {
    let Some(logic) = reference.get("logic") else {
        return Vec::new();
    };
    let grid_ps = reference["grid_ps"]
        .as_u64()
        .unwrap_or_else(|| panic!("{label}: logic needs grid_ps"));
    let observed = serde_json::to_value(&done.results.observations().logic).unwrap();
    let mut windows = Vec::new();
    for (key, list) in logic.as_object().unwrap() {
        let oracle = logic_edges(list);
        let engine = logic_edges(&observed[key]["edges"]);
        let levels = |edges: &[(u64, String)]| -> Vec<String> {
            edges.iter().map(|(_, level)| level.clone()).collect()
        };
        assert_eq!(levels(&engine), levels(&oracle), "{label} {key}: levels");
        let (got, want) = (mean_period(&engine), mean_period(&oracle));
        assert!(
            (got / want - 1.0).abs() <= 1e-3,
            "{label} {key}: period {got} ps vs ngspice {want} ps"
        );
        for ((a, _), (b, _)) in engine.iter().zip(&oracle) {
            assert!(
                a.abs_diff(*b) as f64 <= 1e-3 * want,
                "{label} {key}: edge at {a} ps vs ngspice {b} ps"
            );
            windows.push((
                a.min(b).saturating_sub(grid_ps) as f64,
                a.max(b).saturating_add(grid_ps) as f64,
            ));
        }
    }
    windows
}

fn logic_edges(list: &serde_json::Value) -> Vec<(u64, String)> {
    list.as_array()
        .unwrap_or_else(|| panic!("{list} is not a list of edges"))
        .iter()
        .map(|edge| {
            let at = edge["t_ps"].as_u64().unwrap();
            (at, edge["level"].as_str().unwrap().to_string())
        })
        .collect()
}

/// The mean time between falling edges, the last minus the first over their count less one.
fn mean_period(edges: &[(u64, String)]) -> f64 {
    let falls: Vec<u64> = edges
        .iter()
        .filter(|(_, level)| level == "low")
        .map(|&(at, _)| at)
        .collect();
    assert!(falls.len() >= 2, "{edges:?} has under two falling edges");
    (falls[falls.len() - 1] - falls[0]) as f64 / (falls.len() - 1) as f64
}

struct SpanCheck<'a> {
    failed: bool,
    report: Vec<String>,
    checked: BTreeSet<String>,
    engine_t: &'a [f64],
    reference_t: &'a [f64],
    grid: &'a [f64],
    /// Columns whose t = 0 sample is dropped from both series.
    held: BTreeSet<String>,
}

impl SpanCheck<'_> {
    fn record(&mut self, key: &str, got: &[f64], wanted: &[f64]) {
        // Both series and the grid start at t = 0. Dropping that sample, rather than only not
        // scoring at t = 0, keeps ngspice's 0 out of the chord the engine's next rows are read
        // against. Scoring starts at the reference's next sample, since nothing before it says
        // what ngspice's current was.
        let from = usize::from(self.held.contains(key));
        let (engine_t, got) = (&self.engine_t[from..], &got[from..]);
        let (reference_t, wanted) = (&self.reference_t[from..], &wanted[from..]);
        let start = reference_t[0];
        let span = wanted.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - wanted.iter().copied().fold(f64::INFINITY, f64::min);
        let error = self.grid[from..]
            .iter()
            .filter(|&&t| t >= start)
            .map(|&t| (interpolate(engine_t, got, t) - interpolate(reference_t, wanted, t)).abs())
            .fold(0.0, f64::max);
        self.failed |= error > 1e-3 * span + floor(key);
        self.report
            .push(format!("{key} error {error:e} span {span:e}"));
        self.checked.insert(key.to_string());
    }
}

fn column<'a>(trace: &'a sim_core::trace::Trace, label: &str, key: &str) -> &'a [f64] {
    let index = trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == key)
        .unwrap_or_else(|| panic!("{label}: no column {key}"));
    trace.column(index)
}

fn series(values: &serde_json::Map<String, serde_json::Value>, label: &str, key: &str) -> Vec<f64> {
    values
        .get(key)
        .unwrap_or_else(|| panic!("{label}: no reference {key}"))
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect()
}

/// The current and power of every capacitor with an `initial` voltage. ngspice solves
/// `.ic` without `uic` and reports their current as 0 at t = 0, where the engine reports
/// the current the hold takes. So their t = 0 sample is dropped from both series.
fn held(input: &[u8]) -> BTreeSet<String> {
    let netlist = netlist::parse(input).unwrap();
    netlist
        .parts()
        .filter(|(_, part)| matches!(&part.model, Model::Capacitor(c) if c.initial.is_some()))
        .flat_map(|(_, part)| ["i", "p"].map(|q| format!("#{}.{q}", part.name.as_str())))
        .collect()
}

/// Every contact edge before the stop, from `bounce::expand` on the parsed netlist.
fn expanded(input: &[u8]) -> serde_json::Value {
    let netlist = netlist::parse(input).unwrap();
    let Analysis::Transient(transient) = netlist.analysis() else {
        panic!("not a transient");
    };
    let mut switches = serde_json::Map::new();
    for (_, part) in netlist.parts() {
        if let Model::Switch(switch) = &part.model {
            let edges: Vec<_> = bounce::expand(
                netlist.seed(),
                &part.name,
                &switch.events,
                switch.bounce.as_ref(),
            )
            .into_iter()
            .filter(|edge| edge.at < transient.stop)
            .collect();
            switches.insert(
                format!("#{}", part.name.as_str()),
                serde_json::to_value(edges).unwrap(),
            );
        }
    }
    serde_json::Value::Object(switches)
}

/// Every source jump before the stop, from the parsed netlist. The deck ramps each one like a
/// contact edge.
fn jumps(input: &[u8]) -> Vec<u64> {
    let netlist = netlist::parse(input).unwrap();
    let Analysis::Transient(transient) = netlist.analysis() else {
        panic!("not a transient");
    };
    netlist
        .parts()
        .filter_map(|(_, part)| part.model.waveform())
        .flat_map(|waveform| waveform.jumps(transient.stop, usize::MAX))
        .map(|at| at.as_ps())
        .collect()
}

/// The required row after an edge. It holds the new contact when it is after the edge.
fn ramp_end(edge: u64, stop: u64) -> u64 {
    edge.saturating_add(EDGE_RAMP_PS).min(stop)
}

/// The open interval `(edge, edge + 1 ns)`. The deck is still ramping there when the end
/// is past the stop, so that stop is not a comparable sample.
fn ramps(edges: &[u64]) -> Vec<(f64, f64)> {
    edges
        .iter()
        .map(|&edge| (edge as f64, edge.saturating_add(EDGE_RAMP_PS) as f64))
        .collect()
}

/// Both time grids, within the engine's run, without the inside of each window.
fn grid(engine: &[f64], reference: &[f64], windows: &[(f64, f64)]) -> Vec<f64> {
    let end = *engine.last().unwrap();
    let mut grid: Vec<f64> = engine
        .iter()
        .chain(reference)
        .copied()
        .filter(|&t| t <= end)
        .collect();
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    grid.retain(|&t| !windows.iter().any(|&(from, to)| t > from && t < to));
    grid
}

/// `values` read as piecewise linear in `t`, held flat past either end.
fn interpolate(t: &[f64], values: &[f64], at: f64) -> f64 {
    let k = t.partition_point(|&x| x <= at);
    if k == 0 {
        return values[0];
    }
    if k == t.len() {
        return values[k - 1];
    }
    let (t0, t1, v0, v1) = (t[k - 1], t[k], values[k - 1], values[k]);
    v0 + (v1 - v0) * (at - t0) / (t1 - t0)
}

#[test]
fn the_grid_skips_the_inside_of_each_window_and_keeps_its_ends() {
    let engine = [0.0, 1000.0, 2000.0, 3000.0];
    let reference = [500.0, 1500.0, 2999.5, 4000.0];
    assert_eq!(
        grid(&engine, &reference, &ramps(&[1000])),
        [0.0, 500.0, 1000.0, 2000.0, 2999.5, 3000.0]
    );
    assert_eq!(
        grid(&engine, &reference, &[(400.0, 1500.0), (2000.0, 3000.0)]),
        [0.0, 1500.0, 2000.0, 3000.0]
    );
}

#[test]
fn the_grid_drops_stop_when_the_ramp_runs_past_it() {
    // 9900 + 1 ns is 10900, past the stop. The open interval includes the stop, which the
    // deck has not finished ramping.
    let grid = grid(
        &[0.0, 9900.0, 10_000.0],
        &[9950.0, 10_000.0, 11_000.0],
        &ramps(&[9900]),
    );
    assert_eq!(grid, [0.0, 9900.0]);
}

#[test]
fn the_grid_drops_a_stop_500_ps_after_a_jump() {
    // A 0 to 1 step at 9500 ps with stop 10000. The deck ramps until 10500 ps.
    let open = grid(
        &[0.0, 9_500.0, 10_000.0],
        &[9_800.0, 10_000.0, 10_500.0],
        &ramps(&[9_500]),
    );
    assert_eq!(open, [0.0, 9_500.0]);
    // A ramp that finishes on the stop keeps the stop. The interval is open at that end.
    let finished = grid(
        &[0.0, 9_000.0, 10_000.0],
        &[9_500.0, 10_000.0],
        &ramps(&[9_000]),
    );
    assert_eq!(finished, [0.0, 9_000.0, 10_000.0]);
}

#[test]
fn a_held_capacitor_drops_only_the_t_0_sample_of_its_current_and_power() {
    let input = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "transient": { "stop_ps": 10000, "max_step_ps": 1000 } },
      "ground": "gnd",
      "nets": ["gnd", "vout"],
      "parts": [
        {
          "name": "c1",
          "kind": "capacitor",
          "pins": { "a": "vout", "b": "gnd" },
          "capacitance": { "value": 1e-6, "unit": "F", "source": "user" },
          "initial": { "value": 5.0, "unit": "V", "source": "user" }
        },
        {
          "name": "c2",
          "kind": "capacitor",
          "pins": { "a": "vout", "b": "gnd" },
          "capacitance": { "value": 1e-6, "unit": "F", "source": "user" },
          "initial": null
        },
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vout", "b": "gnd" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let held = held(input);
    assert_eq!(
        held,
        BTreeSet::from(["#c1.i".to_string(), "#c1.p".to_string()])
    );
    // A 1 ms discharge from 5 mA. ngspice reports 0 at t = 0 and samples next at 50 ns.
    // The engine reports the hold current at t = 0 and its next row is at 1 ns.
    let current = |ps: f64| -5e-3 * (-ps * 1e-12 / 1e-3).exp();
    let engine_t = [0.0, 1e3, 5e4, 1e9, 5e9];
    let reference_t = [0.0, 5e4, 1e9, 5e9];
    let engine = engine_t.map(current);
    let ngspice = [0.0, current(5e4), current(1e9), current(5e9)];
    let grid = grid(&engine_t, &reference_t, &[]);
    let mut compared = SpanCheck {
        failed: false,
        report: Vec::new(),
        checked: BTreeSet::new(),
        engine_t: &engine_t,
        reference_t: &reference_t,
        grid: &grid,
        held,
    };
    compared.record("#c1.i", &engine, &ngspice);
    assert!(!compared.failed, "{:?}", compared.report);
    let mut early = engine;
    early[1] = 0.0;
    compared.record("#c1.i", &early, &ngspice);
    assert!(!compared.failed, "{:?}", compared.report);
    compared.record("#c2.i", &engine, &ngspice);
    assert!(compared.failed, "{:?}", compared.report);
    compared.failed = false;
    let mut late = engine;
    late[2] *= 1.01;
    compared.record("#c1.i", &late, &ngspice);
    assert!(compared.failed, "{:?}", compared.report);
}

#[test]
fn interpolation_is_linear_between_samples_and_flat_outside() {
    let t = [0.0, 10.0, 20.0];
    let values = [1.0, 3.0, -1.0];
    assert_eq!(interpolate(&t, &values, 5.0), 2.0);
    assert_eq!(interpolate(&t, &values, 10.0), 3.0);
    assert_eq!(interpolate(&t, &values, 15.0), 1.0);
    assert_eq!(interpolate(&t, &values, 25.0), -1.0);
}
