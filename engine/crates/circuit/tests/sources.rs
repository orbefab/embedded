// Each test crate uses its own part of `common`.
#[allow(dead_code)]
mod common;

use std::fs;
use std::path::PathBuf;

use sim_core::netlist::Model;
use sim_core::time::Time;

use common::{column, resistor, simulate, simulate_steps, span, trace, transient};

fn level(value: f64, unit: &str) -> String {
    format!(r#"{{ "value": {value:?}, "unit": "{unit}", "source": "user" }}"#)
}

fn current_source(name: &str, net: &str, drive: &str) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "current_source", "pins": {{ "p": "{net}", "n": "gnd" }},
          "current": {drive} }}"#
    )
}

fn voltage_source(name: &str, net: &str, drive: &str) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "voltage_source", "pins": {{ "p": "{net}", "n": "gnd" }},
          "voltage": {drive} }}"#
    )
}

fn capacitor(name: &str, a: &str, farad: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "capacitor", "pins": {{ "a": "{a}", "b": "gnd" }},
          "capacitance": {{ "value": {farad:?}, "unit": "F", "source": "user" }}, "initial": null }}"#
    )
}

fn pulse(unit: &str, low: f64, high: f64, delay: u64, width: u64, period: u64) -> String {
    format!(
        r#"{{ "kind": "pulse", "initial": {}, "pulsed": {},
             "delay_ps": {delay}, "width_ps": {width}, "period_ps": {period} }}"#,
        level(low, unit),
        level(high, unit)
    )
}

fn step(unit: &str, low: f64, high: f64, at: u64) -> String {
    format!(
        r#"{{ "kind": "step", "initial": {}, "final": {}, "at_ps": {at} }}"#,
        level(low, unit),
        level(high, unit)
    )
}

fn op(nets: &str, parts: &[&str]) -> String {
    format!(
        r#"{{ "version": 0, "seed": 0, "analysis": {{ "op": {{}} }}, "ground": "gnd",
             "nets": [{nets}], "parts": [{}] }}"#,
        parts.join(",")
    )
}

fn rows(trace: &sim_core::trace::Trace) -> Vec<u64> {
    trace.time().iter().map(|t| t.as_ps()).collect()
}

#[test]
fn a_dc_current_source_delivers_negative_current_and_power() {
    let results = simulate(&op(
        r#""gnd", "vt""#,
        &[
            &current_source("i1", "vt", &level(1e-3, "A")),
            &resistor("r1", "vt", "gnd", 1000.0),
        ],
    ));
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let op = results.observations().op().unwrap();
    let value = |key: &str| op.values[&key.parse().unwrap()].get();
    let gmin = 1e-12;
    let vt = 1e-3 / (1e-3 + gmin);
    assert!(
        (value("#net.vt") - vt).abs() <= 1e-15,
        "{}",
        value("#net.vt")
    );
    assert_eq!(value("#i1.i"), -1e-3);
    assert_eq!(value("#i1.v"), value("#net.vt"));
    assert_eq!(value("#i1.p"), value("#i1.v") * -1e-3);
}

#[test]
fn a_net_fed_only_by_a_current_source_and_a_capacitor_floats() {
    let results = simulate(&op(
        r#""gnd", "vt""#,
        &[
            &current_source("i1", "vt", &level(1e-3, "A")),
            &capacitor("c1", "vt", 1e-6),
        ],
    ));
    let codes: Vec<_> = results
        .diagnostics()
        .iter()
        .map(|d| (d.code(), d.refs()))
        .collect();
    assert_eq!(codes, [("net.floating", vec!["#net.vt".parse().unwrap()])]);
}

/// A 0 to 1 V pulse at 2 ms, 10 ms wide, every 20 ms, through 1 kohm into 2 uF.
fn pulse_rc(stop: u64) -> String {
    transient(
        stop,
        10_000_000,
        r#""gnd", "vin", "vout""#,
        &[
            &voltage_source(
                "v1",
                "vin",
                &pulse("V", 0.0, 1.0, 2_000_000, 10_000_000, 20_000_000),
            ),
            &resistor("r1", "vin", "vout", 1000.0),
            &capacitor("c1", "vout", 2e-6),
        ],
    )
}

