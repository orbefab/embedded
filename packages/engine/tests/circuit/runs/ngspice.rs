use orbe_model::Time;

use orbe_engine::netlist::{self, Analysis, Model, Waveform, bounce};

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::runs::common;
use common::examples::{examples, runs_a_core};
use orbe_engine::Completed;
use orbe_engine::logic::{Level, Logic};
use orbe_engine::name::Ref;

use orbe_engine::timestamp::Timestamp;
use sha2::{Digest, Sha256};

/// What `packages/engine/scripts/ngspice_refs.py` writes into `ngspice.json`.
const NGSPICE: &str = "ngspice-42";
const OPTIONS: &str = "temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12";
/// A transient deck with an inductor or a relay, which crawls at the reltol above.
const INDUCTOR_OPTIONS: &str = "temp=25 tnom=25 reltol=1e-5 abstol=1e-15 vntol=1e-12 gmin=1e-12";
/// The deck's ramp at each contact edge and source jump, which is also the engine's first step
/// after one.
const EDGE_RAMP_PS: u64 = 1000;

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// `#net.*` and `*.v` are volts. `*.i`, `*.ib`, `*.ic`, `*.id` and `*.ig` are amperes. `*.p` is
/// watts. `#net.i` is a voltage.
fn floor(reference: &str) -> f64 {
    if reference.starts_with("#net.") || reference.ends_with(".v") {
        1e-9
    } else if [".i", ".ib", ".ic", ".id", ".ig", ".iin", ".iout"]
        .iter()
        .any(|end| reference.ends_with(end))
    {
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
    assert_eq!(floor("#q1.ib"), 1e-12);
    assert_eq!(floor("#q1.ic"), 1e-12);
    assert_eq!(floor("#m1.id"), 1e-12);
    assert_eq!(floor("#m1.ig"), 1e-12);
    assert_eq!(floor("#u1.iin"), 1e-12);
    assert_eq!(floor("#u1.iout"), 1e-12);
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
            "atmega-analog-serial",
            "atmega-blink",
            "atmega-board-blink",
            "atmega-board-button",
            "atmega-board-knob",
            "atmega-button",
            "atmega-cst816",
            "atmega-dht22",
            "atmega-fade",
            "atmega-i2c-scan",
            "atmega-serial-echo",
            "atmega-ws2812",
            "button-led",
            "ce-amp",
            "current-pulse",
            "diode-led-two-caps",
            "divider",
            "esp32s3-adc",
            "esp32s3-blink",
            "esp32s3-dualcore",
            "esp32s3-echo",
            "esp32s3-encoder",
            "esp32s3-hello",
            "esp32s3-pattern",
            "esp32s3-pulls",
            "esp32s3-ring",
            "esp32s3-screen",
            "esp32s3-touch",
            "esp32s3-uart",
            "i-divider",
            "inductor-open-end",
            "led-5v",
            "led-9v",
            "ne555-led",
            "nmos-led",
            "nmos-power-switch",
            "nmos-sweep",
            "npn-blinker",
            "npn-led",
            "npn-pulse",
            "npn-sweep",
            "op-amp-bias",
            "op-amp-gain-10k",
            "op-amp-gain-200k",
            "op-amp-hysteresis",
            "op-amp-ramp",
            "op-amp-short",
            "op-amp-slew",
            "op-amp-square",
            "op-amp-swing",
            "pnp-load",
            "pnp-sweep",
            "rc-picofarad",
            "rc-switch",
            "regulator-3v3",
            "regulator-dropout",
            "regulator-load-step",
            "relay-flyback",
            "relay-switch",
            "rl-step",
            "spi-replay",
            "st77916-replay",
            "v-pulse-rc",
            "v-serial",
            "v-steps",
            "ws2812-replay",
            "zener-shunt",
            "zener-sweep"
        ]
    );
    for example in examples() {
        matches_its_reference(&example);
    }
}

