use crate::common;

use std::fs;
use std::path::PathBuf;

use orbe_engine::blob::Blobs;
use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::name::Ref;
use orbe_engine::run_folder::Results;
use orbe_engine::trace::Trace;

use common::examples::{self as examples, transient_examples};
use common::{
    column, held_capacitor, resistor, simulate, simulate_steps, source, span, trace, transient,
};

const LED: &str = r#"{
  "name": "led1",
  "kind": "led",
  "pins": { "anode": "vk", "cathode": "vbtn" },
  "saturation_current": { "value": 6.711e-10, "unit": "A", "source": "user" },
  "emission_coefficient": { "value": 4.427, "unit": "1", "source": "user" },
  "series_resistance": { "value": 2.115, "unit": "ohm", "source": "user" },
  "ratings": {
    "current": { "value": 0.03, "unit": "A", "source": "user" },
    "power": { "value": 0.075, "unit": "W", "source": "user" }
  }
}"#;

/// A button to ground with a 10 kohm pull-up, and an LED with `series` ohm from 5 V.
fn button(series: f64, bounce: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 7,
      "analysis": {{ "transient": {{ "stop_ps": 20000000000, "max_step_ps": 10000000, "grid_ps": 10000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "vk", "vbtn"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": 5.0, "unit": "V", "source": "user" }}
        }},
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "vcc", "b": "vbtn" }},
          "resistance": {{ "value": 10000.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "power": null }}
        }},
        {{
          "name": "r2",
          "kind": "resistor",
          "pins": {{ "a": "vcc", "b": "vk" }},
          "resistance": {{ "value": {series:?}, "unit": "ohm", "source": "user" }},
          "ratings": {{ "power": null }}
        }},
        {LED},
        {{
          "name": "sw1",
          "kind": "switch",
          "pins": {{ "a": "vbtn", "b": "gnd" }},
          "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }},
          "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
          "initial": "open",
          "events": [
            {{ "t_ps": 2000000000, "state": "closed" }},
            {{ "t_ps": 12000000000, "state": "open" }}
          ],
          "bounce": {bounce}
        }}
      ]
    }}"#
    )
}

const BOUNCE: &str = r#"{
  "window": { "value": 1000000000, "unit": "ps", "source": "user" },
  "max_bounces": { "value": 4, "unit": "1", "source": "user" }
}"#;

#[ignore = "full tier"]
#[test]
fn a_transient_writes_the_operating_point_and_every_contact_edge() {
    let results = simulate(&button(220.0, BOUNCE));
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let observations = serde_json::to_value(results.observations()).unwrap();
    let vbtn = observations["op"]["#net.vbtn"].as_f64().unwrap();
    assert!((vbtn - 5.0).abs() < 1e-3, "{vbtn}");
    let edges = observations["switches"]["#sw1"].as_array().unwrap();
    assert_eq!(edges.len(), 14);
    assert_eq!(
        edges[0],
        serde_json::json!({ "t_ps": 2000000000, "state": "closed" })
    );
    assert_eq!(
        edges[13],
        serde_json::json!({ "t_ps": 12727823423_u64, "state": "open" })
    );
}

#[ignore = "full tier"]
#[test]
fn an_overloaded_led_is_over_for_exactly_the_time_the_button_is_closed() {
    let results = simulate(&button(22.0, BOUNCE));
    let observations = serde_json::to_value(results.observations()).unwrap();
    let edges = observations["switches"]["#sw1"].as_array().unwrap();
    let closed: u64 = edges
        .chunks(2)
        .map(|pair| pair[1]["t_ps"].as_u64().unwrap() - pair[0]["t_ps"].as_u64().unwrap())
        .sum();
    let found: Vec<_> = results.diagnostics().iter().map(Diagnostic::code).collect();
    assert_eq!(found, ["rating.current", "rating.power"]);
    for diagnostic in results.diagnostics() {
        let doc = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(doc["t_ps"], 2_000_001_000_u64, "{doc}");
        assert_eq!(doc["data"]["over_ps"], closed, "{doc}");
        let peak_ps = doc["data"]["peak_ps"].as_u64().unwrap();
        assert!(peak_ps > 2_000_000_000 && peak_ps < 12_000_000_000, "{doc}");
    }
}

