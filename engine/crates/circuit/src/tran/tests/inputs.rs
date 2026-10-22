use std::collections::BTreeMap;

use sim_core::diagnostic::Diagnostic;
use sim_core::name::Name;
use sim_core::netlist::{self, Contact, Document, Model, Netlist, Part, Switch};

use super::super::*;
use super::fixtures::*;
use super::session::{AVR_STOP_PS, Rng, STIFF_ROW, assert_same_folder, batch, example, finish};
use crate::session::{Applied, Input, Rejected, Setting, State, Status};

/// 1 V through 1 kohm into 5 pF, a 5 ns time constant, so the 1 ns first step after an edge is
/// rejected. `sw1` puts 1 kohm across the capacitor and `sw2` puts 10 kohm across it, with up
/// to two bounces in 50 ns. Neither has an event.
const RC_SWITCHES: &[u8] = br#"{
  "version": 0,
  "seed": 3,
  "analysis": { "transient": { "stop_ps": 20000000, "max_step_ps": 100000, "grid_ps": 500000 } },
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
      "capacitance": { "value": 5e-12, "unit": "F", "source": "user" },
      "initial": null
    },
    {
      "name": "sw1",
      "kind": "switch",
      "pins": { "a": "vout", "b": "gnd" },
      "on_resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
      "initial": "open",
      "events": [],
      "bounce": null
    },
    {
      "name": "sw2",
      "kind": "switch",
      "pins": { "a": "vout", "b": "gnd" },
      "on_resistance": { "value": 10000.0, "unit": "ohm", "source": "user" },
      "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
      "initial": "open",
      "events": [],
      "bounce": {
        "window": { "value": 50000, "unit": "ps", "source": "user" },
        "max_bounces": { "value": 2, "unit": "1", "source": "user" }
      }
    }
  ]
}"#;

fn without_events(input: &[u8]) -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(input).unwrap();
    for part in json["parts"].as_array_mut().unwrap() {
        if part["kind"] == "switch" {
            part["events"] = serde_json::json!([]);
        }
    }
    serde_json::to_vec(&json).unwrap()
}

fn open_document(input: &[u8], rows: Rows, budget: Budget) -> Session {
    let (document, netlist) = Document::parse(input).unwrap();
    let transient = transient(&netlist);
    let columns = Columns::new(&netlist);
    Session::open_recorded(
        netlist,
        &transient,
        Vec::new(),
        budget,
        columns,
        rows,
        Some(document),
    )
    .unwrap()
}

fn netlist_of(session: &Session) -> &Netlist {
    &session.circuit.netlist
}

fn parts(netlist: &Netlist) -> Vec<Part> {
    netlist.parts().map(|(_, part)| part.clone()).collect()
}

fn switches(netlist: &Netlist) -> Vec<Name> {
    netlist
        .parts()
        .filter(|(_, part)| matches!(part.model, Model::Switch(_)))
        .map(|(_, part)| part.name.clone())
        .collect()
}

fn switch<'n>(netlist: &'n Netlist, name: &Name) -> &'n Switch {
    let id = netlist.part_id(name).unwrap();
    let Model::Switch(switch) = &netlist.part(id).model else {
        panic!("{name:?} is not a switch");
    };
    switch
}

/// The contact after every event of the switch, scheduled or applied.
fn last_contact(switch: &Switch) -> Contact {
    switch
        .events
        .events()
        .last()
        .map_or(*switch.schedule.initial(), |event| event.to)
}

fn set(part: &str, at: u64, to: Contact) -> Input {
    Input {
        part: Name::new(part).unwrap(),
        at: Time::from_ps(at),
        setting: Setting::Contact(to),
    }
}

fn accepted_at(session: &Session) -> Time {
    let State::Running { accepted, .. } = &session.state else {
        panic!("the session ended");
    };
    accepted.at
}

/// What the landing rule gives `input`: the first time from the one asked for, `now`, 1 ps
/// after the accepted step and its reach that the part's schedule rules allow, before `stop`.
fn expected(session: &Session, input: &Input) -> Result<Time, Rejected> {
    let State::Running { accepted, now, .. } = &session.state else {
        return Err(Rejected::Ended);
    };
    let switch = switch(netlist_of(session), &input.part);
    let Setting::Contact(to) = input.setting;
    let after = Time::from_ps(accepted.at.as_ps() + 1);
    let floor = input.at.max(*now).max(after).max(accepted.reach);
    match switch.earliest(floor) {
        Some(at) if at < transient(netlist_of(session)).stop => {
            if to == last_contact(switch) {
                Err(Rejected::NoChange)
            } else {
                Ok(at)
            }
        }
        _ => Err(Rejected::PastStop),
    }
}

