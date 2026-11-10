#[allow(dead_code)]
mod common;

use common::{column, resistor, simulate, source, span, trace, transient};
use orbe_engine_core::logic::Level;
use orbe_engine_core::name::Ref;
use orbe_engine_core::run_folder::Results;

const PULL_IN: f64 = 0.05357142857142857;
const DROP_OUT: f64 = 0.007142857142857143;

fn relay(name: &str, pins: [&str; 5], henry: f64) -> String {
    let [coil_a, coil_b, com, no, nc] = pins;
    let value =
        |v: f64, unit: &str| format!(r#"{{ "value": {v:?}, "unit": "{unit}", "source": "user" }}"#);
    format!(
        r#"{{ "name": "{name}", "kind": "relay",
          "pins": {{ "coil_a": "{coil_a}", "coil_b": "{coil_b}", "com": "{com}", "no": "{no}", "nc": "{nc}" }},
          "inductance": {}, "coil_resistance": {}, "pull_in_current": {}, "drop_out_current": {},
          "on_resistance": {}, "off_resistance": {} }}"#,
        value(henry, "H"),
        value(70.0, "ohm"),
        value(PULL_IN, "A"),
        value(DROP_OUT, "A"),
        value(0.1, "ohm"),
        value(1e8, "ohm"),
    )
}

/// A `steps` source on `vin` that takes each `(t_ps, volts)` in turn from 0 V.
fn steps(levels: &[(u64, f64)]) -> String {
    let events: Vec<String> = levels
        .iter()
        .map(|(at, volts)| {
            format!(
                r#"{{ "t_ps": {at}, "level": {{ "value": {volts:?}, "unit": "V", "source": "user" }} }}"#
            )
        })
        .collect();
    format!(
        r#"{{ "name": "v2", "kind": "voltage_source", "pins": {{ "p": "vin", "n": "gnd" }},
          "voltage": {{ "kind": "steps", "initial": {{ "value": 0.0, "unit": "V", "source": "user" }},
                       "events": [{}] }} }}"#,
        events.join(", ")
    )
}

fn switch(name: &str, a: &str, b: &str, events: &[(u64, &str)]) -> String {
    let events: Vec<String> = events
        .iter()
        .map(|(at, state)| format!(r#"{{ "t_ps": {at}, "state": "{state}" }}"#))
        .collect();
    format!(
        r#"{{ "name": "{name}", "kind": "switch", "pins": {{ "a": "{a}", "b": "{b}" }},
          "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }},
          "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
          "initial": "open", "events": [{}], "bounce": null }}"#,
        events.join(", ")
    )
}

fn edges(results: &Results, signal: &str) -> Vec<(u64, Level)> {
    let logic = trace(results).logic().unwrap();
    let signal: Ref = signal.parse().unwrap();
    let index = logic.signals().iter().position(|s| *s == signal).unwrap();
    logic
        .changes()
        .iter()
        .filter(|change| change.signal == index)
        .map(|change| (change.at.as_ps(), change.level))
        .collect()
}

fn at(results: &Results, reference: &str, t_ps: u64) -> f64 {
    let trace = trace(results);
    let row = trace.time().iter().position(|t| t.as_ps() == t_ps).unwrap();
    column(trace, reference)[row]
}

/// Picoseconds after a step at which a coil current heading from `from` to `to` with time
/// constant `tau` crosses `level`.
fn crossing(tau: f64, from: f64, to: f64, level: f64) -> f64 {
    tau * libm::log((from - to) / (level - to)) * 1e12
}