/// Circuits only this test runs. `diode-off` holds a diode with its anode at 5 V and its
/// cathode at 8 V. `led-off` holds an LED with both pins at 5 V. `diode-reverse` and
/// `led-reverse` hold a diode at 0.3 V and an LED at 0.5 V reverse, below `-3 * n * VT`, where
/// ngspice's reverse law leaves the exponential. `npn-off` and `pnp-off` hold
/// the leakage on each kind's internal nodes. `npn-held-off` and `pnp-held-off` hold the
/// base-emitter junction 0.13 V reverse and the collector 12 V away, and `npn-reverse` and
/// `pnp-reverse` hold the base-collector junction 0.13 V reverse. `npn-nf-nr` and `pnp-nf-nr`
/// set `NF` to 1.5 and `NR` to 2, with the emitter 0.13 V from the base and the collector
/// 0.27 V from it. Each transistor there has an `IS` of 1e-8 A, so the reverse law shows
/// above the floor. `nmos-off` holds an off MOSFET
/// so its drain current is the drain-source gmin. `npn-slow-edge` drives the junction
/// capacitors through 1 Mohm, and `nmos-pulse` does the same for the gate capacitances, slow
/// next to the deck's ramp. `nmos-regions` biases the drain through reverse, linear and saturation.
/// `nmos-follower` holds the source off ground. `zener-forward`, `zener-reverse`,
/// `zener-breakdown` and `zener-overload` hold the fitted zener forward, below the knee,
/// at its test current, and past both ratings. `regulator-off-cap` holds the input at 0 V
/// with only a capacitor on the output. `regulator-c100u` is the load step with 100 uF
/// on the output. `regulator-step-4v` steps the input to 4 V and sets `grid_ps` to 20 us.
/// `regulator-rdo-1` uses a 1 ohm dropout resistance and a 0.1 V dropout voltage.
/// `regulator-ground` draws 0.5 A and sets `ground_current_ratio` to 0.02. `i2c-rc-bus`
/// switches a START, two bits and a STOP onto 4.7 kohm pull-ups and 200 pF per line. The
/// other `i2c-` fixtures run firmware, so they have no reference, and `i2c.rs` checks them.
/// `p-relay-npn-bare` has a `rows-skipped` file. `relay.rs` checks its collector envelope
/// and its release. The rows are not compared.
#[test]
fn every_fixture_matches_its_ngspice_reference_within_a_tenth_of_a_percent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/circuit/fixtures");
    let mut fixtures: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    fixtures.sort();
    let names: Vec<_> = fixtures
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "diode-off",
            "diode-reverse",
            "i2c-arbitration",
            "i2c-contend",
            "i2c-cst816-fast",
            "i2c-rc-bus",
            "i2c-read-ack",
            "i2c-scan-400k",
            "i2c-stretch",
            "led-off",
            "led-reverse",
            "nmos-follower",
            "nmos-off",
            "nmos-pulse",
            "nmos-regions",
            "npn-held-off",
            "npn-nf-nr",
            "npn-off",
            "npn-reverse",
            "npn-slow-edge",
            "p-relay-npn-bare",
            "pnp-held-off",
            "pnp-nf-nr",
            "pnp-off",
            "pnp-reverse",
            "regulator-backfeed",
            "regulator-c100u",
            "regulator-ground",
            "regulator-hot",
            "regulator-limit",
            "regulator-off",
            "regulator-off-cap",
            "regulator-open",
            "regulator-rdo-1",
            "regulator-sag",
            "regulator-short",
            "regulator-step-4v",
            "zener-breakdown",
            "zener-forward",
            "zener-overload",
            "zener-reverse"
        ]
    );
    for fixture in fixtures {
        if common::skips_reference_rows(&fixture) {
            continue;
        }
        matches_its_reference(&fixture);
    }
}

/// `example` holds netlist.json, and deck.cir and ngspice.json unless it runs a core.
fn matches_its_reference(example: &Path) {
    let label = example.display();
    let reference_path = example.join("ngspice.json");
    let input = fs::read(example.join("netlist.json")).unwrap();
    if example.file_name().unwrap() == "npn-blinker" {
        assert!(
            !reference_path.exists() && !example.join("deck.cir").exists(),
            "{label} has no reference, because ngspice aborts its deck at the deck's options with Timestep too small"
        );
        return;
    }
    let parsed = common::examples::parse(example);
    if (parsed.parts()).any(|(_, part)| matches!(part.model, Model::OpAmp(_))) {
        assert!(
            !reference_path.exists() && !example.join("deck.cir").exists(),
            "{label} has an op_amp, which ngspice has no kind for, so it has no reference; runs/op_amp.rs checks it"
        );
        return;
    }
    if runs_a_core(&parsed) {
        assert!(
            !reference_path.exists() && !example.join("deck.cir").exists(),
            "{label} runs an active chip's core, which a deck has none of, so it has no reference"
        );
        return;
    }
    assert!(
        reference_path.is_file(),
        "{label} has no ngspice.json; run `uv run packages/engine/scripts/ngspice_refs.py {label}`"
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
        Some(options(&netlist::parse(&input).unwrap())),
        "{label} options"
    );
    let done = orbe_engine::run(&input, Timestamp::from_unix_seconds(0));
    let manifest: serde_json::Value =
        serde_json::from_slice(done.folder.file("manifest.json").unwrap()).unwrap();
    assert_eq!(
        manifest["input"]["sha256"], reference["netlist_sha256"],
        "{label}/netlist.json changed; run `uv run packages/engine/scripts/ngspice_refs.py {label}`"
    );
    match (reference.get("op"), reference.get("tran")) {
        (Some(op), None) => check_op(&label.to_string(), op, &done),
        (None, Some(tran)) => check_tran(&label.to_string(), tran, &input, &done),
        _ => panic!("{label}: ngspice.json needs exactly one of op and tran"),
    }
}