/// A seeded input on a seeded switch. It asks for `now`, a time before it, 1 ps after it, a
/// time just before the accepted step's reach, the held landing or a stride on. It flips the
/// contact or, one time in four, keeps it.
fn random_input(session: &Session, switches: &[Name], rng: &mut Rng) -> Input {
    let State::Running {
        accepted,
        now,
        next,
    } = &session.state
    else {
        panic!("the session ended");
    };
    let stop = transient(netlist_of(session)).stop.as_ps();
    let now = now.as_ps();
    let at = match rng.below(6) {
        0 => now,
        1 => now.saturating_sub(1 + rng.below(1_000)),
        2 => now + 1,
        3 => accepted.reach.as_ps().saturating_sub(rng.below(1_000)),
        4 => next.at.as_ps(),
        _ => now + 1 + rng.below(stop / 20),
    };
    let part = switches[rng.below(switches.len() as u64) as usize].clone();
    let last = last_contact(switch(netlist_of(session), &part));
    let to = if rng.below(4) == 0 {
        last
    } else {
        last.flipped()
    };
    Input {
        part,
        at: Time::from_ps(at),
        setting: Setting::Contact(to),
    }
}

/// Advances `session` to its end and checks that a batch run of its export writes the run
/// folder the session recorded, with the export as the input bytes.
fn ends_as_its_export(mut session: Session) {
    let stop = transient(netlist_of(&session)).stop;
    session.advance_to(stop);
    let export = session.export().unwrap();
    let exported = netlist::parse(&export).unwrap();
    assert_eq!(parts(&exported), parts(netlist_of(&session)));
    let batch = batch(&exported, session.budget);
    assert_same_folder(&export, &exported, &finish(session, &exported), &batch);
}

/// Whether `now` is before the accepted step's reach, where an input at `now` would cut an
/// attempt the session made.
fn inside_the_reach(session: &Session) -> bool {
    matches!(&session.state, State::Running { accepted, now, .. } if accepted.reach > *now)
}

