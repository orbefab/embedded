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
/// The deck's control ramp at each contact edge, which is also the engine's first step after one.
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
        ["button-led", "divider", "led-5v", "led-9v", "rc-switch"]
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

/// The script's bounce expansion must equal the engine's, its edges must be
/// rows of the trace, and each waveform must be within 1e-3 of its span.
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
        .collect();
    edges.sort_unstable();
    assert!(!edges.is_empty(), "{label}");
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
    let grid = grid(&engine_t, &reference_t, &edges);
    let values = reference["values"].as_object().unwrap();
    let mut compared = SpanCheck {
        failed: false,
        report: Vec::new(),
        checked: BTreeSet::new(),
        engine_t: &engine_t,
        reference_t: &reference_t,
        grid: &grid,
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

struct SpanCheck<'a> {
    failed: bool,
    report: Vec<String>,
    checked: BTreeSet<String>,
    engine_t: &'a [f64],
    reference_t: &'a [f64],
    grid: &'a [f64],
}

impl SpanCheck<'_> {
    fn record(&mut self, key: &str, got: &[f64], wanted: &[f64]) {
        let span = wanted.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - wanted.iter().copied().fold(f64::INFINITY, f64::min);
        let error = self
            .grid
            .iter()
            .map(|&t| {
                (interpolate(self.engine_t, got, t) - interpolate(self.reference_t, wanted, t))
                    .abs()
            })
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

/// The later endpoint of an edge's ramp. The row there holds the new contact.
fn ramp_end(edge: u64, stop: u64) -> u64 {
    edge.saturating_add(EDGE_RAMP_PS).min(stop)
}

/// Both time grids, within the engine's run, without the open ramp after each edge.
/// The trace stores only the ramp's endpoints, and ngspice is mid-ramp inside it.
fn grid(engine: &[f64], reference: &[f64], edges: &[u64]) -> Vec<f64> {
    let end = *engine.last().unwrap();
    let mut grid: Vec<f64> = engine
        .iter()
        .chain(reference)
        .copied()
        .filter(|&t| t <= end)
        .collect();
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    grid.retain(|&t| {
        !edges.iter().any(|&edge| {
            let closed = ramp_end(edge, end as u64) as f64;
            t > edge as f64 && t < closed
        })
    });
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
fn the_grid_skips_the_inside_of_each_ramp_and_keeps_its_ends() {
    let grid = grid(
        &[0.0, 1000.0, 2000.0, 3000.0],
        &[500.0, 1500.0, 2999.5, 4000.0],
        &[1000],
    );
    assert_eq!(grid, [0.0, 500.0, 1000.0, 2000.0, 2999.5, 3000.0]);
}

#[test]
fn the_grid_keeps_stop_when_the_ramp_runs_into_it() {
    let grid = grid(
        &[0.0, 9900.0, 10_000.0],
        &[9950.0, 10_000.0, 11_000.0],
        &[9900],
    );
    assert_eq!(grid, [0.0, 9900.0, 10_000.0]);
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
