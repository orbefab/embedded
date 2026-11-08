use orbe_engine_core::netlist::Netlist;
use orbe_engine_core::run_folder::{self, Results};
use orbe_engine_core::serial::SerialByte;
use orbe_engine_core::timestamp::Timestamp;

use super::super::*;
use super::active::{NS, US, capacitor, circuit, resistor, scripted};
use super::fixtures::*;
use crate::active;
use crate::active::Drive;
use crate::active::script::Op;
use crate::lte::Segment;
use crate::newton::Side;
use crate::session::Budget;
use crate::session::{SerialCursor, State, Status, attempt};

/// A seeded xorshift, so a failing chunking replays.
pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

/// `packages/engine/tests/circuits/<name>`, its stop, step and grid cut to `stop_ps` at most.
pub(super) fn example(name: &str, stop_ps: u64) -> Vec<u8> {
    let path = format!(
        "{}/../tests/circuits/{name}/netlist.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let transient = &mut json["analysis"]["transient"];
    for key in ["stop_ps", "max_step_ps", "grid_ps"] {
        transient[key] = transient[key].as_u64().unwrap().min(stop_ps).into();
    }
    serde_json::to_vec(&json).unwrap()
}

/// The results `simulate` gives, with each step's Newton budget and the step limit from `budget`.
pub(super) fn batch(netlist: &Netlist, budget: Budget) -> Results {
    let mut diagnostics = Vec::new();
    let outcome = run_with(
        netlist,
        &transient(netlist),
        Vec::new(),
        budget,
        Rows::Grid,
        &mut diagnostics,
    );
    Results::new(outcome.observations, outcome.trace, diagnostics)
}

pub(super) fn finish(session: Session) -> Results {
    let mut diagnostics = Vec::new();
    let outcome = session.outcome(&mut diagnostics);
    Results::new(outcome.observations, outcome.trace, diagnostics)
}

/// Each run folder file is the same bytes, rendered at timestamp 0.
pub(super) fn assert_same_folder(
    input: &[u8],
    netlist: &Netlist,
    session: &Results,
    batch: &Results,
) {
    let render = |results| {
        run_folder::render(
            input,
            Some(netlist),
            results,
            Timestamp::from_unix_seconds(0),
        )
    };
    let (session, batch) = (render(session), render(batch));
    let names = |folder: &run_folder::RunFolder| -> Vec<String> {
        folder.files().map(|(name, _)| name.to_string()).collect()
    };
    assert_eq!(names(&session), names(&batch));
    for ((name, got), (_, want)) in session.files().zip(batch.files()) {
        assert!(got == want, "{name} differs");
    }
}

/// `packages/engine/tests/circuits/<name>` cut to `stop_ps` advanced in seeded chunks writes the batch's run folder.
fn chunks_write_the_batch_folder(name: &str, stop_ps: u64, budget: Budget, seed: u64) -> Results {
    input_in_chunks_writes_the_batch_folder(name, &example(name, stop_ps), budget, seed)
}

/// The serial bytes of a session on `netlist` advanced to its end in one call.
pub(super) fn serial_of_one_call(netlist: &Netlist, budget: Budget) -> Vec<SerialByte> {
    let mut session = open(netlist, Vec::new(), budget);
    session.advance_to(transient(netlist).stop);
    session
        .serial_since(SerialCursor::default())
        .expect("cursor still held")
        .0
        .to_vec()
}

/// `input` advanced in seeded chunks to its end writes the batch's run folder. A chunk ends on
/// the step about to land, 1 ps either side of it, on the next grid time, or a short or long
/// stride on. The probe at each chunk end that falls on a row is that row, bit for bit. The
/// serial bytes read after each chunk, joined, are the bytes of one call.
fn input_in_chunks_writes_the_batch_folder(
    name: &str,
    input: &[u8],
    budget: Budget,
    seed: u64,
) -> Results {
    let netlist = orbe_engine_core::netlist::parse(input).unwrap();
    let transient = transient(&netlist);
    let batch = batch(&netlist, budget);
    let trace = batch.trace().unwrap();
    let mut session = open(&netlist, Vec::new(), budget);
    let mut rng = Rng(seed);
    let mut on_rows = 0;
    let (mut cursor, mut serial) = (SerialCursor::default(), Vec::new());
    while let State::Running { now, next, .. } = &session.state {
        let (now, landing) = (now.as_ps(), next.at.as_ps());
        let to = match rng.below(6) {
            0 => landing,
            1 => landing - 1,
            2 => landing + 1,
            3 => transient.next_row_after(Time::from_ps(now)).as_ps(),
            4 => now + 1 + rng.below(1_000),
            _ => now + 1 + rng.below(transient.stop.as_ps() / 100),
        };
        session.advance_to(Time::from_ps(to));
        let (bytes, after) = session.serial_since(cursor).expect("cursor still held");
        serial.extend_from_slice(bytes);
        cursor = after;
        let probe = session.probe();
        let at = probe.at();
        assert_eq!(at, session.now());
        if let Ok(k) = trace.time().binary_search(&at) {
            for (j, value) in probe.values().iter().enumerate() {
                assert_eq!(
                    value.get().to_bits(),
                    trace.column(j)[k].to_bits(),
                    "{name} {} at {at:?}",
                    trace.columns()[j]
                );
            }
            on_rows += 1;
        }
    }
    assert!(on_rows > 0, "{name}: no chunk ended on a row");
    assert_eq!(serial, serial_of_one_call(&netlist, budget), "{name}");
    assert_same_folder(input, &netlist, &finish(session), &batch);
    batch
}

pub(super) const AVR_STOP_PS: u64 = 50_000_000_000;

#[test]
fn atmega_analog_serial_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("atmega-analog-serial", u64::MAX, BUDGET, 0x5eed_0001);
}

