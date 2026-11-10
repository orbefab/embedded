//! Each run keeps every accepted step, so a landing and a restart's 1 ns seed are both rows.

use orbe_model::Drive;

use crate::netlist::{NetId, Netlist};

use crate::name::Name;

use super::super::*;
use super::fixtures::*;
use crate::active;
use crate::active::Actives;
use crate::active::script::{self, Op, Sample, eager};
use crate::newton::{Circuit, Conditions, Iterate, newton};
use crate::op;

pub(super) const US: u64 = 1_000_000;
pub(super) const NS: u64 = 1_000;

/// A 5 V `vcc` and the parts in `parts`, over `nets` besides `gnd` and `vcc`. 1 us steps at
/// most and a 1 us grid.
pub(super) fn circuit(stop_ps: u64, nets: &[&str], parts: &[String]) -> Netlist {
    stepped(stop_ps, US, nets, parts)
}

/// `circuit` with steps of at most `max_step_ps` and a grid of the same.
fn stepped(stop_ps: u64, max_step_ps: u64, nets: &[&str], parts: &[String]) -> Netlist {
    let nets: Vec<String> = ["gnd", "vcc"]
        .iter()
        .chain(nets)
        .map(|net| format!("{net:?}"))
        .collect();
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": {max_step_ps}, "grid_ps": {max_step_ps} }} }},
          "ground": "gnd",
          "nets": [{}],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{ "value": 5.0, "unit": "V", "source": "user" }} }},
            {}
          ]
        }}"#,
        nets.join(", "),
        parts.join(",\n"),
    );
    crate::netlist::parse(json.as_bytes()).unwrap()
}

pub(super) fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "resistor", "pins": {{ "a": "{a}", "b": "{b}" }},
              "resistance": {{ "value": {ohm:?}, "unit": "ohm", "source": "user" }},
              "ratings": {{ "power": null }} }}"#
    )
}

pub(super) fn capacitor(name: &str, a: &str, farad: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "capacitor", "pins": {{ "a": "{a}", "b": "gnd" }},
              "capacitance": {{ "value": {farad:?}, "unit": "F", "source": "user" }},
              "initial": null }}"#
    )
}

fn pin_into_rc(stop_ps: u64) -> Netlist {
    circuit(
        stop_ps,
        &["out", "cap"],
        &[
            resistor("r1", "out", "cap", 1000.0),
            capacitor("c1", "cap", 1e-9),
        ],
    )
}

const TAU_US: f64 = (25.0 + 1000.0) * 1e-9 * 1e6;

pub(super) fn net(netlist: &Netlist, name: &str) -> NetId {
    netlist
        .nets()
        .find(|(_, net)| net.as_str() == name)
        .map(|(id, _)| id)
        .unwrap()
}

pub(super) fn scripted(
    netlist: &Netlist,
    out: &str,
    input: &str,
    script: &[(u64, Op)],
    echo: Option<u64>,
) -> active::Chip {
    let pins = (
        net(netlist, "vcc"),
        net(netlist, "gnd"),
        net(netlist, out),
        net(netlist, input),
    );
    let script = script
        .iter()
        .map(|&(at, op)| (Time::from_ps(at), op))
        .collect();
    script::chip(pins, script, echo)
}

pub(super) fn run_active(netlist: &Netlist, chips: Vec<active::Chip>) -> (Trace, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let outcome = run_with(
        netlist,
        &transient(netlist),
        chips,
        BUDGET,
        Rows::Steps,
        &mut diagnostics,
    );
    (outcome.trace.unwrap(), diagnostics)
}

pub(super) fn volts(trace: &Trace, net: &str) -> Vec<(u64, f64)> {
    let column = trace
        .columns()
        .iter()
        .position(|column| *column == Ref::Net(Name::new(net).unwrap()))
        .unwrap();
    trace
        .time()
        .iter()
        .map(|at| at.as_ps())
        .zip(trace.column(column).iter().copied())
        .collect()
}

pub(super) fn at(rows: &[(u64, f64)], ps: u64) -> f64 {
    rows.iter()
        .find(|&&(at, _)| at == ps)
        .unwrap_or_else(|| panic!("no step at {ps} ps"))
        .1
}