fn options(netlist: &netlist::Netlist) -> &'static str {
    let coil = netlist
        .parts()
        .any(|(_, part)| matches!(part.model, Model::Inductor(_) | Model::Relay(_)));
    match netlist.analysis() {
        Analysis::Transient(_) if coil => INDUCTOR_OPTIONS,
        _ => OPTIONS,
    }
}

fn check_op(label: &str, reference: &serde_json::Value, done: &Completed) {
    let op = done
        .results
        .observations()
        .op()
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

/// The script's bounce expansion must equal the engine's, its latch edges must pair with the
/// engine's, and each waveform must be within 1e-3 of its span at every engine row away from
/// every edge and jump.
fn check_tran(label: &str, reference: &serde_json::Value, input: &[u8], done: &Completed) {
    let found: Vec<(String, String)> = done
        .results
        .diagnostics()
        .iter()
        .map(|d| {
            let refs = d.refs();
            assert_eq!(refs.len(), 1, "{label}: {d:?}");
            (d.code().to_string(), refs[0].to_string())
        })
        .collect();
    if label.contains("regulator-load-step")
        || label.contains("regulator-c100u")
        || label.contains("regulator-step-4v")
        || label.contains("regulator-rdo-1")
    {
        assert_eq!(
            found,
            vec![
                ("rating.current".to_string(), "#u1.iout".to_string()),
                ("rating.power".to_string(), "#u1.p".to_string()),
            ],
            "{label}"
        );
    } else {
        assert!(found.is_empty(), "{label}: {found:?}");
    }
    assert_eq!(
        reference["edge_ramp_ps"].as_u64(),
        Some(EDGE_RAMP_PS),
        "{label}"
    );
    let switches = &reference["switches"];
    let observed = serde_json::to_value(done.results.observations().switches()).unwrap();
    assert_eq!(&observed, switches, "{label}: engine edges vs the script's");
    assert_eq!(
        &expanded(input),
        switches,
        "{label}: bounce::expand vs the script's"
    );
    let scripted: Vec<u64> = reference["jumps"]
        .as_array()
        .unwrap_or_else(|| panic!("{label}: tran has no jumps"))
        .iter()
        .map(|time| {
            time.as_u64()
                .unwrap_or_else(|| panic!("{label}: a jump is not an integer"))
        })
        .collect();
    assert_eq!(
        jumps(input),
        scripted,
        "{label}: engine jumps vs the script's"
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

    let engine_t: Vec<f64> = rows.iter().map(|&t| t as f64).collect();
    let reference_t: Vec<f64> = reference["t_s"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_f64().unwrap() * 1e12)
        .collect();
    let mut windows = ramps(&edges);
    windows.extend(check_logic(label, reference, done));
    let grid = grid(&engine_t, &windows);
    let values = reference["values"].as_object().unwrap();
    let mut compared = SpanCheck {
        failed: false,
        report: Vec::new(),
        checked: BTreeSet::new(),
        engine_t: &engine_t,
        reference_t: &reference_t,
        grid: &grid,
        held: held(input),
        spans: reference
            .get("span")
            .map(|spans| spans.as_object().unwrap()),
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
    let netlist = netlist::parse(input).unwrap();
    let gate = reference.get("gate").and_then(|value| value.as_object());
    let mut gates = BTreeSet::new();
    for (_, part) in netlist.parts() {
        let name = format!("#{}.p", part.name.as_str());
        if !trace
            .columns()
            .iter()
            .any(|column| column.to_string() == name)
        {
            continue;
        }
        let stem = format!("#{}", part.name.as_str());
        let ig = gate.and_then(|table| table.get(&stem).map(|series| series_of(label, series)));
        let got = |key: &str| column(trace, label, key).to_vec();
        let wanted = |key: &str| series(values, label, key);
        let (got_p, wanted_p) = power(&netlist, &part.model, &name, got, wanted, ig.as_deref());
        let stored = column(trace, label, &name);
        // An nmos pin formula needs the gate current, which is not an engine column. The
        // circuit test `pin_power_includes_the_gate_current` pins that identity. Here the
        // stored column is what the span check reads.
        if !matches!(part.model, Model::Nmos(_)) {
            for (row, (&p, &derived)) in stored.iter().zip(&got_p).enumerate() {
                assert!(
                    (p - derived).abs() <= 1e-15,
                    "{label} {name} row {row}: stored {p} vs the pin formula {derived}"
                );
            }
        }
        compared.record(&name, &got_p, &wanted_p);
        if let (Model::Nmos(_), Some(ig)) = (&part.model, &ig) {
            let pins = part.model.pins();
            let [d, g, s] = [0, 1, 2].map(|k| {
                column(
                    trace,
                    label,
                    &format!("#net.{}", netlist.net_name(pins[k]).as_str()),
                )
            });
            let id = column(trace, label, &format!("{stem}.id"));
            let (got_ig, rows) = gate_current(&engine_t, &grid, stored, [d, g, s], id);
            let key = format!("{stem}.ig");
            compared.record_on(&key, &got_ig, ig, &rows);
            gates.insert(key);
        }
    }
    let engine_columns: BTreeSet<_> = trace.columns().iter().map(|r| r.to_string()).collect();
    assert_eq!(
        compared.checked,
        &engine_columns | &gates,
        "{label}: compared columns"
    );
    if let Some(spans) = compared.spans {
        let stored: BTreeSet<_> = spans.keys().cloned().collect();
        assert_eq!(stored, compared.checked, "{label}: stored spans");
    }
    eprintln!("{label} span error:");
    for line in &compared.report {
        eprintln!("{line}");
    }
    assert!(
        !compared.failed,
        "{label} span error:\n{}",
        compared.report.join("\n")
    );
}

/// Each logic signal ngspice records against the engine's, with the same levels in order. A
/// latch's mean period between falling edges is within 1e-3, and each edge is within 1e-3 of
/// that period of its partner. A relay contact's edge is within `contact_bound` of its partner.
/// Returns the window around each pair, from the earlier minus the reference grid to the later
/// plus it, where the reference still straddles the edge.
fn check_logic(label: &str, reference: &serde_json::Value, done: &Completed) -> Vec<(f64, f64)> {
    let Some(logic) = reference.get("logic") else {
        return Vec::new();
    };
    let grid_ps = reference["grid_ps"]
        .as_u64()
        .unwrap_or_else(|| panic!("{label}: logic needs grid_ps"));
    let observed = done.results.trace().unwrap().logic().unwrap();
    let mut windows = Vec::new();
    for (key, list) in logic.as_object().unwrap() {
        let oracle = logic_edges(list);
        let engine = engine_edges(observed, key);
        let levels = |edges: &[(u64, String)]| -> Vec<String> {
            edges.iter().map(|(_, level)| level.clone()).collect()
        };
        assert_eq!(levels(&engine), levels(&oracle), "{label} {key}: levels");
        let bounds: Vec<f64> = match key.strip_suffix(".logic.contact") {
            Some(part) => oracle
                .iter()
                .map(|&(at, _)| contact_bound(label, reference, &format!("{part}.i"), at))
                .collect(),
            None => {
                let (got, want) = (mean_period(&engine), mean_period(&oracle));
                assert!(
                    (got / want - 1.0).abs() <= 1e-3,
                    "{label} {key}: period {got} ps vs ngspice {want} ps"
                );
                vec![1e-3 * want; oracle.len()]
            }
        };
        for (((a, _), (b, _)), bound) in engine.iter().zip(&oracle).zip(bounds) {
            assert!(
                a.abs_diff(*b) as f64 <= bound,
                "{label} {key}: edge at {a} ps vs ngspice {b} ps, bound {bound} ps"
            );
            windows.push((
                a.min(b).saturating_sub(grid_ps) as f64,
                a.max(b).saturating_add(grid_ps) as f64,
            ));
        }
    }
    windows
}

/// Picoseconds the coil current `coil` takes to move 1e-3 of its span plus its floor, at
/// ngspice's slope over the reference interval that holds the edge at `at`. The value bound,
/// read as time.
fn contact_bound(label: &str, reference: &serde_json::Value, coil: &str, at: u64) -> f64 {
    let times = series_of(label, &reference["t_s"]);
    let t: Vec<f64> = times.iter().map(|time| time * 1e12).collect();
    let i = series(reference["values"].as_object().unwrap(), label, coil);
    let span = match reference.get("span") {
        Some(spans) => spans[coil].as_f64().unwrap(),
        None => {
            i.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - i.iter().copied().fold(f64::INFINITY, f64::min)
        }
    };
    let k = t.partition_point(|&x| x <= at as f64).clamp(1, t.len() - 1);
    common::coil_edge_bound_ps(span, &times[k - 1..k + 1], &i[k - 1..k + 1])
}

/// A coil current that crosses a threshold and comes back inside one reference interval can
/// leave a flat chord. An infinite bound would let any engine edge pass.
#[test]
#[should_panic(expected = "flat coil current")]
fn contact_bound_rejects_a_flat_coil_current() {
    let reference = serde_json::json!({
        "t_s": [0.0, 1e-6],
        "values": { "#k1.i": [0.05, 0.05] },
    });
    contact_bound("flat", &reference, "#k1.i", 500_000);
}

/// `(t_ps, level)` of every change of `key` the engine recorded.
fn engine_edges(logic: &Logic, key: &str) -> Vec<(u64, String)> {
    let signal = logic
        .signals()
        .iter()
        .position(|signal| signal.to_string() == key)
        .unwrap_or_else(|| panic!("the engine has no {key}"));
    logic
        .changes()
        .iter()
        .filter(|change| change.signal == signal)
        .map(|change| {
            let level = match change.level {
                Level::Low => "low",
                Level::High => "high",
            };
            (change.at.as_ps(), level.to_string())
        })
        .collect()
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
    /// Each column's span over ngspice's own points, when the reference stores the engine's rows.
    spans: Option<&'a serde_json::Map<String, serde_json::Value>>,
}

impl SpanCheck<'_> {
    fn record(&mut self, key: &str, got: &[f64], wanted: &[f64]) {
        self.record_on(key, got, wanted, self.grid);
    }

    fn record_on(&mut self, key: &str, got: &[f64], wanted: &[f64], grid: &[f64]) {
        // Both series and the grid start at t = 0. Dropping that sample, rather than only not
        // scoring at t = 0, keeps ngspice's 0 out of the chord the engine's next rows are read
        // against. Scoring starts at the reference's next sample, since nothing before it says
        // what ngspice's current was.
        let from = usize::from(self.held.contains(key));
        let (engine_t, got) = (&self.engine_t[from..], &got[from..]);
        let (reference_t, wanted) = (&self.reference_t[from..], &wanted[from..]);
        let start = reference_t[0];
        let span = match self.spans {
            Some(spans) => spans
                .get(key)
                .and_then(|span| span.as_f64())
                .unwrap_or_else(|| panic!("no stored span for {key}")),
            None => {
                wanted.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                    - wanted.iter().copied().fold(f64::INFINITY, f64::min)
            }
        };
        let error = grid[from..]
            .iter()
            .filter(|&&t| t >= start)
            .map(|&t| (interpolate(engine_t, got, t) - interpolate(reference_t, wanted, t)).abs())
            .fold(0.0, f64::max);
        self.failed |= error > 1e-3 * span + floor(key);
        self.report.push(format!("{key} error {error} span {span}"));
        self.checked.insert(key.to_string());
    }
}

/// A part's power on every row from its other columns, read once through `got` and once
/// through `wanted`. A two-pin part's is `v * i`. A bipolar transistor's is
/// `(V(c) - V(e)) * ic + (V(b) - V(e)) * ib`, from its pin nets. An nmos engine series is
/// the stored `.p` column. Its ngspice series is `(V(d) - V(s)) * id + (V(g) - V(s)) * ig`,
/// with `ig` from the gate ammeter in `tran.gate`.
fn power(
    netlist: &netlist::Netlist,
    model: &Model,
    name: &str,
    got: impl Fn(&str) -> Vec<f64>,
    wanted: impl Fn(&str) -> Vec<f64>,
    gate: Option<&[f64]>,
) -> (Vec<f64>, Vec<f64>) {
    let stem = name.strip_suffix(".p").unwrap();
    let derive = |read: &dyn Fn(&str) -> Vec<f64>| -> Vec<f64> {
        match model {
            Model::Npn(_) | Model::Pnp(_) => {
                let [c, b, e] = [0, 1, 2].map(|k| {
                    let net = netlist.net_name(model.pins()[k]);
                    read(&format!("#net.{}", net.as_str()))
                });
                let (ib, ic) = (read(&format!("{stem}.ib")), read(&format!("{stem}.ic")));
                (0..ic.len())
                    .map(|row| (c[row] - e[row]) * ic[row] + (b[row] - e[row]) * ib[row])
                    .collect()
            }
            Model::LinearRegulator(_) => {
                let [vin, vout, gnd] = [0, 1, 2].map(|k| {
                    let net = netlist.net_name(model.pins()[k]);
                    read(&format!("#net.{}", net.as_str()))
                });
                let iin = read(&format!("{stem}.iin"));
                let iout = read(&format!("{stem}.iout"));
                (0..iin.len())
                    .map(|row| {
                        (vin[row] - gnd[row]) * iin[row] + (vout[row] - gnd[row]) * iout[row]
                    })
                    .collect()
            }
            Model::Nmos(_) => {
                let [d, g, s] = [0, 1, 2].map(|k| {
                    let net = netlist.net_name(model.pins()[k]);
                    read(&format!("#net.{}", net.as_str()))
                });
                let id = read(&format!("{stem}.id"));
                let ig = gate.unwrap_or(&[]);
                assert_eq!(ig.len(), id.len(), "{stem} gate current");
                (0..id.len())
                    .map(|row| (d[row] - s[row]) * id[row] + (g[row] - s[row]) * ig[row])
                    .collect()
            }
            _ => {
                let (v, i) = (read(&format!("{stem}.v")), read(&format!("{stem}.i")));
                v.iter().zip(&i).map(|(v, i)| v * i).collect()
            }
        }
    };
    match model {
        Model::Nmos(_) => (got(name), derive(&wanted)),
        _ => (derive(&got), derive(&wanted)),
    }
}

/// The engine's gate current on every row, `(p - (V(d) - V(s)) * id) / (V(g) - V(s))` from its
/// stored pin power, and the rows of `grid` where it is scored. A row with the gate within 1 mV of
/// the source does not fix the current. There the series holds 0 and the row is not scored.
fn gate_current(
    engine_t: &[f64],
    grid: &[f64],
    p: &[f64],
    [d, g, s]: [&[f64]; 3],
    id: &[f64],
) -> (Vec<f64>, Vec<f64>) {
    let fixed = |row: usize| (g[row] - s[row]).abs() >= 1e-3;
    let current = (0..p.len())
        .map(|row| {
            if fixed(row) {
                (p[row] - (d[row] - s[row]) * id[row]) / (g[row] - s[row])
            } else {
                0.0
            }
        })
        .collect();
    let rows = grid
        .iter()
        .copied()
        .filter(|&t| fixed(engine_t.partition_point(|&x| x < t)))
        .collect();
    (current, rows)
}

fn series_of(label: &str, series: &serde_json::Value) -> Vec<f64> {
    series
        .as_array()
        .unwrap_or_else(|| panic!("{label}: gate current is not an array"))
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect()
}

fn column<'a>(trace: &'a orbe_engine::trace::Trace, label: &str, key: &str) -> &'a [f64] {
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
                switch.events.events(),
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
    let mut jumps = Vec::new();
    for (_, part) in netlist.parts() {
        let wave = match &part.model {
            Model::VoltageSource(source) => &source.voltage,
            Model::CurrentSource(source) => &source.current,
            _ => continue,
        };
        jumps.extend(jumps_before(wave, transient.stop));
    }
    jumps
}