#[test]
fn atmega_blink_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("atmega-blink", AVR_STOP_PS, BUDGET, 0x5eed_0002);
}

#[test]
fn atmega_button_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("atmega-button", AVR_STOP_PS, BUDGET, 0x5eed_0003);
}

#[test]
fn atmega_fade_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("atmega-fade", AVR_STOP_PS, BUDGET, 0x5eed_0004);
}

#[test]
fn button_led_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("button-led", u64::MAX, BUDGET, 0x5eed_0005);
}

#[test]
fn current_pulse_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("current-pulse", u64::MAX, BUDGET, 0x5eed_0006);
}

#[test]
fn ne555_led_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("ne555-led", u64::MAX, BUDGET, 0x5eed_0007);
}

#[test]
fn rc_switch_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("rc-switch", u64::MAX, BUDGET, 0x5eed_0008);
}

#[test]
fn v_pulse_rc_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("v-pulse-rc", u64::MAX, BUDGET, 0x5eed_0009);
}

#[test]
fn npn_blinker_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("npn-blinker", u64::MAX, BUDGET, 0x5eed_000b);
}

#[test]
fn relay_flyback_in_chunks_writes_the_batch_folder() {
    chunks_write_the_batch_folder("relay-flyback", u64::MAX, BUDGET, 0x5eed_000c);
}

