mod common;

use common::{
    column, held_capacitor, resistor, simulate, source, source_on, span, trace, transient,
};
use sim_core::logic::Level;
use sim_core::name::Ref;
use sim_core::run_folder::Results;

const LOW_DROP: f64 = 1.0 / 60.0;

fn op_value(results: &Results, reference: &str) -> f64 {
    let op = results.observations().op.as_ref().unwrap();
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

fn netlist(analysis: &str, nets: &str, parts: &[String]) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {analysis},
      "ground": "gnd",
      "nets": [{nets}],
      "parts": [{}]
    }}"#,
        parts.join(",")
    )
}

const OP: &str = r#"{ "op": {} }"#;

/// An NE555 with `examples/ne555-led`'s values. Pins in package order:
/// gnd, trig, out, reset, cont, thres, disch, vcc.
fn ne555(name: &str, pins: [&str; 8]) -> String {
    let [gnd, trig, out, reset, cont, thres, disch, vcc] = pins;
    let value =
        |v: f64, unit: &str| format!(r#"{{ "value": {v:?}, "unit": "{unit}", "source": "user" }}"#);
    format!(
        r#"{{ "name": "{name}", "kind": "ne555",
          "pins": {{ "gnd": "{gnd}", "trig": "{trig}", "out": "{out}", "reset": "{reset}",
                    "cont": "{cont}", "thres": "{thres}", "disch": "{disch}", "vcc": "{vcc}" }},
          "ladder_resistance": {}, "reset_threshold": {},
          "discharge": {{ "on_resistance": {}, "off_resistance": {} }},
          "supply_resistance": {},
          "output": {{ "high_drop": {}, "high_resistance": {}, "low_drop": {}, "low_resistance": {} }} }}"#,
        value(5000.0, "ohm"),
        value(0.7, "V"),
        value(18.75, "ohm"),
        value(2.5e8, "ohm"),
        value(1875.0, "ohm"),
        value(0.9, "V"),
        value(8.0, "ohm"),
        value(LOW_DROP, "V"),
        value(50.0 / 3.0, "ohm"),
    )
}

/// TRIG and THRES on ground and RESET on `reset`, 5 V on VCC, nothing on OUT.
fn latch_at_rest(reset_volts: f64) -> Results {
    simulate(&netlist(
        OP,
        r#""gnd", "vcc", "rst", "out", "cont", "dis""#,
        &[
            source(5.0),
            source_on("v2", "rst", reset_volts),
            ne555(
                "u1",
                ["gnd", "gnd", "out", "rst", "cont", "gnd", "dis", "vcc"],
            ),
        ],
    ))
}

#[test]
fn the_operating_point_settles_the_latch_against_its_levels() {
    // Power-on resets the latch. TRIG under 1/3 VCC then sets it, so OUT is high.
    let results = latch_at_rest(5.0);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let out = op_value(&results, "#net.out");
    assert!((out - 4.1).abs() <= 1e-9, "{out}");
    // gmin on CONT and on the lower node pulls CONT about 1.4e-8 V under 2/3 VCC.
    let cont = op_value(&results, "#net.cont");
    assert!((cont - 10.0 / 3.0).abs() <= 2e-8, "{cont}");
    // The discharge is off, 250 Mohm to GND, and DIS has no other path.
    assert!(op_value(&results, "#net.dis").abs() <= 1e-12);
    // 3 mA, SLFS022K's output-low supply current. Unloaded, the output stage draws nothing.
    // gmin on the node rows adds about 1.2e-11 A.
    let supply = 5.0 / 1875.0 + 5.0 / 15_000.0;
    let drawn = -op_value(&results, "#v1.i");
    assert!(
        (drawn - supply).abs() <= 1e-8 * supply,
        "{drawn} vs {supply}"
    );
}

#[test]
fn reset_at_or_under_its_threshold_holds_the_latch_reset() {
    for volts in [0.0, 0.5, 0.7] {
        let out = op_value(&latch_at_rest(volts), "#net.out");
        assert!(
            (out - LOW_DROP).abs() <= 1e-9,
            "RESET at {volts} V: OUT {out}"
        );
    }
    let out = op_value(&latch_at_rest(0.71), "#net.out");
    assert!((out - 4.1).abs() <= 1e-9, "{out}");
}

