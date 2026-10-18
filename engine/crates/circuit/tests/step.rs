mod common;

use common::{
    column, held_capacitor, resistor, simulate, source, source_on, span, trace, transient,
};

/// 5 V through 1 kohm into 10 fF, and a 0.1 ohm switch across it that closes at 1 us.
/// The discharge after the edge has a 1 fs time constant, far under the 1 ps step floor.
const FEMTOSECOND_EDGE: &str = r#"{
  "version": 0,
  "seed": 0,
  "analysis": { "transient": { "stop_ps": 20000000000, "max_step_ps": 100000 } },
  "ground": "gnd",
  "nets": ["gnd", "vcc", "vout"],
  "parts": [
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vcc", "n": "gnd" },
      "voltage": { "value": 5.0, "unit": "V", "source": "user" }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "vcc", "b": "vout" },
      "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    },
    {
      "name": "c1",
      "kind": "capacitor",
      "pins": { "a": "vout", "b": "gnd" },
      "capacitance": { "value": 1e-14, "unit": "F", "source": "user" },
      "initial": null
    },
    {
      "name": "sw1",
      "kind": "switch",
      "pins": { "a": "vout", "b": "gnd" },
      "on_resistance": { "value": 0.1, "unit": "ohm", "source": "user" },
      "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
      "initial": "open",
      "events": [{ "t_ps": 1000000, "state": "closed" }],
      "bounce": null
    }
  ]
}"#;

#[test]
fn a_mode_faster_than_the_step_floor_dies_out_instead_of_ringing() {
    let results = simulate(FEMTOSECOND_EDGE);
    let trace = trace(&results);
    // One row per `max_step_ps` over 20 ms is 200000.
    assert!(trace.rows() < 201_000, "{} rows", trace.rows());
    let vout = column(trace, "#net.vout");
    let settled = 5.0 * 0.1 / 1000.1;
    for (row, (&v, at)) in vout.iter().zip(trace.time()).enumerate() {
        assert!(v >= -1e-9, "row {row} at {at:?}: {v}");
        if at.as_ps() >= 1_001_000 {
            assert!((v - settled).abs() <= 1e-9, "row {row} at {at:?}: {v}");
        }
    }
}

/// 1 V through 1 kohm into 100.1 pF, and a 1 ohm switch across it that closes at 10 ns.
/// It discharges with a 100 ps time constant.
const PICOSECOND_EDGE: &str = r#"{
  "version": 0,
  "seed": 0,
  "analysis": { "transient": { "stop_ps": 15000, "max_step_ps": 1000 } },
  "ground": "gnd",
  "nets": ["gnd", "vcc", "vout"],
  "parts": [
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vcc", "n": "gnd" },
      "voltage": { "value": 1.0, "unit": "V", "source": "user" }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "vcc", "b": "vout" },
      "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    },
    {
      "name": "c1",
      "kind": "capacitor",
      "pins": { "a": "vout", "b": "gnd" },
      "capacitance": { "value": 1.001e-10, "unit": "F", "source": "user" },
      "initial": null
    },
    {
      "name": "sw1",
      "kind": "switch",
      "pins": { "a": "vout", "b": "gnd" },
      "on_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
      "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
      "initial": "open",
      "events": [{ "t_ps": 10000, "state": "closed" }],
      "bounce": null
    }
  ]
}"#;

#[test]
fn a_100_ps_discharge_after_an_edge_stays_on_its_closed_form() {
    let results = simulate(PICOSECOND_EDGE);
    let trace = trace(&results);
    let vout = column(trace, "#net.vout");
    let before = 1e9 / (1e9 + 1000.0);
    let after = 1.0 / 1001.0;
    let mut worst = 0.0f64;
    for (row, at) in trace.time().iter().enumerate() {
        let t = at.as_ps().saturating_sub(10_000) as f64 * 1e-12;
        let v = after + (before - after) * (-t / 1e-10).exp();
        worst = worst.max((vout[row] - v).abs() / (before - after));
    }
    // The trapezoidal rule holds it within 1e-4 of span. Backward Euler alone is near 2e-3.
    assert!(worst <= 1e-4, "{worst}");
}