/// `input` advanced in chunks ended as `input_in_chunks_writes_the_batch_folder` ends them,
/// with a seeded input after one chunk in three and after each chunk that ends inside the
/// accepted step's reach. Each input lands where the landing rule says. A batch run of the
/// export writes the folder the session recorded, and each probe at a row time after the last
/// input before it is that row, bit for bit.
fn inputs_in_chunks_write_the_export_folder(name: &str, input: &[u8], budget: Budget, seed: u64) {
    let mut session = open_document(input, Rows::Grid, budget);
    let transient = transient(netlist_of(&session));
    let switches = switches(netlist_of(&session));
    let mut rng = Rng(seed);
    let mut probes = Vec::new();
    let mut landed = 0;
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
        if inside_the_reach(&session) || session.status() == Status::Running && rng.below(3) == 0 {
            let input = random_input(&session, &switches, &mut rng);
            let want = expected(&session, &input);
            let accepted = accepted_at(&session);
            let got = session.apply(&input);
            match (&got, &want) {
                (Ok(got), Ok(want)) => {
                    assert_eq!(got.at, *want, "{name} {input:?}");
                    assert_eq!(got.status, session.status(), "{name} {input:?}");
                }
                (Err(got), Err(want)) => assert_eq!(got, want, "{name} {input:?}"),
                _ => panic!("{name} {input:?}: {got:?} vs {want:?}"),
            }
            if got.is_ok() {
                landed += 1;
                probes.retain(|&(at, _)| at <= accepted);
            }
        }
        let probe = session.probe();
        probes.push((probe.at(), probe.values().to_vec()));
    }
    assert!(landed > 0, "{name}: no input landed");
    let export = session.export().unwrap();
    let exported = netlist::parse(&export).unwrap();
    assert_eq!(parts(&exported), parts(netlist_of(&session)));
    let batch = batch(&exported, budget);
    let trace = batch.trace().unwrap();
    let mut on_rows = 0;
    for (at, values) in probes {
        if let Ok(k) = trace.time().binary_search(&at) {
            for (j, value) in values.iter().enumerate() {
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
    assert!(on_rows > 0, "{name}: no probe fell on a row");
    assert_same_folder(&export, &exported, &finish(session, &exported), &batch);
}

#[test]
fn atmega_button_with_inputs_writes_the_export_folder() {
    let input = without_events(&example("atmega-button", AVR_STOP_PS));
    inputs_in_chunks_write_the_export_folder("atmega-button", &input, BUDGET, 0x5eed_1001);
}

#[test]
fn button_led_with_inputs_writes_the_export_folder() {
    let input = without_events(&example("button-led", u64::MAX));
    inputs_in_chunks_write_the_export_folder("button-led", &input, BUDGET, 0x5eed_1002);
}

/// `rc-switch` keeps its own events, so an input lands after the last of them.
#[test]
fn rc_switch_with_inputs_writes_the_export_folder() {
    let input = example("rc-switch", u64::MAX);
    inputs_in_chunks_write_the_export_folder("rc-switch", &input, BUDGET, 0x5eed_1003);
}

#[test]
fn rc_switches_with_inputs_write_the_export_folder() {
    for seed in [0x5eed_1004, 0x5eed_1005] {
        inputs_in_chunks_write_the_export_folder("rc-switches", RC_SWITCHES, BUDGET, seed);
    }
}

/// Without its event, the stiff fixture's first attempts are rejected from 1 ns down, so an
/// input early in the run lands at the reach.
#[test]
fn stiff_row_with_inputs_writes_the_export_folder() {
    let input = without_events(STIFF_ROW);
    inputs_in_chunks_write_the_export_folder("stiff-row", &input, BUDGET, 0x5eed_1006);
}

#[test]
fn stiff_row_with_its_event_and_inputs_writes_the_export_folder() {
    inputs_in_chunks_write_the_export_folder("stiff-row", STIFF_ROW, BUDGET, 0x5eed_1008);
}

#[test]
fn a_session_with_inputs_that_reaches_the_step_limit_writes_the_export_folder() {
    let budget = Budget {
        steps: 157,
        ..BUDGET
    };
    let input = example("rc-switch", u64::MAX);
    inputs_in_chunks_write_the_export_folder("rc-switch", &input, budget, 0x5eed_1009);
}

/// Ok when a batch run of the session's export, with a row at every step, holds every step the
/// session accepted and then the step it holds, bit for bit. Otherwise the first that differs.
fn export_takes_the_steps(session: &Session) -> Result<(), String> {
    let State::Running { next, .. } = &session.state else {
        panic!("the session ended");
    };
    let exported = netlist::parse(&session.export().unwrap()).unwrap();
    let mut diagnostics = Vec::new();
    let outcome = run(
        &exported,
        &transient(&exported),
        Rows::Steps,
        &mut diagnostics,
    );
    let batch = outcome.trace.unwrap();
    let live = &session.record.as_ref().unwrap().trace;
    let row = |trace: &Trace, k: usize| {
        let values: Vec<u64> = (0..trace.columns().len())
            .map(|j| trace.column(j)[k].to_bits())
            .collect();
        (trace.time()[k], values)
    };
    for k in 0..live.rows() {
        if k >= batch.rows() || row(&batch, k) != row(live, k) {
            return Err(format!("step {k} at {:?}", live.time()[k]));
        }
    }
    let held = (
        next.at,
        next.row.iter().map(|value| value.get().to_bits()).collect(),
    );
    let k = live.rows();
    if k >= batch.rows() || row(&batch, k) != held {
        return Err(format!("the held landing at {:?}", next.at));
    }
    Ok(())
}

/// `input` advanced in seeded chunks with seeded inputs to a seeded time in the middle of the
/// run, then exported there.
fn an_export_mid_run_takes_every_step_the_session_took(name: &str, input: &[u8], seed: u64) {
    let mut session = open_document(input, Rows::Steps, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    let switches = switches(netlist_of(&session));
    let mut rng = Rng(seed);
    let until = Time::from_ps(stop / 4 + rng.below(stop / 2));
    let mut landed = 0;
    while session.now() < until {
        let now = session.now().as_ps();
        session.advance_to(Time::from_ps(now + 1 + rng.below(stop / 50)).min(until));
        assert_eq!(session.status(), Status::Running, "{name}");
        let input = random_input(&session, &switches, &mut rng);
        if session.apply(&input).is_ok() {
            landed += 1;
        }
    }
    assert!(landed > 0, "{name}: no input landed");
    assert_eq!(export_takes_the_steps(&session), Ok(()), "{name}");
}

#[test]
fn stiff_row_exported_mid_run_takes_every_step() {
    let input = without_events(STIFF_ROW);
    an_export_mid_run_takes_every_step_the_session_took("stiff-row", &input, 0x5eed_1011);
}

#[test]
fn rc_switches_exported_mid_run_take_every_step() {
    an_export_mid_run_takes_every_step_the_session_took("rc-switches", RC_SWITCHES, 0x5eed_1013);
}

#[test]
fn rc_switch_exported_mid_run_takes_every_step() {
    let input = example("rc-switch", u64::MAX);
    an_export_mid_run_takes_every_step_the_session_took("rc-switch", &input, 0x5eed_1014);
}

#[test]
fn atmega_button_exported_mid_run_takes_every_step() {
    let input = without_events(&example("atmega-button", AVR_STOP_PS));
    an_export_mid_run_takes_every_step_the_session_took("atmega-button", &input, 0x5eed_1015);
}

#[test]
fn an_input_at_the_accepted_step_lands_1_ps_after_it() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    assert_eq!(
        session.apply(&set("sw1", 0, Contact::Closed)),
        Ok(Applied {
            at: Time::from_ps(1),
            status: Status::Running,
        })
    );
    let State::Running { next, .. } = &session.state else {
        panic!("the session ended");
    };
    let at = next.at;
    session.advance_to(at);
    let State::Running { accepted, now, .. } = &session.state else {
        panic!("the session ended");
    };
    assert_eq!((accepted.at, *now, accepted.reach), (at, at, at));
    assert_eq!(
        session.apply(&set("sw2", 0, Contact::Closed)),
        Ok(Applied {
            at: Time::from_ps(at.as_ps() + 1),
            status: Status::Running,
        })
    );
    ends_as_its_export(session);
}

/// The fixture's first step is tried at 1 ns and rejected down to 40 ps. An input at 41 ps
/// lands at 1 ns, because a batch run with the edge at 41 ps takes another first step.
#[test]
fn an_input_inside_a_rejected_attempt_lands_at_the_reach() {
    let input = without_events(STIFF_ROW);
    let mut session = open_document(&input, Rows::Grid, BUDGET);
    let State::Running { next, .. } = &session.state else {
        panic!("the session ended");
    };
    let first = next.at;
    session.advance_to(first);
    let State::Running { accepted, .. } = &session.state else {
        panic!("the session ended");
    };
    assert_eq!(
        (accepted.at, accepted.reach),
        (Time::from_ps(40), Time::from_ps(1_000))
    );
    assert_eq!(
        session.apply(&set("sw1", 41, Contact::Closed)),
        Ok(Applied {
            at: Time::from_ps(1_000),
            status: Status::Running,
        })
    );
    ends_as_its_export(session);

    let mut json: serde_json::Value = serde_json::from_slice(&input).unwrap();
    json["parts"][5]["events"] = serde_json::json!([{ "t_ps": 41, "state": "closed" }]);
    let early = netlist::parse(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert_ne!(run_steps(&early).time()[1], first);
}

/// Stands on the step the 1 ns attempt after an edge at `edge` shrank to.
/// That attempt's end is `edge + EDGE_STEP`, the plan's first step of a segment.
fn after_a_rejected_edge_step(edge: u64) -> Session {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let at = Time::from_ps(edge);
    assert_eq!(
        session
            .apply(&set("sw1", edge, Contact::Closed))
            .unwrap()
            .at,
        at
    );
    session.advance_to(at);
    let State::Running { next, .. } = &session.state else {
        panic!("the session ended");
    };
    let shortened = next.at.as_ps();
    let end = edge + crate::step::EDGE_STEP;
    assert!(
        edge < shortened && shortened + 1 < end,
        "the 1 ns step was rejected to {shortened} ps, before {end}"
    );
    session.advance_to(Time::from_ps(shortened));
    session
}

/// An input asked for 1 ps after the edge, inside the rejected 1 ns attempt, lands at
/// `edge + EDGE_STEP`.
#[test]
fn an_input_inside_the_first_attempt_after_an_edge_lands_at_its_end() {
    let edge = 5_000_000;
    let end = edge + crate::step::EDGE_STEP;
    let mut session = after_a_rejected_edge_step(edge);
    let applied = session
        .apply(&set("sw2", edge + 1, Contact::Closed))
        .unwrap();
    assert_eq!(
        (applied.at, applied.status),
        (Time::from_ps(end), Status::Running)
    );
    ends_as_its_export(session);
}

/// The same attempt, asked for 1 ps before its end. The landing is still that end.
#[test]
fn an_input_one_picosecond_before_the_attempt_end_lands_on_it() {
    let edge = 5_000_000;
    let end = edge + crate::step::EDGE_STEP;
    let mut session = after_a_rejected_edge_step(edge);
    let applied = session
        .apply(&set("sw2", end - 1, Contact::Closed))
        .unwrap();
    assert_eq!(
        (applied.at, applied.status),
        (Time::from_ps(end), Status::Running)
    );
    ends_as_its_export(session);
}

#[test]
fn a_second_input_on_one_switch_waits_for_its_gap_and_bounce_window() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let at = 5_000_000;
    for (part, then) in [("sw1", at + 2_000), ("sw2", at + 50_000 + 2_000)] {
        assert_eq!(
            session.apply(&set(part, at, Contact::Closed)),
            Ok(Applied {
                at: Time::from_ps(at),
                status: Status::Running,
            })
        );
        assert_eq!(
            session.apply(&set(part, at, Contact::Open)),
            Ok(Applied {
                at: Time::from_ps(then),
                status: Status::Running,
            })
        );
    }
    ends_as_its_export(session);
}

/// Two switches set for one time land on one step, whose restart solve takes both.
#[test]
fn two_inputs_at_one_time_take_one_restart() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let at = Time::from_ps(5_000_000);
    for part in ["sw1", "sw2"] {
        assert_eq!(
            session.apply(&set(part, at.as_ps(), Contact::Closed)),
            Ok(Applied {
                at,
                status: Status::Running,
            })
        );
    }
    session.advance_to(at);
    let State::Running { accepted, next, .. } = &session.state else {
        panic!("the session ended");
    };
    assert_eq!(accepted.at, at);
    assert!(accepted.restart_row.is_some(), "the edges restart");
    assert!(next.at > at);
    ends_as_its_export(session);
}

/// An input asked for later lands there. One asked for `now` after it lands first. `sw2`
/// bounces after its edge.
#[test]
fn an_input_asked_for_later_lands_there() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let later = Time::from_ps(10_000_000);
    assert_eq!(
        session.apply(&set("sw2", later.as_ps(), Contact::Closed)),
        Ok(Applied {
            at: later,
            status: Status::Running,
        })
    );
    session.advance_to(Time::from_ps(3_000_000));
    let now = session.now();
    assert_eq!(
        session.apply(&set("sw1", 0, Contact::Closed)),
        Ok(Applied {
            at: now,
            status: Status::Running,
        })
    );
    let mut restarts = Vec::new();
    while let State::Running { accepted, next, .. } = &session.state {
        if accepted.restart_row.is_some() {
            restarts.push(accepted.at);
        }
        let at = next.at;
        session.advance_to(at);
    }
    assert_eq!(restarts[..2], [now, later], "sw2 bounces after {later:?}");
    ends_as_its_export(session);
}