#[test]
fn a_chip_with_no_state_it_agrees_with_is_unsettled() {
    // OUT drives TRIG and THRES. Low sets the latch and high resets it, with nothing to slow it.
    let results = simulate(&netlist(
        OP,
        r#""gnd", "vcc", "out", "cont", "dis""#,
        &[
            source(5.0),
            ne555(
                "u1",
                ["gnd", "out", "out", "vcc", "cont", "out", "dis", "vcc"],
            ),
        ],
    ));
    let [found] = results.diagnostics() else {
        panic!("{:?}", results.diagnostics());
    };
    assert_eq!(
        serde_json::to_string(found).unwrap(),
        r##"{"code":"chip.unsettled","severity":"error","refs":["#u1"],"t_ps":0,"message":"the chips kept changing state for 8 solves at 0 ps","data":{"rounds":8}}"##
    );
    assert!(results.observations().op.is_none());
}

#[test]
fn a_chip_adds_no_columns_and_a_net_on_its_input_alone_floats() {
    let results = simulate(&netlist(
        OP,
        r#""gnd", "vcc", "trig", "out", "cont", "dis""#,
        &[
            source(5.0),
            ne555(
                "u1",
                ["gnd", "trig", "out", "vcc", "cont", "gnd", "dis", "vcc"],
            ),
        ],
    ));
    let codes: Vec<_> = results
        .diagnostics()
        .iter()
        .map(|d| (d.code(), d.refs()))
        .collect();
    assert_eq!(
        codes,
        [("net.floating", vec!["#net.trig".parse().unwrap()])]
    );
    let keys: Vec<String> = results
        .observations()
        .op
        .as_ref()
        .unwrap()
        .values
        .keys()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        keys,
        [
            "#net.cont",
            "#net.dis",
            "#net.gnd",
            "#net.out",
            "#net.trig",
            "#net.vcc",
            "#v1.i",
            "#v1.v",
            "#v1.p"
        ]
    );
}

/// `signal`'s edges, as `(t_ps, level)`.
fn edges(results: &Results, signal: &str) -> Vec<(u64, Level)> {
    let logic = results.observations().logic.as_ref().unwrap();
    let signal: Ref = signal.parse().unwrap();
    let index = logic.signals().iter().position(|s| *s == signal).unwrap();
    logic
        .changes()
        .iter()
        .filter(|change| change.signal == index)
        .map(|change| (change.at.as_ps(), change.level))
        .collect()
}

fn row(results: &Results, at: u64) -> usize {
    let times: Vec<u64> = trace(results).time().iter().map(|t| t.as_ps()).collect();
    times
        .binary_search(&at)
        .unwrap_or_else(|_| panic!("no row at {at} ps"))
}

/// 100 kohm and 1 uF from 5 V hold RESET low at power-on. TRIG and THRES are on ground.
fn power_on_reset(stop_ps: u64) -> Results {
    simulate(&transient(
        stop_ps,
        1_000_000_000,
        r#""gnd", "vcc", "rst", "out", "cont", "dis""#,
        &[
            &source(5.0),
            &resistor("rr", "vcc", "rst", 100e3),
            &held_capacitor("cr", "rst", "gnd", 1e-6, 0.0),
            &ne555(
                "u1",
                ["gnd", "gnd", "out", "rst", "cont", "gnd", "dis", "vcc"],
            ),
        ],
    ))
}