/// A 1 mH coil behind its 70 ohm, a 14 us time constant, settles inside each 1 ms level. The
/// contact closes above pull-in and opens at drop-out or below, for a current of either sign.
/// Levels between the two keep the contact as it was. `no` carries the supply while it is closed
/// and `nc` while it is open.
#[test]
fn contact_follows_hysteresis() {
    let ms = 1_000_000_000;
    let volts = [3.0, 4.0, 1.0, 0.4, 3.0, -4.0, -1.0, 0.0];
    let levels: Vec<(u64, f64)> = (1..).map(|k| k * ms).zip(volts).collect();
    let json = transient(
        9 * ms,
        10_000_000,
        r#""gnd", "vin", "vcc", "no", "nc""#,
        &[
            &steps(&levels),
            &source(5.0),
            &relay("k1", ["vin", "gnd", "vcc", "no", "nc"], 1e-3),
            &resistor("rno", "no", "gnd", 1000.0),
            &resistor("rnc", "nc", "gnd", 1000.0),
        ],
    );
    let results = simulate(&json);
    let tau = 1e-3 / 70.0;
    let amps = |k: usize| volts[k] / 70.0;
    let want = [
        (2, crossing(tau, amps(0), amps(1), PULL_IN), Level::High),
        (4, crossing(tau, amps(2), amps(3), DROP_OUT), Level::Low),
        (6, crossing(tau, amps(4), amps(5), -PULL_IN), Level::High),
        (8, crossing(tau, amps(6), 0.0, -DROP_OUT), Level::Low),
    ];
    let got = edges(&results, "#k1.logic.contact");
    assert_eq!(got.len(), want.len(), "{got:?}");
    for ((at, level), (step, after, want_level)) in got.iter().zip(want) {
        let want_at = (step * ms) as f64 + after;
        assert_eq!(*level, want_level, "{got:?}");
        assert!(
            (*at as f64 - want_at).abs() <= 1e-3 * tau * 1e12,
            "{at} ps vs the closed form's {want_at} ps"
        );
    }
    let closed = 5.0 / 1000.1;
    for (t_ms, no, nc) in [
        (1.5, 0.0, closed),
        (2.5, closed, 0.0),
        (3.5, closed, 0.0),
        (4.5, 0.0, closed),
        (5.5, 0.0, closed),
        (6.5, closed, 0.0),
        (7.5, closed, 0.0),
        (8.5, 0.0, closed),
    ] {
        let t_ps = (t_ms * 1e9) as u64;
        for (reference, want) in [("#rno.i", no), ("#rnc.i", nc)] {
            let got = at(&results, reference, t_ps);
            assert!(
                (got - want).abs() <= 1e-6,
                "{reference} at {t_ms} ms: {got}"
            );
        }
    }
}

/// A button closes the coil onto the supply at 2 ms, and the contact closing at about 12 ms
/// puts the supply on the coil beside it. The button opens at 30 ms and the coil keeps its
/// current through its own contact, so the contact never opens.
#[test]
fn latching_relay_settles() {
    let ms = 1_000_000_000;
    let json = transient(
        60 * ms,
        ms,
        r#""gnd", "vcc", "coil", "nc""#,
        &[
            &source(5.0),
            &switch(
                "s1",
                "vcc",
                "coil",
                &[(2 * ms, "closed"), (30 * ms, "open")],
            ),
            &relay("k1", ["coil", "gnd", "vcc", "coil", "nc"], 0.505),
            &resistor("rnc", "nc", "gnd", 1000.0),
        ],
    );
    let results = simulate(&json);
    let got = edges(&results, "#k1.logic.contact");
    let tau = 0.505 / 70.1;
    let close = 2.0e9 + crossing(tau, 0.0, 5.0 / 70.1, PULL_IN);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, Level::High);
    assert!(
        (got[0].0 as f64 - close).abs() <= 1e-3 * tau * 1e12,
        "{got:?} vs {close}"
    );
    // Eight time constants after the button closed, through the contact alone.
    let held = at(&results, "#k1.i", 60 * ms);
    assert!((held / (5.0 / 70.1) - 1.0).abs() <= 1e-3, "{held}");
}

/// The contact feeds its own coil through `nc`, and the coil is powered at t = 0. Open, the
/// coil carries 71 mA, above pull-in. Closed, it carries next to nothing, below drop-out. No
/// state agrees with its own solve.
#[test]
fn a_contact_that_breaks_its_own_coil_current_is_unsettled() {
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "op": {{}} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "nc", "no"],
          "parts": [{}, {}]
        }}"#,
        source(5.0),
        relay("k1", ["nc", "gnd", "vcc", "no", "nc"], 0.505),
    );
    let results = simulate(&json);
    let [found] = results.diagnostics() else {
        panic!("{:?}", results.diagnostics());
    };
    assert_eq!(
        serde_json::to_string(found).unwrap(),
        r##"{"code":"chip.unsettled","severity":"error","refs":["#k1"],"t_ps":0,"message":"the chips kept changing state for 8 solves at 0 ps","data":{"rounds":8}}"##
    );
    assert!(results.observations().op().is_none());
}