/// A setting already in force is refused, a pending event included, and changes nothing.
#[test]
fn an_input_that_changes_nothing_is_refused() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    session.advance_to(Time::from_ps(2_500_000));
    let held = |session: &Session| {
        let State::Running { next, .. } = &session.state else {
            panic!("the session ended");
        };
        (next.at, session.export().unwrap())
    };
    let before = held(&session);
    assert_eq!(
        session.apply(&set("sw1", 0, Contact::Open)),
        Err(Rejected::NoChange)
    );
    assert_eq!(held(&session), before);
    session
        .apply(&set("sw1", 4_000_000, Contact::Closed))
        .unwrap();
    let pending = held(&session);
    assert_eq!(
        session.apply(&set("sw1", 9_000_000, Contact::Closed)),
        Err(Rejected::NoChange)
    );
    assert_eq!(held(&session), pending);
    ends_as_its_export(session);
}

#[test]
fn an_input_on_no_switch_is_refused() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let before = session.export();
    assert_eq!(
        session.apply(&set("sw9", 0, Contact::Closed)),
        Err(Rejected::UnknownPart)
    );
    assert_eq!(
        session.apply(&set("r1", 0, Contact::Closed)),
        Err(Rejected::NotSettable)
    );
    assert_eq!(session.export(), before);
}