#[test]
fn a_source_jump_rows_the_old_value_and_restarts_on_backward_euler() {
    let results = simulate_steps(&pulse_rc(45_000_000));
    let trace = trace(&results);
    let rows = rows(trace);
    let jumps = [2_000_000u64, 12_000_000, 22_000_000, 32_000_000, 42_000_000];
    for jump in jumps {
        for at in [jump, jump + 1_000] {
            assert!(rows.binary_search(&at).is_ok(), "no row at {at}");
        }
        assert!(!rows.iter().any(|&t| t > jump && t < jump + 1_000));
    }
    let (vin, vout) = (column(trace, "#net.vin"), column(trace, "#net.vout"));
    let at = |values: &[f64], t: u64| values[rows.binary_search(&t).unwrap()];
    for (k, jump) in jumps.into_iter().enumerate() {
        let (old, new) = if k % 2 == 0 { (0.0, 1.0) } else { (1.0, 0.0) };
        assert_eq!(at(vin, jump), old, "the row at {jump} holds the old value");
        assert_eq!(at(vin, jump + 1_000), new, "{jump}");
        // The restart holds the capacitor, and the first step is backward Euler from it.
        let (h, g, c) = (1e-9, 1e-3, 2e-6);
        let held = at(vout, jump);
        let euler = (g * new + c / h * held) / (g + c / h);
        let got = at(vout, jump + 1_000);
        assert!((got - euler).abs() <= 1e-12, "{jump}: {got} vs {euler}");
    }
    // After a restart the steps double from 1 ns while the error allows.
    let after = rows.partition_point(|&t| t < 2_000_000);
    let steps: Vec<u64> = rows[after..after + 5]
        .windows(2)
        .map(|w| w[1] - w[0])
        .collect();
    assert_eq!(steps, [1_000, 2_000, 4_000, 8_000]);
}

/// The docs' aliasing case: a 1 MHz 0 to 3.3 V pulse through 100 ohm into 100 pF, on the
/// pulse's own 1 us grid. Its time constant is 10 ns.
fn aliased() -> String {
    transient(
        4_000_000,
        1_000_000,
        r#""gnd", "vin", "out""#,
        &[
            &voltage_source(
                "v1",
                "vin",
                &pulse("V", 0.0, 3.3, 1_000_000, 500_000, 1_000_000),
            ),
            &resistor("r1", "vin", "out", 100.0),
            &capacitor("c1", "out", 100e-12),
        ],
    )
}

#[test]
fn an_aliased_pulse_is_flat_on_the_rows_and_full_swing_in_the_envelope() {
    let results = simulate(&aliased());
    let trace = trace(&results);
    let index = |name: &str| {
        trace
            .columns()
            .iter()
            .position(|r| r.to_string() == name)
            .unwrap()
    };
    let (out, vin) = (index("#net.out"), index("#net.vin"));
    let out_rows = column(trace, "#net.out");
    // Every row lands on a rise and holds the end of the fall before it. The low half of
    // the period is 500 ns, 50 time constants of this 10 ns load, so the fall is gone and
    // the rows sit on the 1 nV floor. The measured span is 6.2e-9 V. The economics test
    // guards the swing tolerance.
    assert!(span(out_rows) < 1e-8, "rows span {:e}", span(out_rows));
    // The first interval is flat too: it ends on the first rise. Every later one holds a
    // full rise or a full fall, so the envelope reads the pulse the rows hide.
    let (mins, maxs) = (trace.min_column(out), trace.max_column(out));
    assert!(maxs[0] < 1e-9, "interval 0 max {:e}", maxs[0]);
    for k in 1..trace.intervals() {
        assert!(mins[k] <= 1e-3, "interval {k} min {}", mins[k]);
        assert!(maxs[k] >= 3.29, "interval {k} max {}", maxs[k]);
    }
    // The drive jumps between the levels, so its envelope is the exact pulse.
    let (mins, maxs) = (trace.min_column(vin), trace.max_column(vin));
    for k in 1..trace.intervals() {
        assert_eq!(mins[k], 0.0, "interval {k}");
        assert_eq!(maxs[k], 3.3, "interval {k}");
    }
}