#[ignore = "full tier"]
#[test]
fn seed_moves_only_the_bounce_edges() {
    let seven = simulate(&button(220.0, BOUNCE));
    let eight = simulate(&button(220.0, BOUNCE).replace(r#""seed": 7"#, r#""seed": 8"#));
    let edges = |results: &Results| {
        serde_json::to_value(results.observations()).unwrap()["switches"]["#sw1"].clone()
    };
    assert_ne!(edges(&seven), edges(&eight));
    let plain = |seed: &str| {
        let json = button(220.0, "null").replace(r#""seed": 7"#, seed);
        serde_json::to_vec(simulate(&json).observations()).unwrap()
    };
    assert_eq!(plain(r#""seed": 7"#), plain(r#""seed": 8"#));
}

fn example_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/circuits")
        .join(name)
}

fn example(name: &str) -> String {
    fs::read_to_string(example_dir(name).join("netlist.json")).unwrap()
}

#[ignore = "full tier"]
#[test]
fn every_example_writes_its_grid_and_its_steps_change_nothing_else() {
    for name in transient_examples() {
        let netlist = examples::parse(&example_dir(&name));
        let orbe_engine::netlist::Analysis::Transient(transient) = netlist.analysis() else {
            panic!("{name} is not a transient");
        };
        let (stop, grid) = (transient.stop.as_ps(), transient.grid);
        let results = orbe_engine::simulate(&netlist);
        let trace = results.trace().unwrap();
        let rows: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
        assert_eq!(rows.len() as u64, stop.div_ceil(grid) + 1, "{name}");
        for (k, &at) in rows.iter().enumerate() {
            assert_eq!(at, (k as u64 * grid).min(stop), "{name} row {k}");
        }
        let op = results.observations().op().unwrap();
        for (index, reference) in trace.columns().iter().enumerate() {
            assert_eq!(
                trace.column(index)[0],
                op.values[reference].get(),
                "{name} {reference}"
            );
        }
        let steps = orbe_engine::simulate_steps(&netlist);
        assert_eq!(results.diagnostics(), steps.diagnostics(), "{name}");
        let (grid, steps) = (results.observations(), steps.observations());
        assert_eq!(grid.end(), steps.end(), "{name}");
        assert_eq!(grid.switches(), steps.switches(), "{name}");
        assert_eq!(grid.logic(), steps.logic(), "{name}");
    }
}

/// Each interval's envelope is the steps run's samples strictly inside it plus its two rows,
/// and a row at the time of an accepted step is that step, bit for bit.
fn assert_rows_read_the_steps(run: &str, trace: &Trace, steps: &Trace) {
    assert_eq!(trace.intervals(), trace.rows() - 1, "{run}");
    let (rows, step_t) = (trace.time(), steps.time());
    for (j, reference) in trace.columns().iter().enumerate() {
        let (values, step_values) = (trace.column(j), steps.column(j));
        for (k, at) in rows.iter().enumerate() {
            if let Ok(step) = step_t.binary_search(at) {
                assert_eq!(
                    values[k].to_bits(),
                    step_values[step].to_bits(),
                    "{run} {reference} row {k}"
                );
            }
            if k == 0 {
                continue;
            }
            let from = step_t.partition_point(|t| t <= &rows[k - 1]);
            let to = step_t.partition_point(|t| t < at);
            let samples = step_values[from..to]
                .iter()
                .chain([&values[k - 1], &values[k]]);
            let low = samples.clone().copied().fold(f64::INFINITY, f64::min);
            let high = samples.copied().fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(
                trace.min_column(j)[k - 1],
                low,
                "{run} {reference} interval {k}"
            );
            assert_eq!(
                trace.max_column(j)[k - 1],
                high,
                "{run} {reference} interval {k}"
            );
        }
    }
}

/// `json` with its `grid_ps` set to `grid`, parsed with `blobs`.
fn with_grid(json: &str, blobs: &Blobs, grid: u64) -> orbe_engine::netlist::Netlist {
    let mut doc: serde_json::Value = serde_json::from_str(json).unwrap();
    doc["analysis"]["transient"]["grid_ps"] = grid.into();
    orbe_engine::netlist::parse_with(doc.to_string().as_bytes(), blobs).unwrap()
}

/// A row is a probe. On the committed grid, a grid four times finer, a grid off every
/// breakpoint, and a grid with no row between t = 0 and `stop_ps`, every row and interval
/// reads the steps run, and the runs take the same steps and stops and write the same
/// diagnostics, observations and logic.
#[ignore = "full tier"]
#[test]
fn a_grid_never_changes_the_run() {
    for name in transient_examples() {
        let json = example(&name);
        let blobs = examples::blobs(&example_dir(&name));
        let netlist = examples::parse(&example_dir(&name));
        let orbe_engine::netlist::Analysis::Transient(transient) = netlist.analysis() else {
            panic!("{name} is not a transient");
        };
        let (grid, stop) = (transient.grid, transient.stop.as_ps());
        let (steps, runs) = std::thread::scope(|scope| {
            let steps = scope.spawn(|| orbe_engine::simulate_steps(&netlist));
            let runs: Vec<_> = [grid, grid / 4, 3 * grid + 7, stop]
                .into_iter()
                .map(|grid| {
                    let (json, blobs) = (&json, &blobs);
                    scope.spawn(move || {
                        (
                            grid,
                            orbe_engine::simulate_stats(&with_grid(json, blobs, grid)),
                        )
                    })
                })
                .collect();
            let runs: Vec<_> = runs.into_iter().map(|run| run.join().unwrap()).collect();
            (steps.join().unwrap(), runs)
        });
        for (grid, (results, _)) in &runs {
            let run = format!("{name} on a {grid} ps grid");
            assert_rows_read_the_steps(&run, results.trace().unwrap(), steps.trace().unwrap());
        }
        let (_, (first, first_stats)) = &runs[0];
        for (grid, (results, stats)) in &runs[1..] {
            let run = format!("{name} on a {grid} ps grid");
            assert_eq!(stats, first_stats, "{run}");
            assert_eq!(results.diagnostics(), first.diagnostics(), "{run}");
            assert_eq!(
                serde_json::to_value(results.observations()).unwrap(),
                serde_json::to_value(first.observations()).unwrap(),
                "{run}"
            );
            assert_eq!(
                results.trace().unwrap().logic(),
                first.trace().unwrap().logic(),
                "{run}"
            );
        }
    }
}

/// `#net.*` and `*.v` are volts, `*.i` amperes, `*.p` watts.
fn floor(reference: &Ref) -> f64 {
    match reference.to_string().rsplit_once('.') {
        Some((_, "i" | "iin" | "iout")) => 1e-12,
        Some((_, "p")) => 1e-15,
        _ => 1e-9,
    }
}

/// The contact edges of `packages/engine/tests/circuits/rc-switch`: closed at 1 ms, open at 31 ms.
const RC_EDGES: [(f64, bool); 2] = [(1e9, true), (31e9, false)];

/// Thevenin voltage and time constant seen by `farad` on `vout` with the switch `closed`.
fn rc_thevenin(closed: bool, farad: f64) -> (f64, f64) {
    let series = 10_000.0 + if closed { 0.1 } else { 1e9 };
    let bleeder = 10_000.0;
    let resistance = series * bleeder / (series + bleeder);
    (5.0 * bleeder / (series + bleeder), resistance * farad)
}

/// `#net.vout` and the current into `farad` at `t_ps`, on the interval that ends there.
fn rc_closed_form(t_ps: f64, farad: f64) -> (f64, f64) {
    let (mut start, mut closed) = (0.0, false);
    let mut v0 = rc_thevenin(false, farad).0;
    for (edge, to) in RC_EDGES {
        if t_ps <= edge {
            break;
        }
        let (vth, tau) = rc_thevenin(closed, farad);
        v0 = vth + (v0 - vth) * libm::exp(-(edge - start) * 1e-12 / tau);
        (start, closed) = (edge, to);
    }
    let (vth, tau) = rc_thevenin(closed, farad);
    let v = vth + (v0 - vth) * libm::exp(-(t_ps - start) * 1e-12 / tau);
    (v, (vth - v) * farad / tau)
}

/// On the committed grid and on one off every step, every row of `packages/engine/tests/circuits/rc-switch` follows
/// the closed form, and so does each row no step lands on, which is a probe.
#[ignore = "full tier"]
#[test]
fn the_rc_example_follows_its_closed_form_at_every_row() {
    let json = example("rc-switch");
    let steps = simulate_steps(&json);
    let steps = trace(&steps).time();
    for grid in [20_000_000, 60_000_007] {
        let results = orbe_engine::simulate(&with_grid(&json, &Blobs::new(), grid));
        let trace = trace(&results);
        let vout = column(trace, "#net.vout");
        let current = column(trace, "#c1.i");
        let (v_span, i_span) = (span(vout), span(current));
        let (mut worst, mut probes) = ((0.0f64, 0.0f64), 0);
        for (row, at) in trace.time().iter().enumerate() {
            probes += usize::from(steps.binary_search(at).is_err());
            let (v, i) = rc_closed_form(at.as_ps() as f64, 1e-6);
            worst.0 = worst.0.max((vout[row] - v).abs() / v_span);
            worst.1 = worst.1.max((current[row] - i).abs() / i_span);
        }
        assert!(probes > trace.rows() / 2, "{grid} ps: {probes} probes");
        assert!(worst.0 <= 1e-3 && worst.1 <= 1e-3, "{grid} ps: {worst:?}");
    }
}

/// `#l1.i` of `packages/engine/tests/circuits/rl-step` at `t_ps`: 5 V onto 0.505 H behind 70 ohm at 1 ms, then 0 V
/// at 30 ms.
fn rl_closed_form(t_ps: f64) -> f64 {
    let (tau, top, on, off) = (0.505 / 70.0, 5.0 / 70.0, 1e9, 30e9);
    let rise = |ps: f64| top * (1.0 - libm::exp(-(ps - on) * 1e-12 / tau));
    if t_ps <= on {
        0.0
    } else if t_ps <= off {
        rise(t_ps)
    } else {
        rise(off) * libm::exp(-(t_ps - off) * 1e-12 / tau)
    }
}

/// Capacitor voltage of a series RLC after a 5 V step at 100 us, from rest. `r` is the
/// inductor's series resistance. The pair is underdamped: Q is about 31.6 at 1 ohm and 316 at
/// 0.1 ohm.
fn rlc_closed_form(t_ps: f64, r: f64) -> f64 {
    let (l, c, vin, step) = (1e-3_f64, 1e-6_f64, 5.0_f64, 1e8_f64);
    if t_ps <= step {
        return 0.0;
    }
    let (w0, alpha) = (1.0 / (l * c).sqrt(), r / (2.0 * l));
    let wd = (w0 * w0 - alpha * alpha).sqrt();
    let t = (t_ps - step) * 1e-12;
    let decay = libm::exp(-alpha * t);
    vin * (1.0 - decay * (libm::cos(wd * t) + (alpha / wd) * libm::sin(wd * t)))
}

/// The two series RLC controls follow the underdamped step, and no store turns stiff.
/// The row bound is the measured drift of that ring. The step count is a ceiling, ten
/// percent above the measured run, so a later policy can take fewer steps.
#[ignore = "full tier"]
#[test]
fn the_rlc_controls_follow_the_underdamped_step() {
    for (name, r, steps) in [("rlc-q31", 1.0, 5329), ("rlc-q316", 0.1, 3482)] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/circuit/controls")
            .join(format!("{name}.json"));
        let json = fs::read_to_string(&path).unwrap();
        let netlist = orbe_engine::netlist::parse(json.as_bytes()).unwrap();
        let (results, stats) = orbe_engine::simulate_stats(&netlist);
        assert!(
            results.diagnostics().is_empty(),
            "{name}: {:?}",
            results.diagnostics()
        );
        assert_eq!(stats.stiffens, 0, "{name} turns a store stiff");
        let bound = steps + steps / 10;
        assert!(
            stats.landings <= bound,
            "{name}: {} steps, bound {bound}",
            stats.landings
        );
        let trace = trace(&results);
        let volts = column(trace, "#net.x");
        let swing = span(volts);
        let mut worst = 0.0f64;
        let mut at_worst = 0u64;
        for (row, at) in trace.time().iter().enumerate() {
            let want = rlc_closed_form(at.as_ps() as f64, r);
            let miss = (volts[row] - want).abs() / swing;
            if miss > worst {
                worst = miss;
                at_worst = at.as_ps();
            }
        }
        eprintln!("{name}: {worst:e} of the swing at {at_worst} ps, swing {swing} V");
        assert!(
            worst <= 2e-3,
            "{name}: {worst:e} of the swing at {at_worst} ps, swing {swing} V"
        );
    }
}

/// On the committed grid and on one off every step, every row of `packages/engine/tests/circuits/rl-step` follows
/// the closed form within 1e-4 of its swing, and so does each row no step lands on, which is a
/// probe. The current is held across the fall at 30 ms, so a coil that reset there would miss.
#[ignore = "full tier"]
#[test]
fn the_rl_example_follows_its_closed_form_at_every_row() {
    let json = example("rl-step");
    let steps = simulate_steps(&json);
    let steps = trace(&steps).time();
    for grid in [50_000_000, 60_000_007] {
        let results = orbe_engine::simulate(&with_grid(&json, &Blobs::new(), grid));
        let trace = trace(&results);
        let current = column(trace, "#l1.i");
        let swing = span(current);
        let (mut worst, mut probes) = (0.0f64, 0);
        for (row, at) in trace.time().iter().enumerate() {
            probes += usize::from(steps.binary_search(at).is_err());
            let want = rl_closed_form(at.as_ps() as f64);
            worst = worst.max((current[row] - want).abs() / swing);
        }
        assert!(probes > trace.rows() / 2, "{grid} ps: {probes} probes");
        assert!(worst <= 1e-4, "{grid} ps: {worst:e} of the swing");
    }
}

/// The run again at half `max_step_ps`, compared on its rows, within `bound` of each column's span.
fn assert_refines(json: &str, max_step: &str, half: &str, bound: f64) {
    let coarse = simulate(json);
    let fine = simulate(&json.replace(
        &format!(r#""max_step_ps": {max_step}"#),
        &format!(r#""max_step_ps": {half}"#),
    ));
    let (coarse, fine) = (trace(&coarse), trace(&fine));
    // The grid sets the rows, so both runs write the same ones.
    assert_eq!(coarse.time(), fine.time());
    for (index, reference) in coarse.columns().iter().enumerate() {
        let (left, right) = (coarse.column(index), fine.column(index));
        let worst = left
            .iter()
            .zip(right)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        assert!(
            worst <= bound * span(left) + floor(reference),
            "{reference}: {worst} against span {}",
            span(left)
        );
    }
}

fn interpolate(t: &[f64], values: &[f64], at: f64) -> f64 {
    let k = t.partition_point(|&x| x <= at);
    if k == 0 || k == t.len() {
        return values[k.saturating_sub(1)];
    }
    let (t0, t1, v0, v1) = (t[k - 1], t[k], values[k - 1], values[k]);
    v0 + (v1 - v0) * (at - t0) / (t1 - t0)
}

#[ignore = "full tier"]
#[test]
fn halving_max_step_moves_each_example_by_under_1e_4_of_span() {
    assert_refines(&example("rc-switch"), "20000000", "10000000", 1e-4);
    assert_refines(&example("button-led"), "10000000", "5000000", 1e-4);
}

/// The button with 100 nF across the contact. Pressing shorts it through 0.1 ohm, a 10 ns mode.
fn stiff_button(max_step: u64) -> String {
    button(220.0, "null")
        .replace(
            r#""stop_ps": 20000000000, "max_step_ps": 10000000, "grid_ps": 10000000"#,
            &format!(r#""stop_ps": 4000000000, "max_step_ps": {max_step}, "grid_ps": {max_step}"#),
        )
        .replace(r#""t_ps": 12000000000"#, r#""t_ps": 3000000000"#)
        .replace(
            r#"        {
          "name": "sw1","#,
            r#"        {
          "name": "c2",
          "kind": "capacitor",
          "pins": { "a": "vbtn", "b": "gnd" },
          "capacitance": { "value": 1e-7, "unit": "F", "source": "user" },
          "initial": null
        },
        {
          "name": "sw1","#,
        )
}

#[ignore = "full tier"]
#[test]
fn a_stiff_mode_dies_out_after_each_edge_instead_of_ringing() {
    let coarse = simulate_steps(&stiff_button(1_000_000));
    let fine = simulate_steps(&stiff_button(500_000));
    let (coarse, fine) = (trace(&coarse), trace(&fine));
    let vbtn = column(coarse, "#net.vbtn");
    let fine_vbtn = column(fine, "#net.vbtn");
    assert_the_edge_settles_without_ringing(coarse);
    assert_the_edge_settles_without_ringing(fine);
    let fine_t: Vec<f64> = fine.time().iter().map(|t| t.as_ps() as f64).collect();
    let settled = |t: u64| {
        [2_000_000_000, 3_000_000_000]
            .iter()
            .all(|&edge| t < edge || t > edge + 50_000_000)
    };
    let mut worst = 0.0f64;
    for (row, at) in coarse.time().iter().enumerate() {
        if settled(at.as_ps()) {
            let other = interpolate(&fine_t, fine_vbtn, at.as_ps() as f64);
            worst = worst.max((vbtn[row] - other).abs());
        }
    }
    assert!(
        worst <= 1e-4 * span(vbtn),
        "{worst} against span {}",
        span(vbtn)
    );
}

fn assert_the_edge_settles_without_ringing(trace: &Trace) {
    for edge in [2_000_000_000, 3_000_000_000] {
        settles_toward_its_later_value(trace, "#net.vbtn", edge);
    }
}

/// From `edge` through the next 5 µs, `reference` moves toward its value 50 µs later
/// and does not cross that value by more than 1e-4 of the column's span.
fn settles_toward_its_later_value(trace: &Trace, reference: &str, edge: u64) {
    let values = column(trace, reference);
    let times: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    let tol = 1e-4 * span(values);
    let settled = times
        .iter()
        .position(|&t| t >= edge + 50_000_000)
        .unwrap_or_else(|| panic!("no row 50 µs after {edge}"));
    let target = values[settled];
    let window: Vec<f64> = times
        .iter()
        .zip(values)
        .filter(|(t, _)| (edge..=edge + 5_000_000).contains(*t))
        .map(|(_, v)| *v)
        .collect();
    assert!(window.len() >= 2, "{edge}: {window:?}");
    let falling = window[0] >= target;
    for pair in window.windows(2) {
        let toward = if falling {
            pair[1] <= pair[0] + tol && pair[1] >= target - tol
        } else {
            pair[1] >= pair[0] - tol && pair[1] <= target + tol
        };
        assert!(toward, "{edge}: {pair:?} toward {target} tol {tol}");
    }
}

/// Twelve 0.1 ohm switches close 50 ps apart from 200 ns. 100 nF sits behind 220 ohm.
fn cluster_switches() -> String {
    let mut switches = String::new();
    for n in 0..12 {
        let edge = 200_000 + 50 * n;
        switches.push_str(&format!(
            r#",
        {{
          "name": "s{n}",
          "kind": "switch",
          "pins": {{ "a": "vout", "b": "gnd" }},
          "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }},
          "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
          "initial": "open",
          "events": [{{ "t_ps": {edge}, "state": "closed" }}],
          "bounce": null
        }}"#
        ));
    }
    format!(
        r#"{{
      "version": 0,
      "seed": 1,
      "analysis": {{ "transient": {{ "stop_ps": 52000000, "max_step_ps": 1000000, "grid_ps": 1000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "vout"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": 5.0, "unit": "V", "source": "user" }}
        }},
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "vcc", "b": "vout" }},
          "resistance": {{ "value": 220.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "power": null }}
        }},
        {{
          "name": "c1",
          "kind": "capacitor",
          "pins": {{ "a": "vout", "b": "gnd" }},
          "capacitance": {{ "value": 1e-7, "unit": "F", "source": "user" }},
          "initial": null
        }}{switches}
      ]
    }}"#
    )
}

#[ignore = "full tier"]
#[test]
fn a_cluster_of_edges_inside_one_nanosecond_settles_without_ringing() {
    let last = 200_000 + 50 * 11;
    settles_toward_its_later_value(
        trace(&simulate_steps(&cluster_switches())),
        "#net.vout",
        last,
    );
}

/// `packages/engine/tests/circuits/rc-switch` with 100 nF across the source and 3 uF beside `c1`.
fn rc_with_loops() -> String {
    let extra = r#"},
    {
      "name": "c2",
      "kind": "capacitor",
      "pins": { "a": "vin", "b": "gnd" },
      "capacitance": { "value": 1e-7, "unit": "F", "source": "user" },
      "initial": null
    },
    {
      "name": "c3",
      "kind": "capacitor",
      "pins": { "a": "vout", "b": "gnd" },
      "capacitance": { "value": 3e-6, "unit": "F", "source": "user" },
      "initial": null
    }
  ]"#;
    let json = example("rc-switch");
    let end = json.rfind("}\n  ]").unwrap();
    format!("{}{}{}", &json[..end], extra, &json[end + "}\n  ]".len()..])
}

#[ignore = "full tier"]
#[test]
fn capacitors_across_a_source_or_each_other_run_through_every_edge() {
    let results = simulate(&rc_with_loops());
    let trace = trace(&results);
    assert_eq!(
        trace.end(),
        Some(orbe_engine::time::Time::from_ps(81_000_000_000))
    );
    let (c1, c3) = (column(trace, "#c1.i"), column(trace, "#c3.i"));
    // Within Newton's current tolerance: 1e-6 relative plus 1 pA.
    for (row, (&one, &three)) in c1.iter().zip(c3).enumerate() {
        assert!(
            (three - 3.0 * one).abs() <= 1e-6 * three.abs() + 1e-12,
            "row {row}: {three} vs 3 x {one}"
        );
    }
    // c1 and c3 are 4 uF on `vout`. c2 sits on the ideal source and carries nothing.
    // The restart holds each at its voltage, so the rows after each edge start on the curve.
    let (vout, c2) = (column(trace, "#net.vout"), column(trace, "#c2.i"));
    let (v_span, i_span) = (span(vout), span(c3));
    let mut worst = (0.0f64, 0.0f64, 0.0f64);
    for (row, at) in trace.time().iter().enumerate() {
        let (v, i) = rc_closed_form(at.as_ps() as f64, 4e-6);
        worst.0 = worst.0.max((vout[row] - v).abs() / v_span);
        worst.1 = worst.1.max((c3[row] - 0.75 * i).abs() / i_span);
        worst.2 = worst.2.max(c2[row].abs());
    }
    assert!(
        worst.0 <= 1e-4 && worst.1 <= 1e-4 && worst.2 <= 1e-12,
        "{worst:?}"
    );
}

/// A 10 us transient of `parts` on `nets`.
fn held(nets: &str, parts: &[&str]) -> String {
    transient(10_000_000, 1_000_000, nets, parts)
}

/// Row 0 of `#{name}.i`, after checking the run reached its stop and held `name`.
/// The hold sits behind 1 fs / C ohm, so the voltage across moves by i * 1 fs / C.
fn held_current(trace: &Trace, name: &str, [a, b]: [&str; 2], farad: f64, initial: f64) -> f64 {
    assert_eq!(
        trace.end(),
        Some(orbe_engine::time::Time::from_ps(10_000_000))
    );
    let node = |net: &str| match net {
        "gnd" => 0.0,
        net => column(trace, &format!("#net.{net}"))[0],
    };
    let across = node(a) - node(b);
    let current = column(trace, &format!("#{name}.i"))[0];
    let moved = current.abs() * 1e-15 / farad;
    assert!(
        (across - initial).abs() <= moved + 1e-12,
        "{name}: {across} V held at {initial} V with {current} A"
    );
    current
}

#[ignore = "full tier"]
#[test]
fn a_held_capacitor_across_a_source_at_its_voltage_takes_no_current() {
    let json = held(
        r#""gnd", "vcc""#,
        &[
            &source(5.0),
            &held_capacitor("c1", "vcc", "gnd", 1e-6, 5.0),
            &resistor("r1", "vcc", "gnd", 1000.0),
        ],
    );
    let results = simulate(&json);
    let current = held_current(trace(&results), "c1", ["vcc", "gnd"], 1e-6, 5.0);
    assert!(current.abs() <= 1e-12, "{current}");
}

#[ignore = "full tier"]
#[test]
fn held_capacitors_in_parallel_share_their_current_by_capacitance() {
    let json = held(
        r#""gnd", "vout""#,
        &[
            &held_capacitor("c1", "vout", "gnd", 1e-6, 5.0),
            &held_capacitor("c2", "vout", "gnd", 2e-6, 5.0),
            &resistor("r1", "vout", "gnd", 1000.0),
        ],
    );
    let results = simulate(&json);
    let trace = trace(&results);
    let one = held_current(trace, "c1", ["vout", "gnd"], 1e-6, 5.0);
    let two = held_current(trace, "c2", ["vout", "gnd"], 2e-6, 5.0);
    // r1 and gmin's 1e-12 S draw 5 mA and 5 pA.
    let total = -(5e-3 + 5e-12);
    assert!((one - total / 3.0).abs() <= 1e-9 * total.abs(), "{one}");
    assert!(
        (two - 2.0 * total / 3.0).abs() <= 1e-9 * total.abs(),
        "{two}"
    );
}

#[ignore = "full tier"]
#[test]
fn held_capacitors_in_parallel_at_different_voltages_start_from_their_shared_charge() {
    let json = held(
        r#""gnd", "vout""#,
        &[
            &held_capacitor("c1", "vout", "gnd", 1e-6, 5.0),
            &held_capacitor("c2", "vout", "gnd", 2e-6, 4.0),
            &resistor("r1", "vout", "gnd", 1000.0),
        ],
    );
    let results = simulate(&json);
    let trace = trace(&results);
    held_current(trace, "c1", ["vout", "gnd"], 1e-6, 5.0);
    held_current(trace, "c2", ["vout", "gnd"], 2e-6, 4.0);
    // The two holds meet at the capacitance-weighted mean, as the charge would.
    let shared = (1.0 * 5.0 + 2.0 * 4.0) / 3.0;
    let vout = column(trace, "#net.vout")[0];
    assert!((vout - shared).abs() <= 1e-9, "{vout}");
}

#[ignore = "full tier"]
#[test]
fn held_capacitors_in_series_across_a_source_split_its_voltage() {
    let json = held(
        r#""gnd", "vcc", "mid""#,
        &[
            &source(5.0),
            &held_capacitor("c1", "vcc", "mid", 1e-6, 3.0),
            &held_capacitor("c2", "mid", "gnd", 1e-6, 2.0),
            &resistor("r1", "mid", "gnd", 1e6),
        ],
    );
    let results = simulate(&json);
    let trace = trace(&results);
    let upper = held_current(trace, "c1", ["vcc", "mid"], 1e-6, 3.0);
    let lower = held_current(trace, "c2", ["mid", "gnd"], 1e-6, 2.0);
    // r1 and gmin draw 2 uA and 2 pA from `mid`. Equal capacitors split it evenly.
    let half = (2e-6 + 2e-12) / 2.0;
    assert!((upper - half).abs() <= 1e-9 * half, "{upper}");
    assert!((lower + half).abs() <= 1e-9 * half, "{lower}");
}