/// 5 V through 1 kohm into 100 nF, and a 0.1 ohm switch across it that closes at 1 us.
/// It discharges with a time constant just under 10 ns.
const FAST_EDGE: &str = r#"{
  "version": 0,
  "seed": 0,
  "analysis": { "transient": { "stop_ps": 2000000, "max_step_ps": 10000 } },
  "ground": "gnd",
  "nets": ["gnd", "vcc", "vout"],
  "parts": [
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vcc", "n": "gnd" },
      "voltage": { "value": 5.0, "unit": "V", "source": "user" }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "vcc", "b": "vout" },
      "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    },
    {
      "name": "c1",
      "kind": "capacitor",
      "pins": { "a": "vout", "b": "gnd" },
      "capacitance": { "value": 1e-7, "unit": "F", "source": "user" },
      "initial": null
    },
    {
      "name": "sw1",
      "kind": "switch",
      "pins": { "a": "vout", "b": "gnd" },
      "on_resistance": { "value": 0.1, "unit": "ohm", "source": "user" },
      "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
      "initial": "open",
      "events": [{ "t_ps": 1000000, "state": "closed" }],
      "bounce": null
    }
  ]
}"#;

/// `#net.vout` and `#c1.i` of `FAST_EDGE` at `t_ps`. The row at the edge holds the open contact.
fn fast_edge_closed_form(t_ps: u64) -> (f64, f64) {
    let thevenin = |switch: f64| {
        (
            5.0 * switch / (1000.0 + switch),
            1000.0 * switch / (1000.0 + switch),
        )
    };
    let (before, _) = thevenin(1e9);
    if t_ps <= 1_000_000 {
        return (before, 0.0);
    }
    let (vth, rth) = thevenin(0.1);
    let decay = (-((t_ps - 1_000_000) as f64 * 1e-12) / (rth * 1e-7)).exp();
    (vth + (before - vth) * decay, -(before - vth) / rth * decay)
}

#[test]
fn a_10_ns_discharge_after_an_edge_follows_its_closed_form_from_the_first_row() {
    let results = simulate(FAST_EDGE);
    let trace = trace(&results);
    let (vout, current) = (column(trace, "#net.vout"), column(trace, "#c1.i"));
    let (v_span, i_span) = (span(vout), span(current));
    let mut worst = (0.0f64, 0.0f64);
    let mut early = 0;
    for (row, at) in trace.time().iter().enumerate() {
        let at = at.as_ps();
        let (v, i) = fast_edge_closed_form(at);
        worst.0 = worst.0.max((vout[row] - v).abs() / v_span);
        worst.1 = worst.1.max((current[row] - i).abs() / i_span);
        if at > 1_000_000 && at <= 1_050_000 {
            early += 1;
        }
    }
    assert!(early >= 10, "{early} rows in the first 50 ns");
    // docs/netlist.md promises under 2e-5 for this discharge.
    assert!(worst.0 <= 2e-5 && worst.1 <= 2e-5, "{worst:?}");
}

/// 1 uF held at 5 V at the operating point, then discharging through 1 kohm.
const HELD_DISCHARGE: &str = r#"{
  "version": 0,
  "seed": 0,
  "analysis": { "transient": { "stop_ps": 5000000000, "max_step_ps": 10000000 } },
  "ground": "gnd",
  "nets": ["gnd", "vout"],
  "parts": [
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "vout", "b": "gnd" },
      "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    },
    {
      "name": "c1",
      "kind": "capacitor",
      "pins": { "a": "vout", "b": "gnd" },
      "capacitance": { "value": 1e-6, "unit": "F", "source": "user" },
      "initial": { "value": 5.0, "unit": "V", "source": "user" }
    }
  ]
}"#;