fn jumps_before(wave: &Waveform, stop: Time) -> Vec<u64> {
    std::iter::successors(wave.next_change_after(Time::ZERO), |&at| {
        wave.next_change_after(at)
    })
    .take_while(|&at| at < stop)
    .map(Time::as_ps)
    .collect()
}

/// The open interval `(edge, edge + 1 ns)`. The deck is still ramping there when the end
/// is past the stop, so that stop is not a comparable sample.
fn ramps(edges: &[u64]) -> Vec<(f64, f64)> {
    edges
        .iter()
        .map(|&edge| (edge as f64, edge.saturating_add(EDGE_RAMP_PS) as f64))
        .collect()
}

/// The engine's rows, without the inside of each window.
fn grid(engine: &[f64], windows: &[(f64, f64)]) -> Vec<f64> {
    let mut grid = engine.to_vec();
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
    let engine = [0.0, 1000.0, 1500.0, 2000.0, 3000.0];
    assert_eq!(
        grid(&engine, &ramps(&[1000])),
        [0.0, 1000.0, 2000.0, 3000.0]
    );
    assert_eq!(
        grid(&engine, &[(400.0, 1500.0), (2000.0, 3000.0)]),
        [0.0, 1500.0, 2000.0, 3000.0]
    );
}

#[test]
fn the_grid_drops_stop_when_the_ramp_runs_past_it() {
    // 9900 + 1 ns is 10900, past the stop. The open interval includes the stop, which the
    // deck has not finished ramping.
    let grid = grid(&[0.0, 9900.0, 10_000.0], &ramps(&[9900]));
    assert_eq!(grid, [0.0, 9900.0]);
}

