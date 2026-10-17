use std::fs;
use std::path::PathBuf;

use sim_core::diagnostic::Diagnostic;
use sim_core::name::Ref;
use sim_core::run_folder::Results;
use sim_core::trace::Trace;

fn simulate(json: &str) -> Results {
    let netlist = sim_core::netlist::parse(json.as_bytes()).unwrap();
    sim_circuit::simulate(&netlist)
}

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
      "analysis": {{ "transient": {{ "stop_ps": 20000000000, "max_step_ps": 10000000 }} }},
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

fn example(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .join("netlist.json");
    fs::read_to_string(path).unwrap()
}

fn trace(results: &Results) -> &Trace {
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    results.trace().unwrap()
}

fn column<'t>(trace: &'t Trace, reference: &str) -> &'t [f64] {
    let reference: Ref = reference.parse().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|r| *r == reference)
        .unwrap();
    trace.column(index)
}

fn span(values: &[f64]) -> f64 {
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    max - min
}

/// `#net.*` and `*.v` are volts, `*.i` amperes, `*.p` watts.
fn floor(reference: &Ref) -> f64 {
    match reference.to_string().rsplit_once('.') {
        Some((_, "i")) => 1e-12,
        Some((_, "p")) => 1e-15,
        _ => 1e-9,
    }
}

/// The contact edges of `examples/rc-switch`: closed at 1 ms, open at 31 ms.
const RC_EDGES: [(f64, bool); 2] = [(1e9, true), (31e9, false)];

/// Thevenin voltage and time constant seen by `c1` with the switch `closed`.
fn rc_thevenin(closed: bool) -> (f64, f64) {
    let series = 10_000.0 + if closed { 0.1 } else { 1e9 };
    let bleeder = 10_000.0;
    let resistance = series * bleeder / (series + bleeder);
    (5.0 * bleeder / (series + bleeder), resistance * 1e-6)
}

/// `#net.vout` and `#c1.i` at `t_ps`, on the interval that ends there.
fn rc_closed_form(t_ps: f64) -> (f64, f64) {
    let (mut start, mut closed) = (0.0, false);
    let mut v0 = rc_thevenin(false).0;
    for (edge, to) in RC_EDGES {
        if t_ps <= edge {
            break;
        }
        let (vth, tau) = rc_thevenin(closed);
        v0 = vth + (v0 - vth) * (-(edge - start) * 1e-12 / tau).exp();
        (start, closed) = (edge, to);
    }
    let (vth, tau) = rc_thevenin(closed);
    let v = vth + (v0 - vth) * (-(t_ps - start) * 1e-12 / tau).exp();
    (v, (vth - v) * 1e-6 / tau)
}

#[test]
fn the_rc_example_follows_its_closed_form_at_every_row() {
    let results = simulate(&example("rc-switch"));
    let trace = trace(&results);
    let vout = column(trace, "#net.vout");
    let current = column(trace, "#c1.i");
    let (v_span, i_span) = (span(vout), span(current));
    let mut worst = (0.0f64, 0.0f64);
    for (row, at) in trace.time().iter().enumerate() {
        let (v, i) = rc_closed_form(at.as_ps() as f64);
        worst.0 = worst.0.max((vout[row] - v).abs() / v_span);
        worst.1 = worst.1.max((current[row] - i).abs() / i_span);
    }
    assert!(worst.0 <= 1e-3 && worst.1 <= 1e-3, "{worst:?}");
}

/// The run again at half `max_step_ps`, compared on both grids, within `bound` of each column's span.
fn assert_refines(json: &str, max_step: &str, half: &str, bound: f64) {
    let coarse = simulate(json);
    let fine = simulate(&json.replace(
        &format!(r#""max_step_ps": {max_step}"#),
        &format!(r#""max_step_ps": {half}"#),
    ));
    let (coarse, fine) = (trace(&coarse), trace(&fine));
    assert!(fine.rows() > coarse.rows());
    let times =
        |trace: &Trace| -> Vec<f64> { trace.time().iter().map(|t| t.as_ps() as f64).collect() };
    let (coarse_t, fine_t) = (times(coarse), times(fine));
    let mut grid: Vec<f64> = coarse_t.iter().chain(&fine_t).copied().collect();
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    for (index, reference) in coarse.columns().iter().enumerate() {
        let (left, right) = (coarse.column(index), fine.column(index));
        let worst = grid
            .iter()
            .map(|&t| (interpolate(&coarse_t, left, t) - interpolate(&fine_t, right, t)).abs())
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

#[test]
fn halving_max_step_moves_each_example_by_under_1e_4_of_span() {
    assert_refines(&example("rc-switch"), "20000000", "10000000", 1e-4);
    assert_refines(&example("button-led"), "10000000", "5000000", 1e-4);
}

/// The button with 100 nF across the contact. Pressing shorts it through 0.1 ohm, a 10 ns mode.
fn stiff_button(max_step: u64) -> String {
    button(220.0, "null")
        .replace(
            r#""stop_ps": 20000000000, "max_step_ps": 10000000"#,
            &format!(r#""stop_ps": 4000000000, "max_step_ps": {max_step}"#),
        )
        .replace(r#""t_ps": 12000000000"#, r#""t_ps": 3000000000"#)
        .replace(
            r#"        {
          "name": "sw1","#,
            r#"        {
          "name": "c2",
          "kind": "capacitor",
          "pins": { "a": "vbtn", "b": "gnd" },
          "capacitance": { "value": 1e-7, "unit": "F", "source": "user" }
        },
        {
          "name": "sw1","#,
        )
}

#[test]
fn a_stiff_mode_dies_out_after_each_edge_instead_of_ringing() {
    let coarse = simulate(&stiff_button(1_000_000));
    let fine = simulate(&stiff_button(500_000));
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
      "analysis": {{ "transient": {{ "stop_ps": 52000000, "max_step_ps": 1000000 }} }},
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
          "capacitance": {{ "value": 1e-7, "unit": "F", "source": "user" }}
        }}{switches}
      ]
    }}"#
    )
}

#[test]
fn a_cluster_of_edges_inside_one_nanosecond_settles_without_ringing() {
    let last = 200_000 + 50 * 11;
    settles_toward_its_later_value(trace(&simulate(&cluster_switches())), "#net.vout", last);
}