#[test]
fn a_held_capacitor_discharges_along_its_closed_form() {
    let results = simulate(HELD_DISCHARGE);
    let trace = trace(&results);
    let (vout, current) = (column(trace, "#net.vout"), column(trace, "#c1.i"));
    // At t = 0 the solved current feeds r1 and gmin's 1e-12 S. The hold sits behind
    // 1 fs / 1 uF ohm, so that 5 mA moves it by 5 pV, and the current by 5 fA.
    assert!(
        (vout[0] - 5.0).abs() <= 5e-3 * 1e-15 / 1e-6 + 1e-15,
        "{}",
        vout[0]
    );
    assert!((current[0] + 5e-3 + 5e-12).abs() <= 1e-14, "{}", current[0]);
    let (v_span, i_span) = (span(vout), span(current));
    let mut worst = (0.0f64, 0.0f64);
    for (row, at) in trace.time().iter().enumerate() {
        let decay = (-(at.as_ps() as f64 * 1e-12) / 1e-3).exp();
        worst.0 = worst.0.max((vout[row] - 5.0 * decay).abs() / v_span);
        worst.1 = worst.1.max((current[row] + 5e-3 * decay).abs() / i_span);
    }
    assert!(worst.0 <= 1e-5 && worst.1 <= 1e-5, "{worst:?}");
}

#[test]
fn a_10_ps_discharge_after_an_edge_is_within_the_documented_floor_error() {
    let results = simulate(&PICOSECOND_EDGE.replace("1.001e-10", "1.001e-11"));
    let trace = trace(&results);
    let vout = column(trace, "#net.vout");
    let before = 1e9 / (1e9 + 1000.0);
    let after = 1.0 / 1001.0;
    let mut worst = 0.0f64;
    for (row, at) in trace.time().iter().enumerate() {
        let t = at.as_ps().saturating_sub(10_000) as f64 * 1e-12;
        let v = after + (before - after) * (-t / 1e-11).exp();
        worst = worst.max((vout[row] - v).abs() / (before - after));
    }
    // docs/netlist.md gives 1.8e-2 for a 10 ps time constant.
    assert!(worst <= 1.8e-2, "{worst}");
}

/// Runs `slow`, `fast`, and both side by side, each a list of nets besides ground and its parts.
/// `#net.{node}` stays within 2e-5 of `closed` beside the fast mode, and the pair takes no more
/// rows than the two halves alone.
fn assert_a_fast_mode_leaves_a_slow_node_alone(
    run: impl Fn(&str, &[&str]) -> String,
    slow: (&str, &[&str]),
    fast: (&str, &[&str]),
    node: &str,
    closed: impl Fn(f64) -> f64,
) {
    let rows = |nets: &str, parts: &[&str]| simulate(&run(&format!(r#""gnd", {nets}"#), parts));
    let alone = rows(slow.0, slow.1);
    let other = rows(fast.0, fast.1);
    let both_parts: Vec<&str> = slow.1.iter().chain(fast.1).copied().collect();
    let both = rows(&format!("{}, {}", slow.0, fast.0), &both_parts);
    for results in [&alone, &both] {
        let trace = trace(results);
        let values = column(trace, &format!("#net.{node}"));
        let worst = trace
            .time()
            .iter()
            .zip(values)
            .map(|(at, v)| (v - closed(at.as_ps() as f64 * 1e-12)).abs())
            .fold(0.0, f64::max);
        assert!(
            worst <= 2e-5 * span(values),
            "{worst} against span {}",
            span(values)
        );
    }
    let (alone, other, both) = (trace(&alone), trace(&other), trace(&both));
    assert!(
        both.rows() <= alone.rows() + other.rows(),
        "{} rows against {} and {}",
        both.rows(),
        alone.rows(),
        other.rows()
    );
}

#[test]
fn a_30_ps_mode_switched_in_beside_a_10_ns_discharge_leaves_it_on_its_closed_form() {
    assert_a_fast_mode_leaves_a_slow_node_alone(
        |nets, parts| transient(200_000, 5_000, nets, parts),
        (
            r#""vs""#,
            &[
                &held_capacitor("cs", "vs", "gnd", 1e-11, 1.0),
                &resistor("rs", "vs", "gnd", 1000.0),
            ],
        ),
        (
            r#""vcc", "vf""#,
            &[
                &source(1.0),
                &resistor("rf", "vcc", "vf", 1000.0),
                &capacitor("cf", "vf", "gnd", 3.003e-11),
                &closing_switch("sw1", "vf", "gnd", 1.0, 5_000),
            ],
        ),
        "vs",
        |t| (-t / 1e-8).exp(),
    );
}

#[test]
fn a_1_fs_mode_switched_in_beside_a_1_ms_charge_leaves_it_on_its_closed_form() {
    assert_a_fast_mode_leaves_a_slow_node_alone(
        |nets, parts| transient(5_000_000_000, 5_000_000_000, nets, parts),
        (
            r#""vcc", "vslow""#,
            &[
                &source(5.0),
                &resistor("r2", "vcc", "vslow", 1000.0),
                &held_capacitor("c2", "vslow", "gnd", 1e-6, 0.0),
            ],
        ),
        (
            r#""vdd", "vfast""#,
            &[
                &source_on("v2", "vdd", 5.0),
                &resistor("r1", "vdd", "vfast", 1000.0),
                &capacitor("c1", "vfast", "gnd", 1e-14),
                &closing_switch("sw1", "vfast", "gnd", 0.1, 1_000_000),
            ],
        ),
        "vslow",
        |t| 5.0 * (1.0 - (-t / 1e-3).exp()),
    );
}

fn capacitor(name: &str, a: &str, b: &str, farad: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "capacitor", "pins": {{ "a": "{a}", "b": "{b}" }},
          "capacitance": {{ "value": {farad:?}, "unit": "F", "source": "user" }}, "initial": null }}"#
    )
}