/// An input at or past `stop` is refused, and so is one whose gap after the last event
/// reaches `stop`.
#[test]
fn an_input_at_or_past_stop_is_refused() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    for at in [stop, stop + 5] {
        assert_eq!(
            session.apply(&set("sw1", at, Contact::Closed)),
            Err(Rejected::PastStop)
        );
    }
    assert_eq!(
        session.apply(&set("sw1", stop - 1, Contact::Closed)),
        Ok(Applied {
            at: Time::from_ps(stop - 1),
            status: Status::Running,
        })
    );
    assert_eq!(
        session.apply(&set("sw1", 0, Contact::Open)),
        Err(Rejected::PastStop)
    );
    ends_as_its_export(session);
}

/// The re-solve hits the step limit. The input has landed, and the session ends at the
/// accepted step it re-solved from, which is before the `now` the call started at.
#[test]
fn a_fault_on_the_re_solve_ends_at_the_accepted_step() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let State::Running { next, .. } = &session.state else {
        panic!("the session ended");
    };
    let next_at = next.at.as_ps();
    assert!(next_at > 1, "{next_at}");
    let between = Time::from_ps(next_at - 1);
    session.advance_to(between);
    let accepted = accepted_at(&session);
    assert!(accepted < session.now());
    assert_eq!(session.stats.landings, 0);
    session.budget.steps = 0;
    let applied = session.apply(&set("sw1", 0, Contact::Closed)).unwrap();
    assert_eq!(applied.status, Status::Faulted);
    assert_eq!(session.status(), Status::Faulted);
    assert_eq!(session.now(), accepted);
    assert!(session.now() < between);
    assert_eq!(
        session.fault(),
        Some(&Diagnostic::SolverStepLimit {
            at: accepted,
            limit: 0,
        })
    );
    let landed = switch(netlist_of(&session), &Name::new("sw1").unwrap())
        .events
        .events()
        .last()
        .unwrap()
        .at;
    assert_eq!(landed, applied.at);
    assert_eq!(
        session.apply(&set("sw2", 0, Contact::Closed)),
        Err(Rejected::Ended)
    );
}