#[test]
fn reset_release_is_located_on_its_closed_form() {
    let results = power_on_reset(40_000_000_000);
    let trace = trace(&results);
    // OUT stays low while RESET holds the latch, though TRIG is under its threshold all along.
    assert!((column(trace, "#net.out")[0] - LOW_DROP).abs() <= 1e-9);
    let rises = edges(&results, "#u1.logic.out");
    let [(at, Level::High)] = rises[..] else {
        panic!("{rises:?}");
    };
    assert_eq!(edges(&results, "#u1.logic.reset"), [(at, Level::High)]);
    let tau = 100e3 * 1e-6;
    let closed = tau * (5.0_f64 / (5.0 - 0.7)).ln() * 1e12;
    assert!(
        (at as f64 / closed - 1.0).abs() <= 1e-5,
        "{at} ps vs {closed} ps"
    );
    let reset = column(trace, "#net.rst");
    let k = row(&results, at);
    let past = reset[k] - 0.7;
    assert!(past > 0.0 && past <= 1e-8, "{past} V past the threshold");
    assert!(reset[k - 1] <= 0.7, "the row before is past already");
    // The row at the edge holds the old drive, the next row the new one.
    assert!(column(trace, "#net.out")[k] < 0.1);
    assert!(column(trace, "#net.out")[k + 1] > 4.0);
    for (k, (&v, t)) in reset.iter().zip(trace.time()).enumerate() {
        let want = 5.0 * (1.0 - (-(t.as_ps() as f64) * 1e-12 / tau).exp());
        assert!(
            (v - want).abs() <= 1e-4 * span(reset),
            "row {k}: {v} vs {want}"
        );
    }
}

/// RA, RB and C, with CONT driven to `cont` volts and every pin's level traced.
fn driven_astable(cont: f64) -> Results {
    simulate(&transient(
        10_000_000_000,
        100_000_000,
        r#""gnd", "vcc", "dis", "thr", "cont", "out""#,
        &[
            &source(5.0),
            &source_on("vc", "cont", cont),
            &resistor("ra", "vcc", "dis", 1e3),
            &resistor("rb", "dis", "thr", 10e3),
            &held_capacitor("c1", "thr", "gnd", 1e-7, 0.0),
            &ne555(
                "u1",
                ["gnd", "thr", "out", "vcc", "cont", "thr", "dis", "vcc"],
            ),
        ],
    ))
}

/// High charges C toward VCC through RA and RB, with the 250 Mohm discharge across RA's end.
/// Low discharges it toward the discharge's divider through RB and RA in parallel with it.
fn astable_period(ra: f64, rb: f64, c: f64, low: f64, high: f64) -> f64 {
    let (vcc, on, off) = (5.0_f64, 18.75, 2.5e8);
    let (charge_to, charge_r) = (vcc * off / (ra + off), rb + ra * off / (ra + off));
    let (drain_to, drain_r) = (vcc * on / (ra + on), rb + ra * on / (ra + on));
    let high_time = charge_r * c * ((charge_to - low) / (charge_to - high)).ln();
    let low_time = drain_r * c * ((high - drain_to) / (low - drain_to)).ln();
    high_time + low_time
}

#[test]
fn a_driven_cont_moves_both_thresholds() {
    let results = driven_astable(4.0);
    let falls: Vec<u64> = edges(&results, "#u1.logic.out")
        .into_iter()
        .filter(|&(_, level)| level == Level::Low)
        .map(|(at, _)| at)
        .collect();
    assert!(falls.len() >= 4, "{falls:?}");
    let period = (falls[falls.len() - 1] - falls[0]) as f64 / (falls.len() - 1) as f64 * 1e-12;
    let closed = astable_period(1e3, 10e3, 1e-7, 2.0, 4.0);
    assert!(
        (period / closed - 1.0).abs() <= 1e-4,
        "{period} vs {closed}"
    );
    // Undriven, the thresholds are 1/3 and 2/3 of VCC and the period is shorter.
    assert!(closed > 1.2 * astable_period(1e3, 10e3, 1e-7, 5.0 / 3.0, 10.0 / 3.0));
    let trace = trace(&results);
    let thr = column(trace, "#net.thr");
    for &at in &falls {
        let past = thr[row(&results, at)] - 4.0;
        assert!(past > 0.0 && past <= 1e-8, "{past} V past CONT at {at}");
    }
}