#[test]
fn the_grid_drops_a_stop_500_ps_after_a_jump() {
    // A 0 to 1 step at 9500 ps with stop 10000. The deck ramps until 10500 ps.
    let open = grid(&[0.0, 9_500.0, 10_000.0], &ramps(&[9_500]));
    assert_eq!(open, [0.0, 9_500.0]);
    // A ramp that finishes on the stop keeps the stop. The interval is open at that end.
    let finished = grid(&[0.0, 9_000.0, 10_000.0], &ramps(&[9_000]));
    assert_eq!(finished, [0.0, 9_000.0, 10_000.0]);
}

#[test]
fn a_held_capacitor_drops_only_the_t_0_sample_of_its_current_and_power() {
    let input = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "transient": { "stop_ps": 10000, "max_step_ps": 1000, "grid_ps": 1000 } },
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
    let current = |ps: f64| -5e-3 * libm::exp(-ps * 1e-12 / 1e-3);
    let engine_t = [0.0, 1e3, 5e4, 1e9, 5e9];
    let reference_t = [0.0, 5e4, 1e9, 5e9];
    let engine = engine_t.map(current);
    let ngspice = [0.0, current(5e4), current(1e9), current(5e9)];
    let grid = grid(&engine_t, &[]);
    let mut compared = SpanCheck {
        failed: false,
        report: Vec::new(),
        checked: BTreeSet::new(),
        engine_t: &engine_t,
        reference_t: &reference_t,
        grid: &grid,
        held,
        spans: None,
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
fn a_stored_span_sets_the_bound_in_place_of_the_rows() {
    // ngspice spiked to 1 A between the rows. The stored rows hold 0, so their own span is 0.
    let t = [0.0, 1e3, 2e3];
    let engine = [0.0, 1e-6, 0.0];
    let reference = [0.0; 3];
    let spans = serde_json::json!({ "#r1.i": 1.0 });
    let mut compared = SpanCheck {
        failed: false,
        report: Vec::new(),
        checked: BTreeSet::new(),
        engine_t: &t,
        reference_t: &t,
        grid: &t,
        held: BTreeSet::new(),
        spans: None,
    };
    compared.record("#r1.i", &engine, &reference);
    assert!(compared.failed, "{:?}", compared.report);
    compared.failed = false;
    compared.spans = spans.as_object();
    compared.record("#r1.i", &engine, &reference);
    assert!(!compared.failed, "{:?}", compared.report);
}

#[test]
fn the_gate_current_comes_from_the_pin_power_where_the_gate_is_off_the_source() {
    let t = [0.0, 1e3, 2e3];
    let (d, g, s) = ([2.0, 2.0, 2.0], [0.0, 5e-4, 4.0], [0.0; 3]);
    let id = [0.0, 1e-3, 1e-3];
    let p = [0.0, 2e-3, 2e-3 + 4.0 * 1e-6];
    let (current, rows) = gate_current(&t, &t, &p, [&d, &g, &s], &id);
    assert_eq!(rows, [2e3]);
    assert_eq!(current[0], 0.0);
    assert!((current[2] - 1e-6).abs() < 1e-18, "{current:?}");
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