/// The aliased pulse's circuit with edges from 1 ns, every step a row. 1e-6 of the level took
/// 362 steps an edge. The swing, not the level, sets the step, so a rise and a fall cost the
/// same.
#[test]
fn a_1_mhz_pin_into_10_ns_takes_under_100_steps_an_edge_within_1e_4_of_its_swing() {
    let edges = 20;
    let results = simulate_steps(&transient(
        10_000_000,
        1_000_000,
        r#""gnd", "vin", "out""#,
        &[
            &voltage_source(
                "v1",
                "vin",
                &pulse("V", 0.0, 3.3, 1_000, 500_000, 1_000_000),
            ),
            &resistor("r1", "vin", "out", 100.0),
            &capacitor("c1", "out", 100e-12),
        ],
    ));
    let trace = trace(&results);
    assert!(trace.rows() <= 100 * edges, "{} steps", trace.rows());
    let (rises, falls): (Vec<u64>, Vec<u64>) = rows(trace)
        .into_iter()
        .filter(|&t| t > 1_000)
        .partition(|&t| (t - 1_001) % 1_000_000 < 500_000);
    let (rises, falls) = (rises.len() as f64, falls.len() as f64);
    assert!(
        (rises - falls).abs() <= 0.1 * rises.min(falls),
        "{rises} rise steps against {falls} fall steps"
    );
    let jumps: Vec<(f64, f64)> = (0..edges)
        .map(|k| (1e3 + k as f64 * 5e5, if k % 2 == 0 { 3.3 } else { -3.3 }))
        .collect();
    let worst = rows(trace)
        .iter()
        .zip(column(trace, "#net.out"))
        .map(|(&t, &v)| (v - charge(0.0, &jumps, 1e4, t as f64)).abs())
        .fold(0.0, f64::max);
    assert!(worst <= 1e-4 * 3.3, "{worst:e}");
}

#[test]
fn a_jump_at_stop_gets_a_row_with_the_old_value() {
    let results = simulate(&transient(
        6_000_000,
        1_000_000,
        r#""gnd", "vt""#,
        &[
            &current_source(
                "i1",
                "vt",
                &pulse("A", 2e-4, 1e-3, 1_000_000, 2_000_000, 5_000_000),
            ),
            &resistor("r1", "vt", "gnd", 1000.0),
        ],
    ));
    let trace = trace(&results);
    assert_eq!(trace.end(), Some(Time::from_ps(6_000_000)));
    assert_eq!(
        rows(trace),
        [
            0, 1_000_000, 2_000_000, 3_000_000, 4_000_000, 5_000_000, 6_000_000
        ]
    );
    // The rows at 1, 3 and 6 ms are on an edge, so each holds the value before it.
    assert_eq!(
        column(trace, "#i1.i"),
        [-2e-4, -2e-4, -1e-3, -1e-3, -2e-4, -2e-4, -2e-4]
    );
}

#[test]
fn a_capacitor_directly_across_a_stepped_source_runs_to_the_stop() {
    let results = simulate(&transient(
        8_000_000_000,
        10_000_000,
        r#""gnd", "vin", "vout""#,
        &[
            &voltage_source("v1", "vin", &step("V", 0.0, 5.0, 1_000_000_000)),
            &resistor("r1", "vin", "vout", 10_000.0),
            &capacitor("c1", "vin", 1e-7),
        ],
    ));
    let trace = trace(&results);
    assert_eq!(trace.end(), Some(Time::from_ps(8_000_000_000)));
    assert!(trace.rows() < 2_000, "{} rows", trace.rows());
    let vin = column(trace, "#net.vin");
    assert_eq!(*vin.last().unwrap(), 5.0);
}

fn example(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .join("netlist.json");
    fs::read_to_string(path).unwrap()
}

/// An RC's voltage from `start`, each `(t_ps, change)` of its target charging in with time
/// constant `tau_ps`.
fn charge(start: f64, jumps: &[(f64, f64)], tau_ps: f64, t_ps: f64) -> f64 {
    jumps
        .iter()
        .filter(|&&(at, _)| at < t_ps)
        .map(|&(at, change)| change * (1.0 - (-(t_ps - at) / tau_ps).exp()))
        .sum::<f64>()
        + start
}