#[test]
fn a_flip_that_changes_no_drive_cuts_the_step_without_a_restart() {
    let results = driven_astable(10.0 / 3.0);
    let trace = trace(&results);
    let times: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    // TRIG rises past 1/3 VCC during the first charge, while the latch stays set.
    let [(trig, Level::High), ..] = edges(&results, "#u1.logic.trig")[..] else {
        panic!();
    };
    let k = row(&results, trig);
    assert!(
        times[k + 1] - trig > 1_000_000,
        "the step after {trig} is {} ps",
        times[k + 1] - trig
    );
    // An output edge restarts on a 1 ns step, which the comparator crossing back 1 ps later cuts.
    let (fall, Level::Low) = edges(&results, "#u1.logic.out")[0] else {
        panic!();
    };
    let k = row(&results, fall);
    assert_eq!(times[k + 1..k + 3], [fall + 1, fall + 1_001]);
    assert_eq!(
        edges(&results, "#u1.logic.thres")[..2],
        [(fall, Level::High), (fall + 1, Level::Low)]
    );
}

#[test]
fn flips_at_one_picosecond_on_two_chips_react_together_on_one_row() {
    // Both chips read one THRES against one CONT, so their resets flip at the same picosecond.
    let results = simulate(&transient(
        10_000_000_000,
        100_000_000,
        r#""gnd", "vcc", "dis", "thr", "cont", "out1", "out2""#,
        &[
            &source(5.0),
            &resistor("ra", "vcc", "dis", 1e3),
            &resistor("rb", "dis", "thr", 10e3),
            &held_capacitor("c1", "thr", "gnd", 1e-7, 0.0),
            &ne555(
                "u1",
                ["gnd", "thr", "out1", "vcc", "cont", "thr", "dis", "vcc"],
            ),
            &ne555(
                "u2",
                ["gnd", "thr", "out2", "vcc", "cont", "thr", "dis", "vcc"],
            ),
        ],
    ));
    let falls = |chip: &str| -> Vec<u64> {
        edges(&results, &format!("#{chip}.logic.out"))
            .into_iter()
            .filter(|&(_, level)| level == Level::Low)
            .map(|(at, _)| at)
            .collect()
    };
    let (one, two) = (falls("u1"), falls("u2"));
    assert!(one.len() >= 4, "{one:?}");
    assert_eq!(one, two);
    let trace = trace(&results);
    for &at in &one {
        let k = row(&results, at);
        assert!(column(trace, "#net.out1")[k] > 4.0 && column(trace, "#net.out2")[k] > 4.0);
        assert!(column(trace, "#net.out1")[k + 2] < 0.1 && column(trace, "#net.out2")[k + 2] < 0.1);
    }
}

#[test]
fn an_edge_at_stop_is_recorded_on_the_last_row() {
    let full = power_on_reset(40_000_000_000);
    let [(at, Level::High)] = edges(&full, "#u1.logic.out")[..] else {
        panic!();
    };
    let ends = power_on_reset(at);
    assert_eq!(edges(&ends, "#u1.logic.out"), [(at, Level::High)]);
    let last = trace(&ends);
    assert_eq!(last.end().unwrap().as_ps(), at);
    // No restart ran, so the last row still holds the old drive.
    assert!(*column(last, "#net.out").last().unwrap() < 0.1);
    let full_reset = column(trace(&full), "#net.rst")[row(&full, at)];
    assert_eq!(*column(last, "#net.rst").last().unwrap(), full_reset);

    // An edge in the final step, before stop, restarts and the run still ends at stop.
    let after = power_on_reset(at + 500);
    assert_eq!(edges(&after, "#u1.logic.out"), [(at, Level::High)]);
    let times: Vec<u64> = trace(&after).time().iter().map(|t| t.as_ps()).collect();
    assert_eq!(times[times.len() - 2..], [at, at + 500]);
    assert!(*column(trace(&after), "#net.out").last().unwrap() > 4.0);
}