/// 1 V through 1 kohm into 30 pF, which a 1 ohm switch shorts at 5.5 ns into a 30 ps mode,
/// beside 10 pF held at 1 V that discharges through 1 kohm. The steps after the edge stiffen
/// `cf`, and the 1 ns grid puts rows inside them.
pub(super) const STIFF_ROW: &[u8] = br#"{
  "version": 0,
  "seed": 0,
  "analysis": { "transient": { "stop_ps": 200000, "max_step_ps": 5000, "grid_ps": 1000 } },
  "ground": "gnd",
  "nets": ["gnd", "vs", "vcc", "vf"],
  "parts": [
    {
      "name": "cs",
      "kind": "capacitor",
      "pins": { "a": "vs", "b": "gnd" },
      "capacitance": { "value": 1e-11, "unit": "F", "source": "user" },
      "initial": { "value": 1.0, "unit": "V", "source": "user" }
    },
    {
      "name": "rs",
      "kind": "resistor",
      "pins": { "a": "vs", "b": "gnd" },
      "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    },
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vcc", "n": "gnd" },
      "voltage": { "value": 1.0, "unit": "V", "source": "user" }
    },
    {
      "name": "rf",
      "kind": "resistor",
      "pins": { "a": "vcc", "b": "vf" },
      "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    },
    {
      "name": "cf",
      "kind": "capacitor",
      "pins": { "a": "vf", "b": "gnd" },
      "capacitance": { "value": 3.003e-11, "unit": "F", "source": "user" },
      "initial": null
    },
    {
      "name": "sw1",
      "kind": "switch",
      "pins": { "a": "vf", "b": "gnd" },
      "on_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
      "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
      "initial": "open",
      "events": [{ "t_ps": 5500, "state": "closed" }],
      "bounce": null
    }
  ]
}"#;

#[test]
fn stiff_row_in_chunks_writes_the_batch_folder() {
    input_in_chunks_writes_the_batch_folder("stiff-row", STIFF_ROW, BUDGET, 0x5eed_000a);
}