/// A stopped or faulted session takes no input, and its export still runs to its end.
#[test]
fn an_ended_session_refuses_inputs() {
    for budget in [BUDGET, Budget { steps: 3, ..BUDGET }] {
        let mut session = open_document(RC_SWITCHES, Rows::Grid, budget);
        session.apply(&set("sw1", 0, Contact::Closed)).unwrap();
        let status = session.advance_to(transient(netlist_of(&session)).stop);
        assert_ne!(status, Status::Running);
        assert_eq!(
            session.apply(&set("sw2", 0, Contact::Closed)),
            Err(Rejected::Ended)
        );
        ends_as_its_export(session);
    }
}

/// Inputs on a switch whose next edge waits leave it one breakpoint. A second one would walk
/// the switch twice from then on.
#[test]
fn a_switch_with_inputs_waits_on_one_breakpoint() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let at = 2_000_000;
    for to in [Contact::Closed, Contact::Open, Contact::Closed] {
        session.apply(&set("sw1", at, to)).unwrap();
    }
    let mut restarts = Vec::new();
    while let State::Running { accepted, next, .. } = &session.state {
        let mut entries = BTreeMap::new();
        for &std::cmp::Reverse((_, id)) in &session.breakpoints.edges {
            *entries.entry(id).or_insert(0) += 1;
        }
        assert!(
            entries.values().all(|&n| n == 1),
            "at {:?}: {entries:?}",
            accepted.at
        );
        if accepted.restart_row.is_some() {
            restarts.push(accepted.at.as_ps());
        }
        let at = next.at;
        session.advance_to(at);
    }
    assert_eq!(restarts, [at, at + 2_000, at + 4_000]);
    ends_as_its_export(session);
}