/// Whether a restart at `ps` seeded the next segment: the next step is exactly 1 ns.
pub(super) fn restarted(rows: &[(u64, f64)], ps: u64) -> bool {
    let k = rows
        .iter()
        .position(|&(at, _)| at == ps)
        .unwrap_or_else(|| panic!("no step at {ps} ps"));
    rows.get(k + 1).is_some_and(|&(next, _)| next == ps + NS)
}

pub(super) fn after(rows: &[(u64, f64)], ps: u64) -> f64 {
    let k = rows
        .iter()
        .position(|&(at, _)| at == ps)
        .unwrap_or_else(|| panic!("no step at {ps} ps"));
    rows[k + 1].1
}

const HIGH_AT: u64 = 2 * US + 333;

/// Every accepted step's time and `net`, with the pin turned high at `HIGH_AT`, and with a write
/// there that leaves it off. The chip reads `idle`.
fn high_and_still(netlist: &Netlist, net: &str) -> [Vec<(u64, f64)>; 2] {
    [Drive::High, Drive::Off].map(|drive| {
        let chip = scripted(netlist, "out", "idle", &[(HIGH_AT, Op::Drive(drive))], None);
        let (trace, diagnostics) = run_active(netlist, vec![chip]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        volts(&trace, net)
    })
}

/// `c1` sits on `idle`, which the pin does not reach, at rest at 0 V through `ri` to ground or
/// at 5 V through `ri` to `vcc`. At 5 V the step and the restart solve give it currents that
/// differ in their last bits, far under what moves it by 1 nV over a step. Either way the
/// segment goes on, with its step, as across a write that leaves the pin off. The step after
/// `HIGH_AT` already has the drive in force.
#[test]
fn a_drive_that_moves_no_capacitor_keeps_the_segment() {
    for rest in ["gnd", "vcc"] {
        let netlist = circuit(
            4 * US,
            &["out", "idle"],
            &[
                resistor("rl", "out", "gnd", 1000.0),
                resistor("ri", "idle", rest, 1000.0),
                capacitor("c1", "idle", 1e-9),
            ],
        );
        let [high, still] = high_and_still(&netlist, "out");
        let times = |rows: &[(u64, f64)]| rows.iter().map(|&(at, _)| at).collect::<Vec<_>>();
        assert_eq!(times(&high), times(&still), "c1 on {rest}");
        assert!(!restarted(&high, HIGH_AT));
        assert!(at(&high, HIGH_AT).abs() < 1e-6, "{}", at(&high, HIGH_AT));
        let driven = 5.0 * 1000.0 / 1025.0;
        assert!(
            (after(&high, HIGH_AT) - driven).abs() < 1e-6,
            "{}",
            after(&high, HIGH_AT)
        );
    }
}

/// Every accepted step's time and `net`, with a write at `cut` that leaves the pin off, and
/// the pin turned high at `edge` or left off there by a second write.
fn cut_then_high(netlist: &Netlist, net: &str, cut: u64, edge: u64) -> [Vec<(u64, f64)>; 2] {
    [Drive::High, Drive::Off].map(|drive| {
        let script = [(cut, Op::Drive(Drive::Off)), (edge, Op::Drive(drive))];
        let chip = scripted(netlist, "out", "idle", &script, None);
        let (trace, diagnostics) = run_active(netlist, vec![chip]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        volts(&trace, net)
    })
}

/// `c1` sits at rest, and `rc` couples the pin into it, so the drive changes its current by
/// `di`, with `max_step * 2|di| / 2C` under 1 nV. A write `landed` ps before the edge cuts the
/// step, and the segment still wants `max_step`. The point before the edge moves by `di`, so
/// the next estimate reads no kink of `di` over the landed step. From rest the edge's mode is
/// all that moves `c1`'s current, by at most `2|di|` a step, so the step after the edge is
/// `max_step`, and every step is the one it is with the pin left off.
#[test]
fn a_kept_edge_into_a_capacitor_at_rest_takes_the_wanted_step_after_a_short_landing() {
    // `c1` at 5 V, 0.49 pA over 1 nF, 0.49 nV over 1 us, 300 ps landed.
    let near = (4 * US, US, "vcc", 1e-9, 1e13, 300);
    // `c1` at 0 V, 9.8 fA over 100 nF, 0.98 nV over 10 ms, 1 us landed.
    let far = (60_000 * US, 10_000 * US, "gnd", 1e-7, 5e14, US);
    for (stop, max_step, rest, farad, coupling, landed) in [near, far] {
        let edge = stop / 2 + 333;
        let netlist = stepped(
            stop,
            max_step,
            &["out", "idle"],
            &[
                resistor("rl", "out", "gnd", 1000.0),
                resistor("ri", "idle", rest, 1000.0),
                resistor("rc", "out", "idle", coupling),
                capacitor("c1", "idle", farad),
            ],
        );
        let [high, still] = cut_then_high(&netlist, "idle", edge - landed, edge);
        let times = |rows: &[(u64, f64)]| rows.iter().map(|&(at, _)| at).collect::<Vec<_>>();
        let next = high.iter().position(|&(at, _)| at == edge).unwrap() + 1;
        assert_eq!(high[next].0, edge + max_step, "{rest}");
        assert_eq!(times(&high), times(&still), "{rest}");
    }
}

/// `c1` and `ri` make a 1 ns mode, far faster than the 1 us step, on which the trapezoidal rule
/// rings, so each kept edge adds its mode to the ones before it. The pin flips every 1 us, and
/// `rc` couples each flip into `c1` by 2.3 pA, above the 1 pA Newton settles a quiet
/// current to. The first `n` edges keep the segment, `n` being how many of
/// `max_step * |di| / C` fit in 1 nV, and that quotient is not an integer. The next one starts
/// a segment, which clears the sum, and so on. Without `rc` no edge moves `c1`, and every
/// edge keeps the segment.
#[test]
fn kept_edges_into_a_fast_mode_start_a_segment_once_their_sum_passes_the_floor() {
    let (edges, farad) = (15, 10e-9);
    let di = 2.3 * crate::newton::ABSTOL;
    let coupling = 5.0 * 1000.0 / 1025.0 / di;
    let flips: Vec<u64> = (0..edges).map(|k| HIGH_AT + k * US).collect();
    let script: Vec<(u64, Op)> = flips
        .iter()
        .zip([Drive::High, Drive::Off].iter().cycle())
        .map(|(&at, &drive)| (at, Op::Drive(drive)))
        .collect();
    let starts = |rc: &[String]| {
        let parts = [
            resistor("rl", "out", "gnd", 1000.0),
            resistor("ri", "idle", "gnd", 1e-9 / farad),
            capacitor("c1", "idle", farad),
        ];
        let netlist = circuit((edges + 3) * US, &["out", "idle"], &[&parts, rc].concat());
        let chip = scripted(&netlist, "out", "idle", &script, None);
        let (trace, diagnostics) = run_active(&netlist, vec![chip]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let rows = volts(&trace, "idle");
        (0..flips.len())
            .filter(|&k| restarted(&rows, flips[k]))
            .collect::<Vec<_>>()
    };
    let di = 5.0 * 1000.0 / 1025.0 / coupling;
    let n = (crate::newton::VNTOL / (1e-6 * di / farad)) as usize;
    let every: Vec<usize> = (n..flips.len()).step_by(n + 1).collect();
    assert_eq!(every.len(), 3);
    assert_eq!(starts(&[resistor("rc", "out", "idle", coupling)]), every);
    assert_eq!(starts(&[]), Vec::<usize>::new());
}

/// The same drive into `c1` through 1 kohm changes its current by about 5 mA, which moves it
/// by volts over a 1 us step. A segment starts at `HIGH_AT` and takes 1 ns.
#[test]
fn a_drive_that_feeds_a_capacitor_starts_a_segment() {
    let netlist = circuit(
        4 * US,
        &["out", "idle"],
        &[
            resistor("r1", "out", "idle", 1000.0),
            capacitor("c1", "idle", 1e-9),
        ],
    );
    let [high, still] = high_and_still(&netlist, "idle");
    assert!(restarted(&high, HIGH_AT));
    assert!(!restarted(&still, HIGH_AT));
}

/// The pin is off until `ON`, high to `OFF`, low to 8 us, and high at `stop_ps`. None of these
/// is a row time. Every step to `ON` solves with the pin off, so the capacitor holds 0 V. Each
/// change lands on its picosecond with the old drive, and restarts. The change at `stop_ps`
/// lands, with no restart.
#[test]
fn a_scripted_drive_lands_on_its_picosecond_and_restarts() {
    const ON: u64 = US + 333;
    const OFF: u64 = 4 * US + 777;
    let netlist = pin_into_rc(8 * US);
    let chip = scripted(
        &netlist,
        "out",
        "cap",
        &[
            (ON, Op::Drive(Drive::High)),
            (OFF, Op::Drive(Drive::Low)),
            (8 * US, Op::Drive(Drive::High)),
        ],
        None,
    );
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let cap = volts(&trace, "cap");
    assert!(
        cap.iter()
            .filter(|&&(t, _)| t <= ON)
            .all(|&(_, v)| v == 0.0)
    );
    assert!(restarted(&cap, ON) && restarted(&cap, OFF));
    let us = |ps: u64| ps as f64 / US as f64;
    let high = 5.0 * (1.0 - libm::exp(-us(OFF - ON) / TAU_US));
    let low = high * libm::exp(-us(8 * US - OFF) / TAU_US);
    assert!(
        (at(&cap, OFF) - high).abs() < 1e-4 * 5.0,
        "{}",
        at(&cap, OFF)
    );
    assert!(
        (at(&cap, 8 * US) - low).abs() < 1e-4 * 5.0,
        "{}",
        at(&cap, 8 * US)
    );
    assert_eq!(cap.last().unwrap().0, 8 * US);
}

/// A second write of the same drive is a stop, so a step lands there. The image is the same, so
/// it does not restart, and the capacitor's curve goes on as if the write were not there.
#[test]
fn a_write_that_leaves_the_image_lands_without_a_restart() {
    let netlist = pin_into_rc(4 * US);
    let run = |script: &[(u64, Op)]| {
        let (trace, diagnostics) = run_active(
            &netlist,
            vec![scripted(&netlist, "out", "cap", script, None)],
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        volts(&trace, "cap")
    };
    let once = run(&[(US, Op::Drive(Drive::High))]);
    let twice = run(&[
        (US, Op::Drive(Drive::High)),
        (2 * US + 7, Op::Drive(Drive::High)),
    ]);
    assert!(once.iter().all(|&(t, _)| t != 2 * US + 7));
    assert!(!restarted(&twice, 2 * US + 7));
    assert!((at(&once, 4 * US) - at(&twice, 4 * US)).abs() < 1e-5);
}

/// At 2 us the chip samples the capacitor, which is at `v = 5 * (1 - e^(-1 / 1.025))` V. 500 ns
/// later it drives high if the sample was above its threshold. A threshold 5 mV under `v` keeps
/// the pin high, and 5 mV over it drives the pin low and restarts. The sample is read at its own
/// picosecond, to well inside 5 mV.
#[test]
fn a_sample_at_the_chips_time_sets_a_later_drive() {
    let netlist = pin_into_rc(4 * US);
    let v = 5.0 * (1.0 - libm::exp(-1.0 / TAU_US));
    let run = |threshold: f64| {
        let script = [
            (US, Op::Drive(Drive::High)),
            (
                2 * US,
                Op::Sample(Sample {
                    after: 500 * NS,
                    threshold,
                }),
            ),
        ];
        let (trace, diagnostics) = run_active(
            &netlist,
            vec![scripted(&netlist, "out", "cap", &script, None)],
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        volts(&trace, "cap")
    };
    let under = run(v - 5e-3);
    let over = run(v + 5e-3);
    assert!(!restarted(&under, 2 * US) && !restarted(&over, 2 * US));
    assert!(!restarted(&under, 2 * US + 500 * NS));
    assert!(restarted(&over, 2 * US + 500 * NS));
    assert!(at(&over, 4 * US) < at(&over, 2 * US + 500 * NS));
    assert!(at(&under, 4 * US) > at(&under, 2 * US + 500 * NS));
}

/// 0 to 5 V at 1 us through 1 kohm into 1 nF on `in`, which crosses 2.5 V near
/// `1 + ln 2` us. The chip drives low from 100 ns and echoes `in` onto `out` 200 ns after it
/// takes `in`: at power-on and after each flip. Its copy runs past the crossing before the crossing is located, so the committed core
/// runs again to it. `out` turns high exactly 200 ns after the located crossing.
fn echo() -> (Netlist, active::Chip) {
    let step = r#"{ "name": "vs", "kind": "voltage_source", "pins": { "p": "src", "n": "gnd" },
          "voltage": { "kind": "step", "initial": { "value": 0.0, "unit": "V", "source": "user" },
                       "final": { "value": 5.0, "unit": "V", "source": "user" }, "at_ps": 1000000 } }"#;
    let netlist = circuit(
        3 * US,
        &["out", "src", "in"],
        &[
            resistor("rl", "out", "gnd", 1000.0),
            step.to_string(),
            resistor("rs", "src", "in", 1000.0),
            capacitor("cs", "in", 1e-9),
        ],
    );
    let chip = scripted(
        &netlist,
        "out",
        "in",
        &[(100 * NS, Op::Drive(Drive::Low))],
        Some(200 * NS),
    );
    (netlist, chip)
}

#[test]
fn an_input_flip_reaches_the_core_at_its_located_picosecond() {
    let (netlist, chip) = echo();
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let input = volts(&trace, "in");
    let crossing = input.iter().find(|&&(_, v)| v > 2.5).unwrap().0;
    // `docs/engine/engine.md` bounds the shift by `1e-4 * tau * swing / left`, 200 ps here.
    let expected = US as f64 * (1.0 + std::f64::consts::LN_2);
    assert!((crossing as f64 - expected).abs() < 200.0, "{crossing}");
    let out = volts(&trace, "out");
    let rise = crossing + 200 * NS;
    assert!(
        out.iter().any(|&(t, _)| t == rise),
        "a step lands on the drive edge"
    );
    assert!(
        out.iter()
            .filter(|&&(t, _)| t <= rise)
            .all(|&(_, v)| v.abs() < 1e-9)
    );
    let high = 5.0 * 1000.0 / 1025.0;
    assert!((out.last().unwrap().1 - high).abs() < 1e-9);
}

/// The echo drives `out`, a net with no capacitor, 200 ns after `in` crosses 2.5 V. The only
/// store is the 1 nF on `in`. That edge moves its current by less than Newton settles, so the
/// segment and its swings stay. The echo of the level at power-on also lands 200 ns later.
/// The accepted steps of `in` match a run that lands at both of those picoseconds and leaves
/// the drive low. The row at the drive edge still holds the old drive.
#[test]
fn a_drive_edge_that_moves_no_store_above_newtons_tolerance_keeps_the_segment() {
    let (netlist, chip) = echo();
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let crossing = volts(&trace, "in")
        .iter()
        .find(|&&(_, v)| v > 2.5)
        .unwrap()
        .0;
    let out = volts(&trace, "out");
    let rise = crossing + 200 * NS;
    assert!(at(&out, rise).abs() < 1e-9, "{}", at(&out, rise));
    let still = scripted(
        &netlist,
        "out",
        "in",
        &[
            (100 * NS, Op::Drive(Drive::Low)),
            (200 * NS, Op::Drive(Drive::Low)),
            (rise, Op::Drive(Drive::Low)),
        ],
        None,
    );
    let (twin, diagnostics) = run_active(&netlist, vec![still]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let times = |rows: &[(u64, f64)]| rows.iter().map(|&(at, _)| at).collect::<Vec<_>>();
    assert_eq!(times(&volts(&trace, "in")), times(&volts(&twin, "in")));
    let high = 5.0 * 1000.0 / 1025.0;
    assert!((out.last().unwrap().1 - high).abs() < 1e-9);
}

#[test]
fn an_active_run_is_deterministic() {
    let run = || {
        let (netlist, chip) = echo();
        run_active(&netlist, vec![chip]).0
    };
    let (first, second) = (run(), run());
    assert_eq!(first.time(), second.time());
    for column in 0..first.columns().len() {
        let bits = |trace: &Trace| -> Vec<u64> {
            trace.column(column).iter().map(|v| v.to_bits()).collect()
        };
        assert_eq!(bits(&first), bits(&second));
    }
}

/// The chip samples `in` under 4 V at the crossing's own picosecond, which a run without the
/// sample finds. The sample and the flip's echo both drive `out` 200 ns later, and the later
/// write wins. The sample is read before the flip, so `out` turns high.
#[test]
fn a_sample_reads_before_a_flip_at_its_own_picosecond() {
    let (netlist, chip) = echo();
    let crossing = |trace: &Trace| {
        volts(trace, "in")
            .iter()
            .find(|&&(_, v)| v > 2.5)
            .unwrap()
            .0
    };
    let at = crossing(&run_active(&netlist, vec![chip]).0);
    let sample = Sample {
        after: 200 * NS,
        threshold: 4.0,
    };
    let chip = scripted(
        &netlist,
        "out",
        "in",
        &[(100 * NS, Op::Drive(Drive::Low)), (at, Op::Sample(sample))],
        Some(200 * NS),
    );
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(crossing(&trace), at);
    let out = volts(&trace, "out");
    assert!((out.last().unwrap().1 - 5.0 * 1000.0 / 1025.0).abs() < 1e-9);
}

/// Chip `a` drives `a_out` high at 1 us into 10 kohm, and chip `b` reads `a_out`.
fn chain() -> Netlist {
    circuit(
        2 * US,
        &["a_out", "b_out", "a_in"],
        &[
            resistor("ra", "a_out", "gnd", 10e3),
            resistor("rb", "b_out", "gnd", 1000.0),
            resistor("rin", "a_in", "gnd", 1000.0),
        ],
    )
}

/// `b` echoes `a_out` onto `b_out` 100 ns later. `b`'s input flips on `a`'s restart solve, not
/// on a step, and still reaches `b` at 1 us.
#[test]
fn one_active_chip_reads_another_through_the_restart() {
    let netlist = chain();
    let a = scripted(
        &netlist,
        "a_out",
        "a_in",
        &[(US, Op::Drive(Drive::High))],
        None,
    );
    let b = scripted(
        &netlist,
        "b_out",
        "a_out",
        &[(100 * NS, Op::Drive(Drive::Low))],
        Some(100 * NS),
    );
    let (trace, diagnostics) = run_active(&netlist, vec![a, b]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let b_out = volts(&trace, "b_out");
    assert!(at(&b_out, US).abs() < 1e-9);
    assert!(at(&b_out, US + 100 * NS).abs() < 1e-9);
    let high = 5.0 * 1000.0 / 1025.0;
    assert!((after(&b_out, US + 100 * NS) - high).abs() < 1e-9);
    assert!((b_out.last().unwrap().1 - high).abs() < 1e-9);
}

/// `a_out` with `a` echoing `b_out` onto it 100 ns after it takes it. `a` and `b` drive low
/// from power-on and run `a_ops` and `b_ops` after that. A source steps at 1 us, so a step ends
/// there.
fn echoed(a_ops: &[(u64, Op)], b_ops: &[(u64, Op)]) -> Vec<(u64, f64)> {
    let step = r#"{ "name": "vs", "kind": "voltage_source", "pins": { "p": "src", "n": "gnd" },
          "voltage": { "kind": "step", "initial": { "value": 0.0, "unit": "V", "source": "user" },
                       "final": { "value": 5.0, "unit": "V", "source": "user" }, "at_ps": 1000000 } }"#;
    let netlist = circuit(
        2 * US,
        &["a_out", "b_out", "b_in", "src"],
        &[
            resistor("ra", "a_out", "gnd", 1000.0),
            resistor("rb", "b_out", "gnd", 10e3),
            resistor("rin", "b_in", "gnd", 1000.0),
            step.to_string(),
            resistor("rs", "src", "gnd", 1000.0),
        ],
    );
    let ops = |more: &[(u64, Op)]| [&[(0, Op::Drive(Drive::Low))], more].concat();
    let a = scripted(&netlist, "a_out", "b_out", &ops(a_ops), Some(100 * NS));
    let b = scripted(&netlist, "b_out", "b_in", &ops(b_ops), None);
    let (trace, diagnostics) = run_active(&netlist, vec![a, b]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    volts(&trace, "a_out")
}

const A_HIGH: f64 = 5.0 * 1000.0 / 1025.0;

/// `a` lands at 1 us on a write that keeps its image, where `b` turns `b_out` high. `a` takes
/// the flip on `b`'s restart solve, so its echo comes out at 1.1 us.
#[test]
fn a_flip_at_a_cores_own_landing_echoes_on_time() {
    let a_out = echoed(
        &[(US, Op::Drive(Drive::Low))],
        &[(US, Op::Drive(Drive::High))],
    );
    assert!(at(&a_out, US + 100 * NS).abs() < 1e-9);
    assert!((after(&a_out, US + 100 * NS) - A_HIGH).abs() < 1e-9);
    assert!((a_out.last().unwrap().1 - A_HIGH).abs() < 1e-9);
}

/// As above, and `b` turns low again at 1.05 us. `a` takes that flip from the core committed at
/// 1 us, whose copy already ran to the first echo, so `a_out` is high from 1.1 to 1.15 us.
#[test]
fn a_flip_at_a_cores_own_landing_echoes_after_a_rewind() {
    let a_out = echoed(
        &[(US, Op::Drive(Drive::Low))],
        &[
            (US, Op::Drive(Drive::High)),
            (US + 50 * NS, Op::Drive(Drive::Low)),
        ],
    );
    assert!((after(&a_out, US + 100 * NS) - A_HIGH).abs() < 1e-9);
    assert!(at(&a_out, US + 150 * NS) > A_HIGH - 1e-9);
    assert!(after(&a_out, US + 150 * NS).abs() < 1e-9);
    assert!(a_out.last().unwrap().1.abs() < 1e-9);
}

/// `a` lands at 0.5 us, and its copy runs on to 1 us without a stop. `b` turns `b_out` high
/// there, so `a` takes the flip at a landing it did not stop at.
#[test]
fn a_flip_at_a_landing_a_core_ran_through_echoes_on_time() {
    let a_out = echoed(
        &[(US / 2, Op::Drive(Drive::Low))],
        &[(US, Op::Drive(Drive::High))],
    );
    assert!(at(&a_out, US + 100 * NS).abs() < 1e-9);
    assert!((after(&a_out, US + 100 * NS) - A_HIGH).abs() < 1e-9);
    assert!((a_out.last().unwrap().1 - A_HIGH).abs() < 1e-9);
}

/// A sample that acts 0 ps later puts its drive at the stop the core already reached.
#[test]
#[should_panic(expected = "a core stops only after where it reached")]
fn a_core_that_stops_where_it_reached_is_caught() {
    let netlist = pin_into_rc(2 * US);
    let sample = Sample {
        after: 0,
        threshold: 1.0,
    };
    let chip = scripted(&netlist, "out", "cap", &[(US, Op::Sample(sample))], None);
    run_active(&netlist, vec![chip]);
}

#[test]
#[should_panic(
    expected = "a sample, an input, a record or a touch at a landing never changes an image"
)]
fn a_sample_that_drives_at_its_read_is_caught() {
    let netlist = pin_into_rc(2 * US);
    let sample = Sample {
        after: NS,
        threshold: 1.0,
    };
    let chip = scripted(&netlist, "out", "cap", &[(US, Op::Sample(sample))], None);
    run_active(&netlist, vec![eager(chip)]);
}

/// `b` drives low from power-on, as its first echo does, so only the flip on `a`'s restart
/// solve drives it.
#[test]
#[should_panic(expected = "an input on the held solve never changes an image")]
fn an_input_that_drives_at_its_read_on_the_held_solve_is_caught() {
    let netlist = chain();
    let a = scripted(
        &netlist,
        "a_out",
        "a_in",
        &[(US, Op::Drive(Drive::High))],
        None,
    );
    let b = scripted(
        &netlist,
        "b_out",
        "a_out",
        &[(0, Op::Drive(Drive::Low))],
        Some(100 * NS),
    );
    run_active(&netlist, vec![a, eager(b)]);
}

/// The chip is off at power-on, and its first echo drives low at once.
#[test]
#[should_panic(expected = "the power-on input never changes an image")]
fn a_power_on_input_that_drives_is_caught() {
    let netlist = pin_into_rc(2 * US);
    let chip = scripted(&netlist, "out", "cap", &[], Some(NS));
    run_active(&netlist, vec![eager(chip)]);
}

#[test]
fn an_open_pin_leaves_its_net_to_the_pull_up() {
    const OFF: u64 = US + 333;
    let netlist = circuit(2 * US, &["out"], &[resistor("rp", "vcc", "out", 1000.0)]);
    let chip = scripted(
        &netlist,
        "out",
        "out",
        &[(0, Op::Drive(Drive::Low)), (OFF, Op::Drive(Drive::Off))],
        None,
    );
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let out = volts(&trace, "out");
    let low = 5.0 * 25.0 / 1025.0;
    assert!((at(&out, 0) - low).abs() < 1e-9, "{}", at(&out, 0));
    assert!((at(&out, OFF) - low).abs() < 1e-9, "{}", at(&out, OFF));
    // GMIN on `out` holds it 5 nV under VCC.
    assert!(
        (after(&out, OFF) - 5.0).abs() < 1e-8,
        "{}",
        after(&out, OFF)
    );
}

/// The pin drives high through a 1 V drop from power-on, and low through a 0.4 V drop from
/// 1 us, into 1 kohm to ground. A drop is a source, so a source-stepping solve at half scale
/// reads half the pin.
#[test]
fn an_output_stages_drop_scales_with_the_sources() {
    let netlist = circuit(2 * US, &["out"], &[resistor("rl", "out", "gnd", 1000.0)]);
    let mut chip = scripted(
        &netlist,
        "out",
        "out",
        &[(0, Op::Drive(Drive::High)), (US, Op::Drive(Drive::Low))],
        None,
    );
    chip.outputs[0].high.drop = 1.0;
    chip.outputs[0].low.drop = 0.4;
    let load = 1000.0 / 1025.0;
    let model = Circuit::with_actives(netlist.clone(), vec![chip.clone()]).unwrap();
    let cores = Actives::power_on(&model);
    let half = Conditions {
        source_scale: 0.5,
        cores: &cores,
        ..Conditions::NOMINAL
    };
    let solved = newton(&model, None, &half, Iterate::initial(&model), op::OP_BUDGET).unwrap();
    let pin = solved.iterate.solution.voltage(net(&netlist, "out"));
    assert!((pin - 0.5 * 4.0 * load).abs() < 1e-9, "{pin}");
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let out = volts(&trace, "out");
    assert!((at(&out, 0) - 4.0 * load).abs() < 1e-9, "{}", at(&out, 0));
    assert!(
        (after(&out, US) - 0.4 * load).abs() < 1e-9,
        "{}",
        after(&out, US)
    );
}

/// At `T`, off the grid, a switch closes, a source steps from 0 to 2 V and the chip drives
/// high. One step lands there with every old state. One restart takes all three, so the step
/// 1 ns later has all three in force.
#[test]
fn a_chip_stop_a_switch_edge_and_a_source_jump_share_one_landing() {
    const T: u64 = US + 500 * NS + 17;
    let switch = format!(
        r#"{{ "name": "sw", "kind": "switch", "pins": {{ "a": "vcc", "b": "sw" }},
              "on_resistance": {{ "value": 10.0, "unit": "ohm", "source": "user" }},
              "off_resistance": {{ "value": 1e6, "unit": "ohm", "source": "user" }},
              "initial": "open", "events": [{{ "t_ps": {T}, "state": "closed" }}], "bounce": null }}"#
    );
    let step = format!(
        r#"{{ "name": "vs", "kind": "voltage_source", "pins": {{ "p": "src", "n": "gnd" }},
              "voltage": {{ "kind": "step", "initial": {{ "value": 0.0, "unit": "V", "source": "user" }},
                           "final": {{ "value": 2.0, "unit": "V", "source": "user" }}, "at_ps": {T} }} }}"#
    );
    let netlist = circuit(
        3 * US,
        &["out", "sw", "src"],
        &[
            resistor("rl", "out", "gnd", 1000.0),
            switch,
            resistor("rs", "sw", "gnd", 1000.0),
            step,
            resistor("rsrc", "src", "gnd", 1000.0),
        ],
    );
    let chip = scripted(&netlist, "out", "out", &[(T, Op::Drive(Drive::High))], None);
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let (out, sw, src) = (
        volts(&trace, "out"),
        volts(&trace, "sw"),
        volts(&trace, "src"),
    );
    assert_eq!(out.iter().filter(|&&(t, _)| t == T).count(), 1);
    assert!(restarted(&out, T));
    let near = |rows: &[(u64, f64)], ps: u64, v: f64| (at(rows, ps) - v).abs() < 1e-9;
    assert!(near(&out, T, 0.0) && near(&src, T, 0.0));
    assert!(near(&sw, T, 5.0 * 1000.0 / 1_001_000.0));
    assert!(near(&out, T + NS, 5.0 * 1000.0 / 1025.0));
    assert!(near(&src, T + NS, 2.0));
    assert!(near(&sw, T + NS, 5.0 * 1000.0 / 1010.0));
}