/// The same contact, powered at 1 ms rather than at t = 0. The coil holds its current across
/// each move, so the run has no diagnostic and the contact keeps buzzing. Each break puts the
/// held current across `off_resistance`, far beyond the supply.
#[test]
fn a_series_nc_contact_powered_later_buzzes() {
    let ms = 1_000_000_000;
    let json = transient(
        40 * ms,
        ms,
        r#""gnd", "vin", "nc", "no""#,
        &[
            &steps(&[(ms, 5.0)]),
            &relay("k1", ["nc", "gnd", "vin", "no", "nc"], 0.505),
            &resistor("rno", "no", "gnd", 1000.0),
        ],
    );
    let results = simulate(&json);
    let got = edges(&results, "#k1.logic.contact");
    let closes: Vec<u64> = got
        .iter()
        .filter(|(_, level)| *level == Level::High)
        .map(|(at, _)| *at)
        .collect();
    assert!(closes.len() >= 3, "{got:?}");
    let tau = 0.505 / 70.1;
    let top = 5.0 / 70.1;
    let period = tau * libm::log((top - DROP_OUT) / (top - PULL_IN)) * 1e12;
    for pair in closes.windows(2) {
        let gap = (pair[1] - pair[0]) as f64;
        assert!(
            (gap / period - 1.0).abs() <= 1e-2,
            "{gap} ps vs {period} ps, edges {got:?}"
        );
    }
    let trace = trace(&results);
    let nc: Ref = "#net.nc".parse().unwrap();
    let index = trace.columns().iter().position(|r| *r == nc).unwrap();
    let break_v = trace
        .min_column(index)
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    assert!(break_v < -100.0 * 5.0, "break voltage {break_v} V");
}

/// `#k1.i` at `t_ps`. 5 V onto 0.505 H behind 70 ohm at 1 ms, then 0 V at 30 ms.
fn coil_closed_form(t_ps: f64) -> f64 {
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

/// The coil is the inductor's, held to the same tolerance. Every row stays within 1e-4 of the
/// swing of its closed form. The contact closes and opens where that current crosses pull-in
/// and drop-out.
#[test]
fn a_relay_coil_follows_its_closed_form() {
    let ms = 1_000_000_000;
    let json = transient(
        50 * ms,
        ms,
        r#""gnd", "vin", "no", "nc""#,
        &[
            &steps(&[(ms, 5.0), (30 * ms, 0.0)]),
            &relay("k1", ["vin", "gnd", "gnd", "no", "nc"], 0.505),
        ],
    );
    let results = simulate(&json);
    let trace = trace(&results);
    let current = column(trace, "#k1.i");
    let swing = span(current);
    let worst = trace
        .time()
        .iter()
        .zip(current)
        .map(|(at, i)| (i - coil_closed_form(at.as_ps() as f64)).abs() / swing)
        .fold(0.0f64, f64::max);
    assert!(worst <= 1e-4, "{worst:e} of the swing");
    let tau = 0.505 / 70.0;
    let top = 5.0 / 70.0;
    let off = coil_closed_form(30e9);
    let want = [
        1e9 + crossing(tau, 0.0, top, PULL_IN),
        30e9 + crossing(tau, off, 0.0, DROP_OUT),
    ];
    let got = edges(&results, "#k1.logic.contact");
    assert_eq!(got.len(), 2, "{got:?}");
    for ((at, _), want) in got.iter().zip(want) {
        assert!(
            (*at as f64 - want).abs() <= 1e-3 * tau * 1e12,
            "{at} ps vs {want} ps"
        );
    }
}

/// `coil_b` is on a net that a capacitor alone would leave floating. The coil's DC path joins
/// it to the supply, so the operating point reports no `net.floating`.
#[test]
fn a_net_reached_only_through_the_coil_has_a_dc_path() {
    let capacitor = r#"{ "name": "c1", "kind": "capacitor", "pins": { "a": "k", "b": "gnd" },
          "capacitance": { "value": 1e-6, "unit": "F", "source": "user" }, "initial": null }"#;
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "op": {{}} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "k", "no", "nc"],
          "parts": [{}, {}, {}]
        }}"#,
        source(5.0),
        relay("k1", ["vcc", "k", "vcc", "no", "nc"], 0.505),
        capacitor,
    );
    let results = simulate(&json);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
}

