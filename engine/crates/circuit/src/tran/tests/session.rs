use sim_core::netlist::Netlist;
use sim_core::run_folder::{self, Results};
use sim_core::timestamp::Timestamp;

use super::super::*;
use super::active::{NS, US, capacitor, circuit, resistor, scripted};
use super::fixtures::*;
use crate::active::Drive;
use crate::active::script::Op;
use crate::topology;

/// A seeded xorshift, so a failing chunking replays.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

/// `examples/<name>`, its stop, step and grid cut to `stop_ps` at most.
fn example(name: &str, stop_ps: u64) -> Vec<u8> {
    let path = format!(
        "{}/../../examples/{name}/netlist.json",
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
fn batch(netlist: &Netlist, budget: Budget) -> Results {
    let mut diagnostics = topology::floating_nets(netlist);
    let outcome = run_with(
        netlist,
        &transient(netlist),
        Vec::new(),
        budget,
        Rows::Grid,
        Columns::new(netlist),
        &mut diagnostics,
    );
    Results::new(outcome.observations, outcome.trace, diagnostics)
}

fn finish(session: Session<'_>, netlist: &Netlist) -> Results {
    let mut diagnostics = topology::floating_nets(netlist);
    let outcome = session.finish(&mut diagnostics);
    Results::new(outcome.observations, outcome.trace, diagnostics)
}

/// Each run folder file is the same bytes, rendered at timestamp 0.
fn assert_same_folder(input: &[u8], netlist: &Netlist, session: &Results, batch: &Results) {
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

/// The row at the time the session stands at.
fn read(session: &Session<'_>) -> (Time, Vec<Finite>) {
    match &session.state {
        State::Running { accepted, now, .. } if *now == accepted.at => (*now, accepted.row.clone()),
        State::Running { now, .. } => match session.span().unwrap().probe(*now) {
            Probed::Solved(row) | Probed::Interpolated(row) => (*now, row),
        },
        State::Ended { at, row, .. } => (*at, row.clone()),
    }
}

/// `examples/<name>` cut to `stop_ps` advanced in seeded chunks writes the batch's run folder.
fn chunks_write_the_batch_folder(name: &str, stop_ps: u64, budget: Budget, seed: u64) -> Results {
    input_in_chunks_writes_the_batch_folder(name, &example(name, stop_ps), budget, seed)
}

/// `input` advanced in seeded chunks to its end writes the batch's run folder. A chunk ends on
/// the step about to land, 1 ps either side of it, on the next grid time, or a short or long
/// stride on. The read at each chunk end that falls on a row is that row, bit for bit.
fn input_in_chunks_writes_the_batch_folder(
    name: &str,
    input: &[u8],
    budget: Budget,
    seed: u64,
) -> Results {
    let netlist = sim_core::netlist::parse(input).unwrap();
    let transient = transient(&netlist);
    let batch = batch(&netlist, budget);
    let trace = batch.trace().unwrap();
    let mut session = open(&netlist, Vec::new(), budget);
    let mut rng = Rng(seed);
    let mut on_rows = 0;
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
        let (at, row) = read(&session);
        if let Ok(k) = trace.time().binary_search(&at) {
            for (j, value) in row.iter().enumerate() {
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
    assert_same_folder(input, &netlist, &finish(session, &netlist), &batch);
    batch
}

const AVR_STOP_PS: u64 = 50_000_000_000;

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

/// 1 V through 1 kohm into 30 pF, which a 1 ohm switch shorts at 5.5 ns into a 30 ps mode,
/// beside 10 pF held at 1 V that discharges through 1 kohm. The steps after the edge stiffen
/// `cf`, and the 1 ns grid puts rows inside them.
const STIFF_ROW: &[u8] = br#"{
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

/// A call that ends inside a stiffened step holds that landing, and the run still writes the
/// batch's folder.
#[test]
fn a_call_inside_a_stiffened_step_holds_it() {
    let netlist = sim_core::netlist::parse(STIFF_ROW).unwrap();
    let batch = batch(&netlist, BUDGET);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    let inside = Time::from_ps(7000);
    session.advance_to(inside);
    let State::Running {
        accepted,
        now,
        next,
        ..
    } = &session.state
    else {
        panic!("the run ends at {:?}", transient(&netlist).stop);
    };
    assert!(
        next.stiffened.is_some(),
        "{inside:?} is inside a stiffened step"
    );
    assert!(accepted.at < inside && inside < next.at && *now == inside);
    session.advance_to(transient(&netlist).stop);
    assert_same_folder(STIFF_ROW, &netlist, &finish(session, &netlist), &batch);
}

/// A row inside a step that a stiffen retried is solved on the stiffened segment, the one the
/// landing converged on, and not on the accepted step's.
#[test]
fn a_row_inside_a_stiffened_step_is_solved_on_the_stiffened_segment() {
    let netlist = sim_core::netlist::parse(STIFF_ROW).unwrap();
    let transient = transient(&netlist);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    let mut rows = Vec::new();
    while let State::Running { accepted, next, .. } = &session.state {
        let row = transient.next_row_after(accepted.at);
        if let Some(stiffened) = &next.stiffened
            && row < next.at
        {
            let solve = |segment| {
                let h = row.as_ps() - accepted.at.as_ps();
                let cores = session.actives.cores();
                let converged = attempt(
                    &session.circuit,
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
                    row,
                    Side::Before,
                )
                .unwrap()
            };
            let (stiff, smooth) = (solve(stiffened), solve(&accepted.segment));
            assert_ne!(stiff, smooth, "the stiffen at {row:?} moves the row");
            rows.push((row, stiff));
        }
        let at = next.at;
        session.advance_to(at);
    }
    assert!(!rows.is_empty(), "a grid row falls inside a stiffened step");
    let results = finish(session, &netlist);
    let trace = results.trace().unwrap();
    for (at, want) in rows {
        let k = trace.time().binary_search(&at).unwrap();
        for (j, value) in want.iter().enumerate() {
            assert_eq!(
                trace.column(j)[k].to_bits(),
                value.get().to_bits(),
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
fn standing(session: &Session<'_>) -> (Time, Time, Time, u64) {
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
    assert_same_folder(
        &[],
        &netlist,
        &finish(session, &netlist),
        &batch(&netlist, BUDGET),
    );
}

#[test]
fn a_target_past_stop_ends_at_stop() {
    let netlist = led_switch();
    let stop = transient(&netlist).stop;
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.advance_to(Time::from_ps(stop.as_ps() + 1_000_000));
    assert!(
        matches!(session.state, State::Ended { at, end: End::Stop, .. } if at == stop),
        "the session ends at stop"
    );
    assert_same_folder(
        &[],
        &netlist,
        &finish(session, &netlist),
        &batch(&netlist, BUDGET),
    );
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
    assert_same_folder(
        &[],
        &netlist,
        &finish(session, &netlist),
        &batch(&netlist, BUDGET),
    );
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