/// Open at 1 Gohm, closed at `ohm` from `t_ps`.
fn closing_switch(name: &str, a: &str, b: &str, ohm: f64, t_ps: u64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "switch", "pins": {{ "a": "{a}", "b": "{b}" }},
          "on_resistance": {{ "value": {ohm:?}, "unit": "ohm", "source": "user" }},
          "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
          "initial": "open", "events": [{{ "t_ps": {t_ps}, "state": "closed" }}], "bounce": null }}"#
    )
}

/// `x' = a x + b` from `x0` at t = 0, for an `a` with real distinct eigenvalues, as two RC
/// networks coupled through a resistor have.
fn two_poles(a: [[f64; 2]; 2], b: [f64; 2], x0: [f64; 2]) -> impl Fn(f64) -> [f64; 2] {
    let [[a11, a12], [a21, a22]] = a;
    let det = a11 * a22 - a12 * a21;
    let settled = [
        (a12 * b[1] - a22 * b[0]) / det,
        (a21 * b[0] - a11 * b[1]) / det,
    ];
    let half = (a11 + a22) / 2.0;
    let root = (half * half - det).sqrt();
    let poles = [half + root, half - root];
    // Each pole's eigenvector is (a12, pole - a11).
    let [p, q] = poles.map(|pole| [a12, pole - a11]);
    let w = [x0[0] - settled[0], x0[1] - settled[1]];
    let cross = p[0] * q[1] - q[0] * p[1];
    let k = [
        (w[0] * q[1] - q[0] * w[1]) / cross,
        (p[0] * w[1] - w[0] * p[1]) / cross,
    ];
    move |t| {
        let [e0, e1] = poles.map(|pole| (pole * t).exp());
        [
            settled[0] + k[0] * p[0] * e0 + k[1] * q[0] * e1,
            settled[1] + k[0] * p[1] * e0 + k[1] * q[1] * e1,
        ]
    }
}

/// The worst error over `swing` of the nets `nodes` from `after_ps` past `edge_ps`, against
/// `a` and `b` started from the row at the edge, which holds the voltages before it.
fn two_pole_error(
    trace: &sim_core::trace::Trace,
    nodes: [&str; 2],
    edge_ps: u64,
    after_ps: u64,
    (a, b): ([[f64; 2]; 2], [f64; 2]),
    swing: f64,
) -> f64 {
    let columns = nodes.map(|node| column(trace, &format!("#net.{node}")));
    let edge = trace
        .time()
        .iter()
        .position(|at| at.as_ps() == edge_ps)
        .unwrap();
    let closed = two_poles(a, b, columns.map(|values| values[edge]));
    trace
        .time()
        .iter()
        .enumerate()
        .filter(|(_, at)| at.as_ps() >= edge_ps + after_ps)
        .flat_map(|(row, at)| {
            let want = closed((at.as_ps() - edge_ps) as f64 * 1e-12);
            [0, 1].map(|n| (columns[n][row] - want[n]).abs())
        })
        .fold(0.0, f64::max)
        / swing
}