/// Accepted steps after t = 0. An error diagnostic is a failed solve, and a run with no
/// landings is hollow, so neither one can pass a step bound.
fn accepted_steps(rel: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    let bytes = std::fs::read(&path).unwrap();
    let netlist = orbe_engine_core::netlist::parse(&bytes).unwrap();
    let (results, stats) = orbe_circuit::simulate_stats(&netlist);
    assert!(
        results
            .diagnostics()
            .iter()
            .all(|d| d.severity() != orbe_engine_core::diagnostic::Severity::Error),
        "{:?}",
        results.diagnostics()
    );
    assert!(stats.landings > 0, "no accepted step after t = 0");
    stats.landings
}

/// Measured 1340 accepted steps. The bound is 10 percent above that count, in integer math.
#[test]
fn relay_flyback_stays_near_its_step_count() {
    let measured = 1_340;
    let steps = accepted_steps("../tests/circuits/relay-flyback/netlist.json");
    eprintln!("relay-flyback steps {steps}");
    assert!(
        steps <= measured + measured / 10,
        "{steps} steps, bound {}",
        measured + measured / 10
    );
}

/// `#net.b` from just after 30.01 ms through 41 ms. A reversal is a step whose neighbors
/// change the voltage in opposite directions. The size is the larger of those two changes.
fn fb_null_window(bytes: &[u8]) -> (u64, f64) {
    let netlist = orbe_engine_core::netlist::parse(bytes).unwrap();
    let results = orbe_circuit::simulate_steps(&netlist);
    let trace = results.trace().unwrap();
    let times: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    let column = trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == "#net.b")
        .unwrap();
    let volts = trace.column(column);
    let mut steps = 0u64;
    let mut reversal = 0.0f64;
    for index in 1..times.len() - 1 {
        if times[index] <= 30_010_000_000 || times[index] > 41_000_000_000 {
            continue;
        }
        steps += 1;
        let left = volts[index] - volts[index - 1];
        let right = volts[index + 1] - volts[index];
        if left * right < 0.0 {
            reversal = reversal.max(left.abs().max(right.abs()));
        }
    }
    (steps, reversal)
}

/// Measured 13868 accepted steps. The bound is 10 percent above that count, in integer math.
#[test]
fn the_bare_inductor_stays_near_its_step_count() {
    let measured = 13_868;
    let steps = accepted_steps("tests/controls/bare-inductor.json");
    eprintln!("bare-inductor steps {steps}");
    assert!(
        steps <= measured + measured / 10,
        "{steps} steps, bound {}",
        measured + measured / 10
    );
}

/// Measured 2665 accepted steps. From 30.01 ms to 41 ms the node no longer reverses by
/// more than 1e-10 V. That window takes 116 steps, and the bound is 1000.
#[test]
fn fb_null_20000_stops_zigzagging_on_net_b() {
    let measured = 2_665;
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/controls/fb-null-20000.json"),
    )
    .unwrap();
    let steps = accepted_steps("tests/controls/fb-null-20000.json");
    let (window, reversal) = fb_null_window(&bytes);
    eprintln!("fb-null window steps {window} reversal {reversal:e}");
    assert!(
        steps <= measured + measured / 10,
        "{steps} steps, bound {}",
        measured + measured / 10
    );
    assert!(window < 1_000, "{window} steps in the window");
    assert!(reversal < 1e-10, "reversal {reversal:e} V");
}

/// Measured 13668 accepted steps. The bound is 10 percent above that count, in integer math.
/// The fixture can carry a power warning, so the run check is the absence of an error.
#[test]
fn the_bare_relay_stays_near_its_step_count() {
    let measured = 13_668;
    let steps = accepted_steps("tests/fixtures/p-relay-npn-bare/netlist.json");
    eprintln!("p-relay-npn-bare steps {steps}");
    assert!(
        steps <= measured + measured / 10,
        "{steps} steps, bound {}",
        measured + measured / 10
    );
}