#[test]
fn a_crossing_the_restart_pulls_back_is_recorded_at_the_edge() {
    // A monostable: TRIG starts low and sets the latch, then THRES on DIS resets it. The
    // discharge's 0.18 A through the hold of 1 fs over 1 nF pulls DIS 1.8e-7 V back, far more
    // than the located row sits past the threshold, so the restart's solve is under it again.
    let results = simulate(&transient(
        2_000_000_000,
        100_000_000,
        r#""gnd", "vcc", "trig", "dis", "cont", "out""#,
        &[
            &source(5.0),
            &resistor("rt", "vcc", "trig", 10e3),
            &held_capacitor("ct", "trig", "gnd", 1e-9, 0.0),
            &resistor("ra", "vcc", "dis", 1e6),
            &held_capacitor("c1", "dis", "gnd", 1e-9, 0.0),
            &ne555(
                "u1",
                ["gnd", "trig", "out", "vcc", "cont", "dis", "dis", "vcc"],
            ),
        ],
    ));
    let [(fall, Level::Low)] = edges(&results, "#u1.logic.out")[..] else {
        panic!("{:?}", edges(&results, "#u1.logic.out"));
    };
    let trace = trace(&results);
    let k = row(&results, fall);
    let past = column(trace, "#net.dis")[k] - column(trace, "#net.cont")[k];
    assert!(past > 0.0 && past <= 1e-8, "{past} V past CONT");
    // The level the latch reset on, then its return on the next row.
    let next = trace.time()[k + 1].as_ps();
    assert_eq!(
        edges(&results, "#u1.logic.thres"),
        [(fall, Level::High), (next, Level::Low)]
    );
}

#[test]
fn a_level_the_restart_flips_is_recorded_at_the_edge() {
    // U1's OUT drives U2's TRIG, which only the restart after each OUT fall sees go low.
    let results = simulate(&transient(
        10_000_000_000,
        100_000_000,
        r#""gnd", "vcc", "dis", "thr", "cont1", "out1", "cont2", "out2", "dis2""#,
        &[
            &source(5.0),
            &resistor("ra", "vcc", "dis", 1e3),
            &resistor("rb", "dis", "thr", 10e3),
            &held_capacitor("c1", "thr", "gnd", 1e-7, 0.0),
            &ne555(
                "u1",
                ["gnd", "thr", "out1", "vcc", "cont1", "thr", "dis", "vcc"],
            ),
            &ne555(
                "u2",
                ["gnd", "out1", "out2", "vcc", "cont2", "gnd", "dis2", "vcc"],
            ),
        ],
    ));
    let falls = |signal: &str| -> Vec<u64> {
        edges(&results, signal)
            .into_iter()
            .filter(|&(_, level)| level == Level::Low)
            .map(|(at, _)| at)
            .collect()
    };
    let out = falls("#u1.logic.out");
    assert!(out.len() >= 4, "{out:?}");
    assert_eq!(falls("#u2.logic.trig"), out);
}

#[test]
fn two_chips_that_flip_each_other_at_one_time_are_unsettled() {
    // Each OUT drives the other's TRIG. When both reset at once, both OUTs fall, which sets
    // both latches, while both timing capacitors are still over THRES.
    let half = |n: &str| {
        [
            resistor(&format!("ra{n}"), "vcc", &format!("d{n}"), 10e3),
            resistor(&format!("rb{n}"), &format!("d{n}"), &format!("t{n}"), 10e3),
            held_capacitor(&format!("c{n}"), &format!("t{n}"), "gnd", 1e-8, 0.0),
        ]
    };
    let [ra1, rb1, c1] = half("1");
    let [ra2, rb2, c2] = half("2");
    let results = simulate(&transient(
        500_000_000,
        10_000_000,
        r#""gnd", "vcc", "d1", "t1", "o1", "k1", "d2", "t2", "o2", "k2""#,
        &[
            &source(5.0),
            &ra1,
            &rb1,
            &c1,
            &ne555("u1", ["gnd", "o2", "o1", "vcc", "k1", "t1", "d1", "vcc"]),
            &ra2,
            &rb2,
            &c2,
            &ne555("u2", ["gnd", "o1", "o2", "vcc", "k2", "t2", "d2", "vcc"]),
        ],
    ));
    let [found] = results.diagnostics() else {
        panic!("{:?}", results.diagnostics());
    };
    assert_eq!(found.code(), "chip.unsettled");
    assert_eq!(
        found.refs(),
        ["#u1".parse().unwrap(), "#u2".parse().unwrap()]
    );
    assert!(found.at().unwrap().as_ps() > 0);
}