/// docs/engine.md's worst error of an exponential whose step is capped at `step_ps`, under
/// its tolerance: `x^2 / (12e)` of the swing, `x` the step over the time constant.
fn capped(swing: f64, step_ps: f64, tau_ps: f64) -> f64 {
    let x = step_ps / tau_ps;
    swing * x * x / (12.0 * std::f64::consts::E)
}

/// docs/engine.md's worst error of one exponential whose steps ride its tolerance: 9.5e-5 of
/// the swing.
fn riding(swing: f64) -> f64 {
    9.5e-5 * swing
}

/// 1 mA into 1 kohm parallel 2 kohm, gmin included. The solve matched the formula exactly.
#[test]
fn the_current_divider_example_matches_its_closed_form() {
    let results = simulate(&example("i-divider"));
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let op = results.observations().op().unwrap();
    let value = |key: &str| op.values[&key.parse().unwrap()].get();
    let want = 1e-3 / (1.0 / 1000.0 + 1.0 / 2000.0 + 1e-12);
    let error = (value("#net.vt") - want).abs();
    assert!(error <= 1e-15, "{error:e}");
    assert_eq!(value("#i1.i"), -1e-3);
}

/// 0 to 1 V at 200 us, back at 400 us, every 600 us, through 1 kohm into 100 nF: tau is
/// 100 us. `max_step_ps` is the time constant, so the tolerance sets the steps.
#[test]
fn the_voltage_pulse_example_follows_its_closed_form_at_every_row() {
    let results = simulate(&example("v-pulse-rc"));
    let trace = trace(&results);
    let rows = rows(trace);
    let jumps = [(2e8, 1.0), (4e8, -1.0), (8e8, 1.0), (10e8, -1.0)];
    let vout = column(trace, "#net.vout");
    let worst = rows
        .iter()
        .zip(vout)
        .map(|(&t, &v)| (v - charge(0.0, &jumps, 1e8, t as f64)).abs())
        .fold(0.0, f64::max);
    assert!(worst <= riding(1.0), "{worst:e}");
    let vin = column(trace, "#net.vin");
    let at = |t: u64| vin[rows.binary_search(&t).unwrap()];
    for (jump, change) in jumps {
        let jump = jump as u64;
        let old = if change > 0.0 { 0.0 } else { 1.0 };
        assert_eq!(at(jump), old, "the row at {jump} holds the old value");
        assert_eq!(at(jump + 1_000_000), old + change, "{jump}");
    }
}

/// 0.2 mA to 1 mA at 1 ms for 2 ms, every 5 ms, into 1 kohm parallel 1 uF: tau is 1 ms.
/// The rise at 16 ms is the stop. Its 20 us `max_step_ps` caps the step.
#[test]
fn the_current_pulse_example_follows_its_closed_form_at_every_row() {
    let json = example("current-pulse");
    let results = simulate(&json);
    let trace = trace(&results);
    let rows = rows(trace);
    let jumps = [1e9, 3e9, 6e9, 8e9, 11e9, 13e9]
        .iter()
        .enumerate()
        .map(|(k, &at)| (at, if k % 2 == 0 { 0.8 } else { -0.8 }))
        .collect::<Vec<_>>();
    let vout = column(trace, "#net.vout");
    let worst = rows
        .iter()
        .zip(vout)
        .map(|(&t, &v)| (v - charge(0.2, &jumps, 1e9, t as f64)).abs())
        .fold(0.0, f64::max);
    assert!(worst <= capped(0.8, 2e7, 1e9), "{worst:e}");
    let netlist = sim_core::netlist::parse(json.as_bytes()).unwrap();
    let source = netlist
        .parts()
        .find_map(|(_, part)| match &part.model {
            Model::CurrentSource(source) => Some(source),
            _ => None,
        })
        .unwrap();
    let current = column(trace, "#i1.i");
    for (&t, &i) in rows.iter().zip(current) {
        assert_eq!(i, -source.current.before(Time::from_ps(t)), "{t}");
    }
    assert_eq!(*rows.last().unwrap(), 16_000_000_000);
    assert_eq!(*current.last().unwrap(), -2e-4);
}