/// 10 pF charged to 5 V through 1 Mohm, switched at 1 ns through `share` ohm onto 1 nF at 0 V.
/// Then both charge through the 1 Mohm, with a time constant near 1 ms.
fn full_swing(share: f64) -> (sim_core::run_folder::Results, ([[f64; 2]; 2], [f64; 2])) {
    let results = simulate(&transient(
        5_000_000_000,
        5_000_000_000,
        r#""gnd", "vcc", "va", "m""#,
        &[
            &source(5.0),
            &resistor("r2", "vcc", "va", 1e6),
            &capacitor("c2", "va", "gnd", 1e-11),
            &closing_switch("sw1", "va", "m", share, 1_000),
            &held_capacitor("c3", "m", "gnd", 1e-9, 0.0),
        ],
    ));
    let (c2, c3) = (1e-11, 1e-9);
    let a = [
        [-(1.0 / 1e6 + 1.0 / share) / c2, 1.0 / (share * c2)],
        [1.0 / (share * c3), -1.0 / (share * c3)],
    ];
    (results, (a, [5.0 / (1e6 * c2), 0.0]))
}

#[test]
fn a_stiff_capacitor_whose_own_voltage_keeps_moving_stays_on_its_closed_form() {
    // A 2 ohm share is a 20 ps mode, so c2 turns stiff. A 200 kohm share is 2 us, and nothing does.
    let (stiff, closed) = full_swing(2.0);
    let (smooth, smooth_closed) = full_swing(2e5);
    let (stiff, smooth) = (trace(&stiff), trace(&smooth));
    // Held on backward Euler for the whole charge, the pair is 2.3e-4 of the swing off.
    let worst = two_pole_error(stiff, ["va", "m"], 1_000, 1_000, closed, 4.95);
    assert!(worst <= 2e-5, "{worst}");
    let control = two_pole_error(smooth, ["va", "m"], 1_000, 0, smooth_closed, 4.95);
    assert!(control <= 2e-5, "{control}");
    assert!(
        stiff.rows() <= 2 * smooth.rows(),
        "{} rows against {}",
        stiff.rows(),
        smooth.rows()
    );
}

#[test]
fn stiff_capacitors_that_share_a_slow_mode_stay_on_its_closed_form() {
    // Two 100 pF at 5 V and 0 V join through 0.1 ohm at 1 us, a 5 ps mode that makes both
    // stiff. Then the pair discharges through 2 Mohm with a 400 us time constant.
    let pair = simulate(&transient(
        2_000_000_000,
        2_000_000_000,
        r#""gnd", "nf", "ns""#,
        &[
            &held_capacitor("cf", "nf", "gnd", 1e-10, 5.0),
            &held_capacitor("cs", "ns", "gnd", 1e-10, 0.0),
            &resistor("rs", "ns", "gnd", 2e6),
            &closing_switch("sw", "nf", "ns", 0.1, 1_000_000),
        ],
    ));
    // The same discharge on one 200 pF from 2.5 V.
    let alone = simulate(&transient(
        2_000_000_000,
        2_000_000_000,
        r#""gnd", "n""#,
        &[
            &held_capacitor("c", "n", "gnd", 2e-10, 2.5),
            &resistor("rs", "n", "gnd", 2e6),
        ],
    ));
    let (pair, alone) = (trace(&pair), trace(&alone));
    let c = 1e-10;
    let a = [
        [-1.0 / (0.1 * c), 1.0 / (0.1 * c)],
        [1.0 / (0.1 * c), -(1.0 / 0.1 + 1.0 / 2e6) / c],
    ];
    // Held on backward Euler for the whole discharge, the pair is 2.3e-4 of the swing off.
    let worst = two_pole_error(pair, ["nf", "ns"], 1_000_000, 1_000, (a, [0.0, 0.0]), 2.5);
    assert!(worst <= 2e-5, "{worst}");
    assert!(
        pair.rows() <= 2 * alone.rows(),
        "{} rows against {}",
        pair.rows(),
        alone.rows()
    );
}