/// `npn-blinker` cut to 1 us on a 1 ns grid. A ringing stiffen there retries a step that
/// holds a grid row.
fn long_stiff_blinker() -> Vec<u8> {
    let path = format!(
        "{}/../tests/circuits/npn-blinker/netlist.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let transient = &mut json["analysis"]["transient"];
    transient["stop_ps"] = serde_json::json!(1_000_000);
    transient["max_step_ps"] = serde_json::json!(1_000_000);
    transient["grid_ps"] = serde_json::json!(1_000);
    serde_json::to_vec(&json).unwrap()
}

/// Grid rows strictly inside a stiffened step longer than 1 ps, each solved on that step's
/// stiffened segment. The batch trace has to match these bits.
fn rows_inside_long_stiffened_steps(netlist: &Netlist) -> Vec<(Time, Vec<u64>)> {
    let spec = transient(netlist);
    let mut session = open(netlist, Vec::new(), BUDGET);
    let mut rows = Vec::new();
    let mut longest = 0u64;
    loop {
        let at = {
            let State::Running { accepted, next, .. } = &session.state else {
                break;
            };
            let h = next.at.as_ps() - accepted.at.as_ps();
            if next.stiffened.is_some() {
                longest = longest.max(h);
            }
            if let Some(stiffened) = &next.stiffened
                && h > 1
            {
                let mut row = spec.next_row_after(accepted.at);
                while row < next.at {
                    let step = row.as_ps() - accepted.at.as_ps();
                    let solve = |segment: &Segment| {
                        let cores = session.actives.cores();
                        let converged = attempt(
                            &session.circuit,
                            None,
                            accepted,
                            segment,
                            cores,
                            step,
                            &accepted.iterate,
                            BUDGET.row,
                        )
                        .unwrap();
                        observe::row(
                            &session.circuit,
                            &session.columns,
                            &converged.iterate.solution,
                            row,
                            Side::Before,
                        )
                        .unwrap()
                    };
                    let stiff = solve(stiffened);
                    let smooth = solve(&accepted.segment);
                    let stiff_bits: Vec<u64> = stiff.iter().map(|v| v.get().to_bits()).collect();
                    let smooth_bits: Vec<u64> = smooth.iter().map(|v| v.get().to_bits()).collect();
                    assert_ne!(
                        stiff_bits, smooth_bits,
                        "the stiffen at {row:?} moves the row"
                    );
                    rows.push((row, stiff_bits));
                    let next_row = spec.next_row_after(row);
                    if next_row <= row {
                        break;
                    }
                    row = next_row;
                }
            }
            next.at
        };
        session.advance_to(at);
    }
    assert!(
        !rows.is_empty(),
        "no grid row falls inside a stiffened step longer than 1 ps, longest {longest} ps"
    );
    rows
}

/// A call that stops strictly inside a long stiffened step reads that step's stiffened
/// solve, and the run still writes the batch's folder.
#[test]
fn a_call_inside_a_stiffened_step_holds_it() {
    let input = long_stiff_blinker();
    let netlist = orbe_engine_core::netlist::parse(&input).unwrap();
    let batch = batch(&netlist, BUDGET);
    let spec = transient(&netlist);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    let inside = loop {
        let at = {
            let State::Running { accepted, next, .. } = &session.state else {
                panic!("the run ends at {:?}", spec.stop);
            };
            let row = spec.next_row_after(accepted.at);
            if next.stiffened.is_some()
                && next.at.as_ps() - accepted.at.as_ps() > 1
                && accepted.at < row
                && row < next.at
            {
                break row;
            }
            next.at
        };
        session.advance_to(at);
    };
    session.advance_to(inside);
    let probe: Vec<u64> = session
        .probe()
        .values()
        .iter()
        .map(|value| value.get().to_bits())
        .collect();
    {
        let State::Running {
            accepted,
            now,
            next,
            ..
        } = &session.state
        else {
            panic!("the run ends at {:?}", spec.stop);
        };
        assert!(
            next.stiffened.is_some(),
            "{inside:?} leaves the retry waiting"
        );
        assert!(accepted.at < inside && inside < next.at && *now == inside);
        let h = inside.as_ps() - accepted.at.as_ps();
        let stiffened = next.stiffened.clone().unwrap();
        let solve = |segment: &Segment| {
            let cores = session.actives.cores();
            let converged = attempt(
                &session.circuit,
                None,
                accepted,
                segment,
                cores,
                h,
                &accepted.iterate,
                BUDGET.row,
            )
            .unwrap();
            observe::row(
                &session.circuit,
                &session.columns,
                &converged.iterate.solution,
                inside,
                Side::Before,
            )
            .unwrap()
        };
        let stiff: Vec<u64> = solve(&stiffened)
            .iter()
            .map(|value| value.get().to_bits())
            .collect();
        let smooth: Vec<u64> = solve(&accepted.segment)
            .iter()
            .map(|value| value.get().to_bits())
            .collect();
        assert_ne!(stiff, smooth, "the stiffen at {inside:?} moves the probe");
        assert_eq!(
            probe, stiff,
            "the probe at {inside:?} is the stiffened solve"
        );
    }
    session.advance_to(spec.stop);
    assert_same_folder(&input, &netlist, &finish(session), &batch);
}

/// A grid row inside a step that a stiffen retried equals a direct solve on the stiffened
/// segment, and differs from a solve on the accepted segment.
#[test]
fn a_row_inside_a_stiffened_step_is_solved_on_the_stiffened_segment() {
    let netlist = orbe_engine_core::netlist::parse(&long_stiff_blinker()).unwrap();
    let rows = rows_inside_long_stiffened_steps(&netlist);
    let results = batch(&netlist, BUDGET);
    let trace = results.trace().unwrap();
    for (at, want) in rows {
        let k = trace.time().binary_search(&at).unwrap();
        for (j, bits) in want.iter().enumerate() {
            assert_eq!(
                trace.column(j)[k].to_bits(),
                *bits,
                "{} at {at:?}",
                trace.columns()[j]
            );
        }
    }
}

#[test]
fn a_session_that_reaches_the_step_limit_in_chunks_writes_the_batch_folder() {
    let budget = Budget {
        steps: 157,
        ..BUDGET
    };
    for name in ["rc-switch", "atmega-analog-serial"] {
        let batch = chunks_write_the_batch_folder(name, u64::MAX, budget, 0x5eed_0010);
        assert!(
            batch
                .diagnostics()
                .iter()
                .any(|d| matches!(d, Diagnostic::SolverStepLimit { .. })),
            "{name} reaches the step limit"
        );
    }
}

/// The accepted step's time, `now`, the next landing's time and the landings so far.
fn standing(session: &Session) -> (Time, Time, Time, u64) {
    let State::Running {
        accepted,
        now,
        next,
    } = &session.state
    else {
        panic!("the session ended");
    };
    (accepted.at, *now, next.at, session.stats.landings)
}

#[test]
fn a_target_inside_the_first_step_accepts_nothing_and_a_second_call_there_changes_nothing() {
    let netlist = led_switch();
    let mut session = open(&netlist, Vec::new(), BUDGET);
    let first = standing(&session).2;
    session.advance_to(Time::from_ps(1));
    let there = standing(&session);
    assert_eq!(there, (Time::ZERO, Time::from_ps(1), first, 0));
    session.advance_to(Time::from_ps(1));
    assert_eq!(standing(&session), there);
    session.advance_to(Time::ZERO);
    assert_eq!(standing(&session), there);
    session.advance_to(transient(&netlist).stop);
    assert_same_folder(&[], &netlist, &finish(session), &batch(&netlist, BUDGET));
}

#[test]
fn a_target_past_stop_ends_at_stop() {
    let netlist = led_switch();
    let stop = transient(&netlist).stop;
    let mut session = open(&netlist, Vec::new(), BUDGET);
    let status = session.advance_to(Time::from_ps(stop.as_ps() + 1_000_000));
    assert_eq!(status, Status::Stopped);
    assert_eq!(session.now(), stop);
    assert_same_folder(&[], &netlist, &finish(session), &batch(&netlist, BUDGET));
}

/// A step limit of 0 fails the step after t = 0, so the session has faulted at t = 0 before
/// any call.
#[test]
fn a_session_can_end_as_it_opens() {
    let netlist = led_switch();
    let session = open(&netlist, Vec::new(), Budget { steps: 0, ..BUDGET });
    assert_eq!(session.status(), Status::Faulted);
    assert_eq!(session.now(), Time::ZERO);
    assert_eq!(
        session.fault(),
        Some(&Diagnostic::SolverStepLimit {
            at: Time::ZERO,
            limit: 0,
        })
    );
}

/// A step limit of 3 fails the step after the third as soon as the third is accepted. Calls
/// 137 ps apart reach it, and the call that accepts the third returns the fault. Each call's
/// status stays bound while the session is read.
#[test]
fn the_call_that_accepts_the_last_good_step_returns_the_fault() {
    let netlist = led_switch();
    let mut session = open(&netlist, Vec::new(), Budget { steps: 3, ..BUDGET });
    let mut to = Time::ZERO;
    loop {
        let before = session.now();
        to = Time::from_ps(to.as_ps() + 137);
        let status = session.advance_to(to);
        let (now, probe, fault) = (session.now(), session.probe(), session.fault());
        assert_eq!(probe.at(), now);
        match status {
            Status::Running => assert_eq!((now, fault), (to, None)),
            Status::Faulted => {
                assert!(before < now && now <= to, "{before:?} {now:?} {to:?}");
                assert_eq!(
                    fault,
                    Some(&Diagnostic::SolverStepLimit { at: now, limit: 3 })
                );
                assert_eq!(session.stats.landings, 3);
                return;
            }
            Status::Stopped => panic!("the run reaches stop"),
        }
    }
}

/// A session that faulted stays at its last accepted step. Every later call returns the same
/// fault and moves nothing.
#[test]
fn now_stays_put_after_a_fault() {
    let netlist = led_switch();
    let stop = transient(&netlist).stop;
    let mut session = open(&netlist, Vec::new(), Budget { steps: 3, ..BUDGET });
    assert_eq!(session.advance_to(stop), Status::Faulted);
    let (at, values, fault) = (
        session.now(),
        session.probe().values().to_vec(),
        session.fault().cloned(),
    );
    assert!(at < stop, "{at:?}");
    for to in [Time::from_ps(at.as_ps() + 1), stop, Time::ZERO] {
        assert_eq!(session.advance_to(to), Status::Faulted);
        assert_eq!(session.now(), at);
        assert_eq!(session.probe().values(), values);
        assert_eq!(session.fault().cloned(), fault);
    }
}

/// The switch closes at 1 us. A chunk that ends 1 ps before it stands in front of the edge, and
/// the next one lands on it and restarts there.
#[test]
fn chunks_that_end_beside_a_switch_edge_land_on_it_once() {
    let netlist = led_switch();
    let edge = Time::from_ps(1_000_000);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.advance_to(Time::from_ps(edge.as_ps() - 1));
    let (accepted, now, next, _) = standing(&session);
    assert!(
        accepted < now,
        "{accepted:?} is a landing before the chunk end"
    );
    assert_eq!((now, next), (Time::from_ps(edge.as_ps() - 1), edge));
    session.advance_to(edge);
    let State::Running { accepted, now, .. } = &session.state else {
        panic!("the session ended at the edge");
    };
    assert_eq!((accepted.at, *now), (edge, edge));
    assert!(accepted.restart_row.is_some(), "the edge restarts");
    session.advance_to(transient(&netlist).stop);
    assert_same_folder(&[], &netlist, &finish(session), &batch(&netlist, BUDGET));
}

/// 0 to 5 V at 1 us through 1 kohm into 0.3 nF on `in`, which 2 kohm and an LED load. The chip
/// watches `in` and drives `out` low from 100 ns. With two Newton iterations a step, the step
/// that crosses 2.5 V stalls before it converges.
fn stalled_crossing() -> (Netlist, active::Chip) {
    let step = r#"{ "name": "vs", "kind": "voltage_source", "pins": { "p": "src", "n": "gnd" },
          "voltage": { "kind": "step", "initial": { "value": 0.0, "unit": "V", "source": "user" },
                       "final": { "value": 5.0, "unit": "V", "source": "user" }, "at_ps": 1000000 } }"#;
    let led = r#"{ "name": "led1", "kind": "led", "pins": { "anode": "k", "cathode": "gnd" },
          "saturation_current": { "value": 6.711e-10, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 4.427, "unit": "1", "source": "user" },
          "series_resistance": { "value": 2.115, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null } }"#;
    let netlist = circuit(
        3 * US,
        &["out", "src", "in", "k"],
        &[
            resistor("rl", "out", "gnd", 1000.0),
            step.to_string(),
            resistor("rs", "src", "in", 1000.0),
            capacitor("cs", "in", 0.3e-9),
            resistor("r3", "in", "k", 2000.0),
            led.to_string(),
        ],
    );
    let chip = scripted(
        &netlist,
        "out",
        "in",
        &[(100 * NS, Op::Drive(Drive::Low))],
        None,
    );
    (netlist, chip)
}

/// A step that stalls is halved, and a watch then cuts the halved step. The step after the cut
/// is planned from the halved one, which is the step it wanted, and not from the plan before
/// the stall.
#[test]
fn a_cut_after_a_stall_plans_from_the_halved_step() {
    let (netlist, chip) = stalled_crossing();
    let mut session = open(&netlist, vec![chip], Budget { step: 2, ..BUDGET });
    let mut cuts = 0;
    while let State::Running { next, .. } = &session.state {
        let (at, wanted) = (next.at, next.plan);
        let halved = next.cut && wanted != session.plan;
        session.advance_to(at);
        if let State::Running { accepted, .. } = &session.state
            && halved
            && !matches!(accepted.segment, Segment::Start { .. })
        {
            assert_eq!(session.plan, wanted, "the plan after the cut at {at:?}");
            cuts += 1;
        }
    }
    assert!(cuts > 0, "a watch cuts a halved step");
}
