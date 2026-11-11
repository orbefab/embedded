use crate::device::Mismatch;
use crate::netlist::{
    self, Appended, Baud, Chip, Contact, Document, Frame, Model, Netlist, Part, Schedule,
    SerialSend, Value,
};
use crate::passive::{Switch, position_rule};

use std::collections::BTreeMap;

use crate::diagnostic::{Diagnostic, ValueRule};
use crate::finite::Finite;
use crate::logic::Level;
use crate::name::Name;
use crate::name::Ref;

use crate::run_folder::Results;
use crate::serial::SerialByte;
use crate::trace::Trace;

use super::super::*;
use super::fixtures::*;
use super::session::{
    AVR_STOP_PS, Rng, STIFF_ROW, assert_same_folder, batch, example, finish, serial_of_one_call,
};
use crate::session::Budget;
use crate::session::{Applied, Input, Rejected, Serial, SerialCursor, Setting, State, Status};

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

const RC_LEVELS: &[u8] = br#"{
  "version": 0,
  "seed": 4,
  "analysis": { "transient": { "stop_ps": 1000000, "max_step_ps": 5000, "grid_ps": 1000 } },
  "ground": "gnd",
  "nets": ["gnd", "vcc", "vout"],
  "parts": [
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vcc", "n": "gnd" },
      "voltage": {
        "kind": "steps",
        "initial": { "value": 1.0, "unit": "V", "source": "user" },
        "events": []
      }
    },
    {
      "name": "i1",
      "kind": "current_source",
      "pins": { "p": "vout", "n": "gnd" },
      "current": {
        "kind": "steps",
        "initial": { "value": 0.0002, "unit": "A", "source": "user" },
        "events": []
      }
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
      "capacitance": { "value": 1e-9, "unit": "F", "source": "user" },
      "initial": null
    }
  ]
}"#;

const KINDS: &[u8] = br#"{
  "version": 0,
  "seed": 5,
  "analysis": { "transient": { "stop_ps": 100000, "max_step_ps": 1000, "grid_ps": 1000 } },
  "ground": "gnd",
  "nets": ["gnd", "ns", "np", "nv", "nw"],
  "parts": [
    {
      "name": "vs",
      "kind": "voltage_source",
      "pins": { "p": "ns", "n": "gnd" },
      "voltage": {
        "kind": "step",
        "initial": { "value": 0.0, "unit": "V", "source": "user" },
        "final": { "value": 1.0, "unit": "V", "source": "user" },
        "at_ps": 5000
      }
    },
    {
      "name": "vp",
      "kind": "voltage_source",
      "pins": { "p": "np", "n": "gnd" },
      "voltage": {
        "kind": "pulse",
        "initial": { "value": 0.0, "unit": "V", "source": "user" },
        "pulsed": { "value": 1.0, "unit": "V", "source": "user" },
        "delay_ps": 5000,
        "width_ps": 2000,
        "period_ps": 4000
      }
    },
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "nv", "n": "gnd" },
      "voltage": {
        "kind": "steps",
        "initial": { "value": 1.0, "unit": "V", "source": "user" },
        "events": []
      }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "nv", "b": "gnd" },
      "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    },
    {
      "name": "sw1",
      "kind": "switch",
      "pins": { "a": "nv", "b": "gnd" },
      "on_resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
      "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
      "initial": "open",
      "events": [],
      "bounce": null
    },
    {
      "name": "rv1",
      "kind": "potentiometer",
      "pins": { "a": "nv", "wiper": "nw", "b": "gnd" },
      "resistance": { "value": 10000.0, "unit": "ohm", "source": "user" },
      "position": { "value": 0.5, "unit": "1", "source": "user" }
    }
  ]
}"#;

/// 12 ms. The second line is on TXD after the first line, a 1 ms pause and five more frames.
const ANALOG_STOP_PS: u64 = 12_000_000_000;
/// 25 ns of the 200 ns stiff row.
const STIFF_LEVEL_STOP_PS: u64 = 25_000;

struct PartSpec {
    name: &'static str,
    draw: Draw,
}

enum Draw {
    Contact,
    Level { low: f64, high: f64 },
    Serial,
}

const SERIAL_PARTS: &[PartSpec] = &[
    PartSpec {
        name: "v1",
        draw: Draw::Serial,
    },
    PartSpec {
        name: "sw1",
        draw: Draw::Contact,
    },
];

const ANALOG_PARTS: &[PartSpec] = &[PartSpec {
    name: "rv1",
    draw: Draw::Level {
        low: 0.05,
        high: 0.95,
    },
}];

const LEVEL_PARTS: &[PartSpec] = &[
    PartSpec {
        name: "v1",
        draw: Draw::Level {
            low: 0.0,
            high: 2.0,
        },
    },
    PartSpec {
        name: "i1",
        draw: Draw::Level {
            low: 0.0,
            high: 0.001,
        },
    },
];

const STIFF_PARTS: &[PartSpec] = &[
    PartSpec {
        name: "v1",
        draw: Draw::Level {
            low: 0.0,
            high: 2.0,
        },
    },
    PartSpec {
        name: "sw1",
        draw: Draw::Contact,
    },
];

pub(super) fn without_events(input: &[u8]) -> Vec<u8> {
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
    Session::start(
        netlist,
        &transient,
        Vec::new(),
        budget,
        columns,
        Some(rows),
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

fn level(part: &str, at: u64, si: f64) -> Input {
    Input {
        part: Name::new(part).unwrap(),
        at: Time::from_ps(at),
        setting: Setting::Level(Finite::new(si).unwrap()),
    }
}

fn stiff_level_input() -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(&stiff_steps()).unwrap();
    json["analysis"]["transient"]["stop_ps"] = STIFF_LEVEL_STOP_PS.into();
    serde_json::to_vec(&json).unwrap()
}

fn stiff_steps() -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(STIFF_ROW).unwrap();
    for part in json["parts"].as_array_mut().unwrap() {
        if part["name"] == "v1" {
            let voltage = part["voltage"].clone();
            part["voltage"] = serde_json::json!({
                "kind": "steps",
                "initial": voltage,
                "events": []
            });
        }
    }
    serde_json::to_vec(&json).unwrap()
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
    let Setting::Contact(to) = &input.setting else {
        panic!("a switch input");
    };
    let after = Time::from_ps(accepted.at.as_ps() + 1);
    let floor = input.at.max(*now).max(after).max(accepted.reach);
    if *to == last_contact(switch) {
        return Err(Rejected::NoChange);
    }
    match switch.earliest(floor) {
        Some(at) if at < transient(netlist_of(session)).stop => Ok(at),
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

fn random_at(session: &Session, rng: &mut Rng) -> Time {
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
    Time::from_ps(at)
}

fn level_now(session: &Session, name: &str) -> f64 {
    let netlist = netlist_of(session);
    let id = netlist.part_id(&Name::new(name).unwrap()).unwrap();
    let at = session.now();
    let Some(level) = netlist.part(id).model.level() else {
        panic!("{name} takes no level");
    };
    level.schedule.after(at).si.get()
}

fn draw_level(rng: &mut Rng, low: f64, high: f64, in_force: f64) -> f64 {
    if rng.below(4) == 0 {
        return in_force;
    }
    let mut level = in_force;
    for _ in 0..8 {
        let unit = (rng.below(1_000_000) + 1) as f64 / 1_000_001.0;
        level = low + (high - low) * unit;
        if level != in_force {
            return level;
        }
    }
    level
}

fn random_spec(session: &Session, specs: &[PartSpec], rng: &mut Rng) -> Input {
    let spec = &specs[rng.below(specs.len() as u64) as usize];
    let at = random_at(session, rng);
    let setting = match spec.draw {
        Draw::Contact => {
            let name = Name::new(spec.name).unwrap();
            let last = last_contact(switch(netlist_of(session), &name));
            let to = if rng.below(4) == 0 {
                last
            } else {
                last.flipped()
            };
            Setting::Contact(to)
        }
        Draw::Level { low, high } => {
            let si = draw_level(rng, low, high, level_now(session, spec.name));
            Setting::Level(Finite::new(si).unwrap())
        }
        Draw::Serial => {
            let bauds = [500_000_000, 333_333_333, 100_000_000, 499_999_999];
            let baud = bauds[rng.below(4) as usize];
            let n = rng.below(4);
            let bytes = (0..n)
                .map(|_| u8::try_from(rng.below(256)).unwrap())
                .collect();
            Setting::Serial(Serial {
                baud: Baud::new(baud).unwrap(),
                frame: Frame::EightN1,
                bytes,
            })
        }
    };
    Input {
        part: Name::new(spec.name).unwrap(),
        at,
        setting,
    }
}

fn ruling(session: &Session, input: &Input) -> Result<Time, Rejected> {
    let State::Running { accepted, now, .. } = &session.state else {
        return Err(Rejected::Ended);
    };
    let netlist = netlist_of(session);
    let Some(id) = netlist.part_id(&input.part) else {
        return Err(Rejected::UnknownPart);
    };
    let after = Time::from_ps(accepted.at.as_ps() + 1);
    let from = input.at.max(*now).max(after).max(accepted.reach);
    let stop = transient(netlist).stop;
    let model = &netlist.part(id).model;
    match (model, &input.setting) {
        (Model::Switch(switch), Setting::Contact(to)) => {
            if *switch.schedule.after_last() == *to {
                Err(Rejected::NoChange)
            } else {
                landing(switch.earliest(from), stop)
            }
        }
        (Model::Switch(_), Setting::Level(_)) => Err(Rejected::WrongSetting),
        (Model::Potentiometer(pot), Setting::Level(level)) => {
            if let Some(rule) = position_rule(level.get(), pot.resistance.si.get()).first() {
                return Err(Rejected::Value(rule));
            }
            level_ruling(&pot.position, *level, from, stop)
        }
        (Model::Potentiometer(_), Setting::Contact(_)) => Err(Rejected::WrongSetting),
        (_, Setting::Event(event)) => {
            match model.device().filter(|(row, _)| !row.events.is_empty()) {
                Some((row, events)) => {
                    let spec = row.events.iter().find(|spec| spec.name == event.name);
                    match spec.map(|spec| spec.declared(&event.fields)) {
                        None | Some(Err(Mismatch::Shape)) => Err(Rejected::WrongSetting),
                        Some(Err(Mismatch::Span)) => Err(Rejected::Value(ValueRule::Span)),
                        Some(Ok(_)) => landing(events.earliest(from), stop),
                    }
                }
                None => Err(
                    if matches!(model, Model::Switch(_) | Model::Potentiometer(_))
                        || model.level().is_some()
                        || model.serial().is_some()
                    {
                        Rejected::WrongSetting
                    } else {
                        Rejected::NotSettable
                    },
                ),
            }
        }
        (_, Setting::Serial(send)) => serial_ruling(model, send, from, stop),
        (_, Setting::Contact(_)) => {
            if model.serial().is_some() || model.level().is_some() {
                Err(Rejected::WrongSetting)
            } else {
                Err(Rejected::NotSettable)
            }
        }
        (_, Setting::Level(level)) => {
            if model.serial().is_some() {
                return Err(Rejected::WrongSetting);
            }
            let Some(field) = model.level() else {
                return Err(Rejected::NotSettable);
            };
            level_ruling(field.schedule, *level, from, stop)
        }
    }
}

fn serial_ruling(model: &Model, send: &Serial, from: Time, stop: Time) -> Result<Time, Rejected> {
    let Some(field) = model.serial() else {
        return Err(
            if matches!(model, Model::Switch(_) | Model::Potentiometer(_))
                || model.level().is_some()
            {
                Rejected::WrongSetting
            } else {
                Rejected::NotSettable
            },
        );
    };
    if send.bytes.is_empty() {
        return Err(Rejected::NoChange);
    }
    let at = match field.waveform.events().last() {
        Some(event) => from.max(event.end()),
        None => from,
    };
    let wire = SerialSend {
        baud: send.baud,
        frame: send.frame,
        bytes: send.bytes.clone(),
    };
    match wire.end(at) {
        Some(_) if at < stop => Ok(at),
        _ => Err(Rejected::PastStop),
    }
}

fn level_ruling(
    schedule: &Schedule<Value>,
    level: Finite,
    from: Time,
    stop: Time,
) -> Result<Time, Rejected> {
    if schedule.after_last().si == level {
        Err(Rejected::NoChange)
    } else {
        landing(schedule.earliest(from, 0), stop)
    }
}

fn landing(at: Option<Time>, stop: Time) -> Result<Time, Rejected> {
    match at {
        Some(at) if at < stop => Ok(at),
        _ => Err(Rejected::PastStop),
    }
}

fn apply_checked(
    name: &str,
    session: &mut Session,
    input: &Input,
    want: Result<Time, Rejected>,
) -> Option<Time> {
    let State::Running { accepted, now, .. } = &session.state else {
        panic!("{name} ended");
    };
    let (accepted_at, now_at, reach) = (accepted.at, *now, accepted.reach);
    let before = session.export();
    let got = session.apply(input);
    match (&got, &want) {
        (Ok(got), Ok(want_at)) => {
            assert_eq!(got.at, *want_at, "{name} {input:?}");
            assert!(got.at >= input.at.max(now_at), "{name} {input:?}");
            assert!(got.at > accepted_at, "{name} {input:?}");
            assert!(got.at >= reach, "{name} {input:?}");
            assert_eq!(got.status, session.status(), "{name} {input:?}");
        }
        (Err(got_err), Err(want_err)) => {
            assert_eq!(got_err, want_err, "{name} {input:?}");
            if *got_err == Rejected::NoChange {
                assert_eq!(session.export(), before, "{name} {input:?}");
            }
        }
        _ => panic!("{name} {input:?}: {got:?} vs {want:?}"),
    }
    got.ok().map(|applied| applied.at)
}

/// A serial send that starts at the previous event's end, after the time it asked for.
fn waited_for_frames(landings: &Netlist, input: &Input, at: Time) -> bool {
    let Setting::Serial(_) = &input.setting else {
        return false;
    };
    let id = landings.part_id(&input.part).unwrap();
    let Some(last) = landings
        .part(id)
        .model
        .serial()
        .unwrap()
        .waveform
        .events()
        .last()
    else {
        return false;
    };
    last.end() == at && at > input.at
}

fn draws_serial(specs: Option<&[PartSpec]>) -> bool {
    specs.is_some_and(|specs| specs.iter().any(|spec| matches!(&spec.draw, Draw::Serial)))
}

/// Appends `input` to `landings` at `at`, where the session said it landed.
fn land(landings: &mut Netlist, input: &Input, at: Time) {
    let id = landings.part_id(&input.part).unwrap();
    let to = match &input.setting {
        Setting::Contact(to) => Appended::Contact(*to),
        Setting::Level(level) => Appended::Level(*level),
        Setting::Serial(send) => Appended::Serial(SerialSend {
            baud: send.baud,
            frame: send.frame,
            bytes: send.bytes.clone(),
        }),
        Setting::Event(event) => {
            let (row, _) = landings.part(id).model.device().unwrap();
            let spec = row.events.iter().find(|spec| spec.name == event.name);
            Appended::Event(spec.unwrap().declared(&event.fields).unwrap())
        }
    };
    landings.append(id, at, to);
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
    assert_same_folder(&export, &exported, &finish(session), &batch);
}

/// Whether `now` is before the accepted step's reach, where an input at `now` would cut an
/// attempt the session made.
fn inside_the_reach(session: &Session) -> bool {
    matches!(&session.state, State::Running { accepted, now, .. } if accepted.reach > *now)
}

fn serial(session: &Session) -> Vec<SerialByte> {
    session
        .serial_since(SerialCursor::default())
        .expect("cursor still held")
        .0
        .to_vec()
}

/// `input` advanced in chunks ended as `input_in_chunks_writes_the_batch_folder` ends them,
/// with a seeded input after one chunk in three and after each chunk that ends inside the
/// accepted step's reach. Each input lands where the landing rule says, and the export holds
/// each landing at its time. A batch run of the export writes the folder the session recorded,
/// and each probe at a row time after the last input before it is that row, bit for bit. Past
/// half the run, the session's serial bytes are the first bytes of one call on its export. Once
/// it has ended they are all of them. Returns the batch and those bytes.
fn inputs_in_chunks_write_the_export_folder(
    name: &str,
    input: &[u8],
    budget: Budget,
    seed: u64,
    specs: Option<&[PartSpec]>,
) -> (Results, Vec<SerialByte>) {
    let mut session = open_document(input, Rows::Grid, budget);
    let transient = transient(netlist_of(&session));
    let switches = switches(netlist_of(&session));
    let mut rng = Rng(seed);
    let mut probes = Vec::new();
    let mut landings = netlist_of(&session).clone();
    let mut landed = 0;
    let mut serial_landed = 0;
    let mut serial_queued = 0;
    let mut halfway = false;
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
            let input = match specs {
                Some(specs) => random_spec(&session, specs, &mut rng),
                None => random_input(&session, &switches, &mut rng),
            };
            let accepted = accepted_at(&session);
            let landing = match specs {
                Some(_) => {
                    let want = ruling(&session, &input);
                    apply_checked(name, &mut session, &input, want)
                }
                None => {
                    let want = expected(&session, &input);
                    let got = session.apply(&input);
                    match (&got, &want) {
                        (Ok(got), Ok(want)) => {
                            assert_eq!(got.at, *want, "{name} {input:?}");
                            assert_eq!(got.status, session.status(), "{name} {input:?}");
                        }
                        (Err(got), Err(want)) => assert_eq!(got, want, "{name} {input:?}"),
                        _ => panic!("{name} {input:?}: {got:?} vs {want:?}"),
                    }
                    got.ok().map(|applied| applied.at)
                }
            };
            if let Some(at) = landing {
                if matches!(input.setting, Setting::Serial(_)) {
                    serial_landed += 1;
                    if waited_for_frames(&landings, &input, at) {
                        serial_queued += 1;
                    }
                }
                land(&mut landings, &input, at);
                landed += 1;
                probes.retain(|&(at, _)| at <= accepted);
            }
        }
        let probe = session.probe();
        probes.push((probe.at(), probe.values().to_vec()));
        if !halfway
            && session.status() == Status::Running
            && session.now().as_ps() > transient.stop.as_ps() / 2
        {
            halfway = true;
            let exported = netlist::parse(&session.export().unwrap()).unwrap();
            let whole = serial_of_one_call(&exported, budget);
            let now = session.now();
            assert!(whole.starts_with(&serial(&session)), "{name} at {now:?}");
        }
    }
    assert!(landed > 0, "{name}: no input landed");
    if draws_serial(specs) {
        assert!(serial_landed > 0, "{name}: no serial send landed");
        assert!(
            serial_queued > 0,
            "{name}: no serial send waited for frames still on the wire"
        );
    }
    assert!(halfway || session.status() == Status::Faulted, "{name}");
    let export = session.export().unwrap();
    let exported = netlist::parse(&export).unwrap();
    assert_eq!(parts(&exported), parts(netlist_of(&session)));
    assert_eq!(parts(&exported), parts(&landings), "{name}: each landing");
    let bytes = serial(&session);
    assert_eq!(bytes, serial_of_one_call(&exported, budget), "{name}");
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
    assert_same_folder(&export, &exported, &finish(session), &batch);
    (batch, bytes)
}

#[test]
fn atmega_button_with_inputs_writes_the_export_folder() {
    let input = without_events(&example("atmega-button", AVR_STOP_PS));
    inputs_in_chunks_write_the_export_folder("atmega-button", &input, BUDGET, 0x5eed_1001, None);
}

#[test]
fn button_led_with_inputs_writes_the_export_folder() {
    let input = without_events(&example("button-led", u64::MAX));
    inputs_in_chunks_write_the_export_folder("button-led", &input, BUDGET, 0x5eed_1002, None);
}

/// `rc-switch` keeps its own events, so an input lands after the last of them.
#[test]
fn rc_switch_with_inputs_writes_the_export_folder() {
    let input = example("rc-switch", u64::MAX);
    inputs_in_chunks_write_the_export_folder("rc-switch", &input, BUDGET, 0x5eed_1003, None);
}

#[test]
fn rc_switches_with_inputs_write_the_export_folder() {
    for seed in [0x5eed_1004, 0x5eed_1005] {
        inputs_in_chunks_write_the_export_folder("rc-switches", RC_SWITCHES, BUDGET, seed, None);
    }
}

/// Without its event, the stiff fixture's first attempts are rejected from 1 ns down, so an
/// input early in the run lands at the reach.
#[test]
fn stiff_row_with_inputs_writes_the_export_folder() {
    let input = without_events(STIFF_ROW);
    inputs_in_chunks_write_the_export_folder("stiff-row", &input, BUDGET, 0x5eed_1006, None);
}

#[test]
fn stiff_row_with_its_event_and_inputs_writes_the_export_folder() {
    inputs_in_chunks_write_the_export_folder("stiff-row", STIFF_ROW, BUDGET, 0x5eed_1008, None);
}

#[test]
fn a_session_with_inputs_that_reaches_the_step_limit_writes_the_export_folder() {
    let budget = Budget {
        steps: 157,
        ..BUDGET
    };
    let input = example("rc-switch", u64::MAX);
    inputs_in_chunks_write_the_export_folder("rc-switch", &input, budget, 0x5eed_1009, None);
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
/// run, then exported there. The export holds each landing at its time.
fn an_export_mid_run_takes_every_step_the_session_took(
    name: &str,
    input: &[u8],
    seed: u64,
    specs: Option<&[PartSpec]>,
) {
    let mut session = open_document(input, Rows::Steps, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    let switches = switches(netlist_of(&session));
    let mut rng = Rng(seed);
    let until = Time::from_ps(stop / 4 + rng.below(stop / 2));
    let mut landings = netlist_of(&session).clone();
    let mut landed = 0;
    let mut serial_landed = 0;
    let mut serial_queued = 0;
    while session.now() < until {
        let now = session.now().as_ps();
        session.advance_to(Time::from_ps(now + 1 + rng.below(stop / 50)).min(until));
        assert_eq!(session.status(), Status::Running, "{name}");
        let input = match specs {
            Some(specs) => random_spec(&session, specs, &mut rng),
            None => random_input(&session, &switches, &mut rng),
        };
        let landing = match specs {
            Some(_) => {
                let want = ruling(&session, &input);
                apply_checked(name, &mut session, &input, want)
            }
            None => session.apply(&input).ok().map(|applied| applied.at),
        };
        if let Some(at) = landing {
            if matches!(input.setting, Setting::Serial(_)) {
                serial_landed += 1;
                if waited_for_frames(&landings, &input, at) {
                    serial_queued += 1;
                }
            }
            land(&mut landings, &input, at);
            landed += 1;
        }
    }
    assert!(landed > 0, "{name}: no input landed");
    if draws_serial(specs) {
        assert!(serial_landed > 0, "{name}: no serial send landed");
        assert!(
            serial_queued > 0,
            "{name}: no serial send waited for frames still on the wire"
        );
    }
    let exported = netlist::parse(&session.export().unwrap()).unwrap();
    assert_eq!(parts(&exported), parts(&landings), "{name}: each landing");
    assert_eq!(export_takes_the_steps(&session), Ok(()), "{name}");
}

#[test]
fn stiff_row_exported_mid_run_takes_every_step() {
    let input = without_events(STIFF_ROW);
    an_export_mid_run_takes_every_step_the_session_took("stiff-row", &input, 0x5eed_1011, None);
}

#[test]
fn rc_switches_exported_mid_run_take_every_step() {
    an_export_mid_run_takes_every_step_the_session_took(
        "rc-switches",
        RC_SWITCHES,
        0x5eed_1013,
        None,
    );
}

#[test]
fn rc_switch_exported_mid_run_takes_every_step() {
    let input = example("rc-switch", u64::MAX);
    an_export_mid_run_takes_every_step_the_session_took("rc-switch", &input, 0x5eed_1014, None);
}

#[test]
fn atmega_button_exported_mid_run_takes_every_step() {
    let input = without_events(&example("atmega-button", AVR_STOP_PS));
    an_export_mid_run_takes_every_step_the_session_took("atmega-button", &input, 0x5eed_1015, None);
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
    let before = session.export();
    assert_eq!(
        session.apply(&set("sw1", stop, Contact::Open)),
        Err(Rejected::NoChange)
    );
    assert_eq!(session.export(), before);
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

#[test]
fn atmega_analog_serial_with_level_inputs_writes_the_export_folder() {
    let input = example("atmega-analog-serial", ANALOG_STOP_PS);
    let (batch, bytes) = inputs_in_chunks_write_the_export_folder(
        "atmega-analog-serial",
        &input,
        BUDGET,
        0x5eed_2001,
        Some(ANALOG_PARTS),
    );
    let netlist = netlist::parse(&input).unwrap();
    assert_a_sample_read_a_moved_level(&netlist, batch.trace().unwrap());
    assert_eq!(bytes.iter().filter(|byte| byte.byte == b'\n').count(), 2);
}

#[test]
fn rc_levels_with_inputs_write_the_export_folder() {
    inputs_in_chunks_write_the_export_folder(
        "rc-levels",
        RC_LEVELS,
        BUDGET,
        0x5eed_2002,
        Some(LEVEL_PARTS),
    );
}

#[test]
fn rc_levels_exported_mid_run_take_every_step() {
    an_export_mid_run_takes_every_step_the_session_took(
        "rc-levels",
        RC_LEVELS,
        0x5eed_2003,
        Some(LEVEL_PARTS),
    );
}

#[test]
fn stiff_row_with_level_inputs_writes_the_export_folder() {
    let input = stiff_level_input();
    inputs_in_chunks_write_the_export_folder(
        "stiff-row",
        &input,
        BUDGET,
        0x5eed_2004,
        Some(STIFF_PARTS),
    );
}

#[test]
fn stiff_row_with_level_inputs_exported_mid_run_takes_every_step() {
    let input = stiff_steps();
    an_export_mid_run_takes_every_step_the_session_took(
        "stiff-row",
        &input,
        0x5eed_2005,
        Some(STIFF_PARTS),
    );
}

/// `v1` sends 0 to 3 bytes at a few bauds, and `sw1` takes a contact, on the stiff row.
/// A send asked for while frames are still on the wire starts at the previous event's end.
#[test]
fn stiff_serial_with_inputs_writes_the_export_folder() {
    let input = stiff_serial();
    for seed in [0x5eed_9000, 0x5eed_9001] {
        inputs_in_chunks_write_the_export_folder(
            "stiff-serial",
            &input,
            BUDGET,
            seed,
            Some(SERIAL_PARTS),
        );
    }
}

/// The same draws, exported while the session still runs. The export holds each landing,
/// and a batch run of it takes every step the session took.
#[test]
fn stiff_serial_exported_mid_run_takes_every_step() {
    let input = stiff_serial();
    for seed in [0x5eed_9100, 0x5eed_9101] {
        an_export_mid_run_takes_every_step_the_session_took(
            "stiff-serial",
            &input,
            seed,
            Some(SERIAL_PARTS),
        );
    }
}

#[test]
fn a_level_inside_a_rejected_attempt_lands_at_the_reach() {
    let mut session = open_document(&stiff_steps(), Rows::Grid, BUDGET);
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
        session.apply(&level("v1", 41, 1.5)),
        Ok(Applied {
            at: Time::from_ps(1_000),
            status: Status::Running,
        })
    );
    session.advance_to(Time::from_ps(1_000));
    let State::Running { accepted, .. } = &session.state else {
        panic!("the session ended");
    };
    assert_eq!(accepted.at, Time::from_ps(1_000));
    assert!(accepted.restart_row.is_some(), "the level restarts");
    ends_as_its_export(session);
}

#[test]
fn a_second_level_on_one_part_waits_2_ns() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    assert_eq!(
        session.apply(&level("v1", 0, 1.5)),
        Ok(Applied {
            at: Time::from_ps(1),
            status: Status::Running,
        })
    );
    assert_eq!(
        session.apply(&level("v1", 0, 0.5)),
        Ok(Applied {
            at: Time::from_ps(1 + 2_000),
            status: Status::Running,
        })
    );
    session.advance_to(Time::from_ps(1));
    let State::Running { accepted, next, .. } = &session.state else {
        panic!("the session ended");
    };
    assert_eq!(accepted.at, Time::from_ps(1));
    assert!(accepted.restart_row.is_some(), "the level restarts");
    assert!(next.at > accepted.at);
    ends_as_its_export(session);
}

#[test]
fn a_level_and_a_contact_at_one_time_take_one_restart() {
    let mut session = open_document(RC_SWITCHES, Rows::Grid, BUDGET);
    let at = Time::from_ps(5_000_000);
    for input in [
        level("v1", at.as_ps(), 1.5),
        set("sw1", at.as_ps(), Contact::Closed),
    ] {
        assert_eq!(
            session.apply(&input),
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

#[test]
fn a_level_already_in_force_is_refused() {
    let mut session = open_document(RC_LEVELS, Rows::Grid, BUDGET);
    let before = session.export();
    assert_eq!(session.apply(&level("v1", 0, 1.0)), Err(Rejected::NoChange));
    assert_eq!(session.export(), before);
    session.apply(&level("v1", 4_000, 1.5)).unwrap();
    let pending = session.export();
    assert_ne!(pending, before);
    assert_eq!(
        session.apply(&level("v1", 9_000, 1.5)),
        Err(Rejected::NoChange)
    );
    assert_eq!(session.export(), pending);
    ends_as_its_export(session);
}

#[test]
fn a_contact_on_a_level_part_and_a_level_on_a_switch_are_refused() {
    let mut session = open_document(KINDS, Rows::Grid, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    let before = session.export();
    for at in [0, stop] {
        assert_eq!(
            session.apply(&set("v1", at, Contact::Closed)),
            Err(Rejected::WrongSetting)
        );
        assert_eq!(
            session.apply(&set("rv1", at, Contact::Closed)),
            Err(Rejected::WrongSetting)
        );
        assert_eq!(
            session.apply(&level("sw1", at, 0.5)),
            Err(Rejected::WrongSetting)
        );
    }
    assert_eq!(session.export(), before);
}

#[test]
fn a_position_of_0_1_or_above_1_is_refused() {
    let mut session = open_document(KINDS, Rows::Grid, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    let before = session.export();
    for position in [0.0, 1.0, 1.5] {
        for at in [0, stop] {
            assert_eq!(
                session.apply(&level("rv1", at, position)),
                Err(Rejected::Value(ValueRule::Fraction)),
                "{position} at {at}"
            );
        }
    }
    assert_eq!(
        session.apply(&level("rv1", 0, 1e-320)),
        Err(Rejected::Value(ValueRule::FiniteReciprocal))
    );
    assert_eq!(session.export(), before);
}

#[test]
fn a_transistor_takes_no_input() {
    let path = format!(
        "{}/tests/circuits/npn-led/netlist.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    json["analysis"] = serde_json::json!({
        "transient": { "stop_ps": 1000, "max_step_ps": 1000, "grid_ps": 1000 }
    });
    let mut session = open_document(&serde_json::to_vec(&json).unwrap(), Rows::Grid, BUDGET);
    let before = session.export();
    for input in [
        level("q1", 0, 1.0),
        set("q1", 0, Contact::Closed),
        send("q1", 0, 9600, &[0x31]),
    ] {
        assert_eq!(
            session.apply(&input),
            Err(Rejected::NotSettable),
            "{input:?}"
        );
    }
    assert_eq!(session.export(), before);
}

#[test]
fn an_nmos_takes_no_input() {
    let path = format!(
        "{}/tests/circuits/nmos-led/netlist.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    json["analysis"] = serde_json::json!({
        "transient": { "stop_ps": 1000, "max_step_ps": 1000, "grid_ps": 1000 }
    });
    let mut session = open_document(&serde_json::to_vec(&json).unwrap(), Rows::Grid, BUDGET);
    let before = session.export();
    for input in [
        level("m1", 0, 1.0),
        set("m1", 0, Contact::Closed),
        send("m1", 0, 9600, &[0x31]),
    ] {
        assert_eq!(
            session.apply(&input),
            Err(Rejected::NotSettable),
            "{input:?}"
        );
    }
    assert_eq!(session.export(), before);
}

#[test]
fn an_inductor_takes_no_input() {
    let mut session = open_document(&example("rl-step", 1_000), Rows::Grid, BUDGET);
    let before = session.export();
    for input in [
        level("l1", 0, 1.0),
        set("l1", 0, Contact::Closed),
        send("l1", 0, 9600, &[0x31]),
    ] {
        assert_eq!(
            session.apply(&input),
            Err(Rejected::NotSettable),
            "{input:?}"
        );
    }
    assert_eq!(session.export(), before);
}

#[test]
fn a_relay_takes_no_input() {
    let mut session = open_document(&example("relay-switch", 1_000), Rows::Grid, BUDGET);
    let before = session.export();
    for input in [
        level("k1", 0, 1.0),
        set("k1", 0, Contact::Closed),
        send("k1", 0, 9600, &[0x31]),
    ] {
        assert_eq!(
            session.apply(&input),
            Err(Rejected::NotSettable),
            "{input:?}"
        );
    }
    assert_eq!(session.export(), before);
}

#[test]
fn a_step_a_pulse_and_a_resistor_take_no_setting() {
    let mut session = open_document(KINDS, Rows::Grid, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    let before = session.export();
    for part in ["vs", "vp", "r1"] {
        for at in [0, stop] {
            assert_eq!(
                session.apply(&level(part, at, 1.5)),
                Err(Rejected::NotSettable),
                "{part} at {at}"
            );
            assert_eq!(
                session.apply(&set(part, at, Contact::Closed)),
                Err(Rejected::NotSettable),
                "{part} at {at}"
            );
        }
    }
    assert_eq!(session.export(), before);
}

#[test]
fn a_level_at_or_past_stop_is_refused() {
    let mut session = open_document(RC_LEVELS, Rows::Grid, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    for at in [stop, stop + 5] {
        assert_eq!(
            session.apply(&level("v1", at, 1.5)),
            Err(Rejected::PastStop)
        );
    }
    assert_eq!(
        session.apply(&level("v1", stop - 1, 1.5)),
        Ok(Applied {
            at: Time::from_ps(stop - 1),
            status: Status::Running,
        })
    );
    assert_eq!(session.apply(&level("v1", 0, 0.5)), Err(Rejected::PastStop));
    ends_as_its_export(session);
}

#[test]
fn a_level_already_in_force_at_stop_is_no_change() {
    let mut session = open_document(RC_LEVELS, Rows::Grid, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    let before = session.export();
    assert_eq!(
        session.apply(&level("v1", stop, 1.0)),
        Err(Rejected::NoChange)
    );
    assert_eq!(session.export(), before);
    assert_eq!(
        session.apply(&level("v1", stop - 1, 1.5)),
        Ok(Applied {
            at: Time::from_ps(stop - 1),
            status: Status::Running,
        })
    );
    let pending = session.export();
    assert_eq!(
        session.apply(&level("v1", stop, 1.5)),
        Err(Rejected::NoChange)
    );
    assert_eq!(session.export(), pending);
    ends_as_its_export(session);
}

#[test]
fn two_levels_at_one_time_take_one_restart() {
    let mut session = open_document(RC_LEVELS, Rows::Grid, BUDGET);
    let at = Time::from_ps(5_000);
    for input in [
        level("v1", at.as_ps(), 1.5),
        level("i1", at.as_ps(), 0.0004),
    ] {
        assert_eq!(
            session.apply(&input),
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
    assert!(accepted.restart_row.is_some(), "the levels restart");
    assert!(next.at > at);
    ends_as_its_export(session);
}

/// `1e-30 * 1e-300` underflows to 0, so the leg from `a` is not positive.
#[test]
fn a_position_whose_leg_underflows_is_refused() {
    let input = br#"{
      "version": 0,
      "seed": 6,
      "analysis": { "transient": { "stop_ps": 1000, "max_step_ps": 1000, "grid_ps": 1000 } },
      "ground": "gnd",
      "nets": ["gnd", "n"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "n", "n": "gnd" },
          "voltage": { "value": 0.0, "unit": "V", "source": "user" }
        },
        {
          "name": "rv1",
          "kind": "potentiometer",
          "pins": { "a": "n", "wiper": "gnd", "b": "gnd" },
          "resistance": { "value": 1e-300, "unit": "ohm", "source": "user" },
          "position": { "value": 0.5, "unit": "1", "source": "user" }
        }
      ]
    }"#;
    let mut session = open_document(input, Rows::Grid, BUDGET);
    let before = session.export();
    assert_eq!(
        session.apply(&level("rv1", 0, 1e-30)),
        Err(Rejected::Value(ValueRule::Positive))
    );
    assert_eq!(session.export(), before);
}

/// `analog_serial.S` leaves `U2X0` clear and writes `UBRR0` = 103. A bit is 16 baud ticks,
/// and a tick is `UBRR0 + 1` cycles.
fn bit_ps(netlist: &Netlist) -> u64 {
    let hz = netlist
        .parts()
        .find_map(|(_, part)| match &part.model {
            Model::Chip(Chip::Atmega328p(mcu)) => Some(mcu.clock_hz.value),
            _ => None,
        })
        .expect("an ATmega328P");
    assert_eq!(1_000_000_000_000 % hz, 0);
    16 * (103 + 1) * (1_000_000_000_000 / hz)
}

fn assert_a_sample_read_a_moved_level(netlist: &Netlist, trace: &Trace) {
    let resting = adc_code(
        column_at(trace, "#net.pc0", 0),
        column_at(trace, "#net.vcc", 0),
    );
    let until = trace.time().last().expect("a row").as_ps();
    let lines = decode_lines(&pd1_edges(trace), bit_ps(netlist), until);
    assert!(
        lines.len() >= 2,
        "the run printed {} adc line(s), want two conversions",
        lines.len()
    );
    assert_eq!(
        lines[0], resting,
        "the first conversion samples before the first move, want the resting code {resting}, got {lines:?}"
    );
    assert!(
        lines.iter().any(|&code| code != resting),
        "every sample read the resting code {resting}: {lines:?}"
    );
}

fn column_at(trace: &Trace, key: &str, row: usize) -> f64 {
    let want: Ref = key.parse().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|column| *column == want)
        .unwrap_or_else(|| panic!("no {key}"));
    trace.column(index)[row]
}

fn pd1_edges(trace: &Trace) -> Vec<(u64, bool)> {
    let signal: Ref = "#u1.logic.PD1".parse().unwrap();
    let logic = trace.logic().expect("logic");
    let index = logic
        .signals()
        .iter()
        .position(|s| *s == signal)
        .expect("PD1");
    logic
        .changes()
        .iter()
        .filter(|change| change.signal == index)
        .map(|change| (change.at.as_ps(), change.level == Level::High))
        .collect()
}

/// §24.7. `floor(Vin * 1024 / Vref)`.
fn adc_code(vin: f64, vref: f64) -> u16 {
    let code = (vin * 1024.0 / vref).floor();
    assert!(
        (0.0..1024.0).contains(&code),
        "ADC code {code} is outside 0..1024"
    );
    code as u16
}

/// 8N1 frames whose stop bit is inside the trace. Each later fall is a start bit.
fn decode_lines(edges: &[(u64, bool)], bit_ps: u64, until: u64) -> Vec<u16> {
    let text: String = decode_txd(edges, bit_ps, until)
        .into_iter()
        .map(|(_, byte)| byte as char)
        .collect();
    text.split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .map(|line| {
            let line = line.trim_end_matches(['\r', '\n']);
            line.parse()
                .unwrap_or_else(|err| panic!("{line:?} is not an adc code: {err}"))
        })
        .collect()
}

fn decode_txd(edges: &[(u64, bool)], bit_ps: u64, until: u64) -> Vec<(u64, u8)> {
    let (at, level) = edges[0];
    assert!(level, "TXD rises to idle at {at} ps");
    let mut i = 1;
    let mut high = true;
    let mut frames = Vec::new();
    while i < edges.len() {
        let (start, level) = edges[i];
        if start + 9 * bit_ps > until {
            break;
        }
        assert!(!level && high, "start bit at {start} ps");
        i += 1;
        high = false;
        let mut bits = [false; 9];
        for (k, bit) in bits.iter_mut().enumerate() {
            let boundary = start + (k as u64 + 1) * bit_ps;
            if i < edges.len() && edges[i].0 == boundary {
                high = edges[i].1;
                i += 1;
            } else if i < edges.len() && edges[i].0 < boundary {
                panic!(
                    "edge at {} ps is off the bit grid of the frame at {start} ps",
                    edges[i].0
                );
            }
            *bit = high;
        }
        assert!(bits[8], "stop bit of the frame at {start} ps");
        let mut byte = 0u8;
        for (bit, &level) in bits[..8].iter().enumerate() {
            if level {
                byte |= 1 << bit;
            }
        }
        frames.push((start, byte));
        high = true;
    }
    frames
}

/// Bit boundary `i` is `start + floor(i * 10^12 / baud)`, the same rule a serial event stores.
fn frame_end(start: u64, baud: u64, nbytes: u64) -> u64 {
    let bits = nbytes * 10;
    start + u64::try_from(u128::from(bits) * 1_000_000_000_000 / u128::from(baud)).unwrap()
}

fn send(part: &str, at: u64, baud: u32, bytes: &[u8]) -> Input {
    Input {
        part: Name::new(part).unwrap(),
        at: Time::from_ps(at),
        setting: Setting::Serial(Serial {
            baud: Baud::new(baud).unwrap(),
            frame: Frame::EightN1,
            bytes: bytes.to_vec(),
        }),
    }
}

fn v_serial_bytes() -> Vec<u8> {
    example("v-serial", u64::MAX)
}

fn with_stop(input: &[u8], stop_ps: u64) -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(input).unwrap();
    json["analysis"]["transient"]["stop_ps"] = stop_ps.into();
    serde_json::to_vec(&json).unwrap()
}

fn without_serial_events(input: &[u8]) -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(input).unwrap();
    for part in json["parts"].as_array_mut().unwrap() {
        if part["name"] == "v1" {
            part["voltage"]["events"] = serde_json::json!([]);
        }
    }
    serde_json::to_vec(&json).unwrap()
}

fn stiff_serial() -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(STIFF_ROW).unwrap();
    for part in json["parts"].as_array_mut().unwrap() {
        if part["name"] == "v1" {
            part["voltage"] = serde_json::json!({
                "kind": "serial",
                "mark": { "value": 1.0, "unit": "V", "source": "user" },
                "space": { "value": 0.0, "unit": "V", "source": "user" },
                "events": []
            });
        }
    }
    serde_json::to_vec(&json).unwrap()
}

fn serial_rows(session: &Session, name: &str) -> Vec<(u64, u32, Vec<u8>)> {
    let netlist = netlist_of(session);
    let id = netlist.part_id(&Name::new(name).unwrap()).unwrap();
    let field = netlist.part(id).model.serial().expect("a serial source");
    field
        .waveform
        .events()
        .iter()
        .map(|event| {
            assert_eq!(event.frame(), Frame::EightN1);
            (
                event.at().as_ps(),
                event.baud().get(),
                event.bytes().to_vec(),
            )
        })
        .collect()
}

/// The source's one event has ended, so the line is idle. A send asked for before `now`
/// lands at `now`.
#[test]
fn a_serial_send_asked_before_now_lands_at_now() {
    let mut session = open_document(&v_serial_bytes(), Rows::Grid, BUDGET);
    let now = 2_600_000_000;
    let asked = 2_200_000_000;
    session.advance_to(Time::from_ps(now));
    assert_eq!(session.now().as_ps(), now);
    assert!(asked > frame_end(1_000_000_000, 9600, 1) && asked < now);
    assert_eq!(
        session.apply(&send("v1", asked, 9600, &[0x32])),
        Ok(Applied {
            at: Time::from_ps(now),
            status: Status::Running,
        })
    );
    assert_eq!(
        serial_rows(&session, "v1"),
        vec![(1_000_000_000, 9600, vec![0x31]), (now, 9600, vec![0x32]),]
    );
}

/// The source's one event ends at 2041666666 ps. A send asked for at open lands there.
#[test]
fn a_serial_send_lands_at_the_end_of_the_source_s_last_event() {
    let mut session = open_document(&v_serial_bytes(), Rows::Grid, BUDGET);
    let end = frame_end(1_000_000_000, 9600, 1);
    assert_eq!(end, 2_041_666_666);
    assert_eq!(
        session.apply(&send("v1", 0, 9600, &[0x32])),
        Ok(Applied {
            at: Time::from_ps(end),
            status: Status::Running,
        })
    );
    assert_eq!(
        serial_rows(&session, "v1"),
        vec![(1_000_000_000, 9600, vec![0x31]), (end, 9600, vec![0x32]),]
    );
    ends_as_its_export(session);
}

/// A second send asked for while the first send's frames are still on the wire starts at
/// that send's end. A later ask lands at the time it asks for. Three bytes are one event.
#[test]
fn a_second_send_during_the_first_s_frames_starts_at_its_end() {
    let stop = 9_000_000_000;
    let input = with_stop(&v_serial_bytes(), stop);
    let mut session = open_document(&input, Rows::Grid, BUDGET);
    let first = frame_end(1_000_000_000, 9600, 1);
    assert_eq!(
        session.apply(&send("v1", 0, 9600, &[0x41, 0x42, 0x43])),
        Ok(Applied {
            at: Time::from_ps(first),
            status: Status::Running,
        })
    );
    let during = first + 10_000;
    session.advance_to(Time::from_ps(during));
    assert!(session.now().as_ps() < frame_end(first, 9600, 3));
    let second = frame_end(first, 9600, 3);
    assert_eq!(
        session.apply(&send("v1", 0, 9600, &[0x44])),
        Ok(Applied {
            at: Time::from_ps(second),
            status: Status::Running,
        })
    );
    let idle = frame_end(second, 9600, 1);
    let asked = 7_000_000_000;
    assert!(asked > idle && asked < stop);
    assert_eq!(
        session.apply(&send("v1", asked, 9600, &[0x45])),
        Ok(Applied {
            at: Time::from_ps(asked),
            status: Status::Running,
        })
    );
    assert_eq!(
        serial_rows(&session, "v1"),
        vec![
            (1_000_000_000, 9600, vec![0x31]),
            (first, 9600, vec![0x41, 0x42, 0x43]),
            (second, 9600, vec![0x44]),
            (asked, 9600, vec![0x45]),
        ]
    );
    ends_as_its_export(session);
}

/// An idle serial source has no event end to wait for. The first send lands at 1 ps.
#[test]
fn the_first_send_on_an_idle_source_lands_at_1_ps() {
    let mut session = open_document(
        &without_serial_events(&v_serial_bytes()),
        Rows::Grid,
        BUDGET,
    );
    assert_eq!(
        session.apply(&send("v1", 0, 9600, &[0x31])),
        Ok(Applied {
            at: Time::from_ps(1),
            status: Status::Running,
        })
    );
    assert_eq!(serial_rows(&session, "v1"), vec![(1, 9600, vec![0x31])]);
    ends_as_its_export(session);
}

/// The stiff row's first step is rejected, so the reach is 1000 ps. The send lands there.
#[test]
fn a_serial_send_inside_a_rejected_attempt_lands_at_the_reach() {
    let mut session = open_document(&stiff_serial(), Rows::Grid, BUDGET);
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
        session.apply(&send("v1", 41, 9600, &[0x31])),
        Ok(Applied {
            at: Time::from_ps(1_000),
            status: Status::Running,
        })
    );
    ends_as_its_export(session);
}

#[test]
fn a_serial_send_is_refused_when_it_changes_nothing_or_lands_too_late() {
    let mut session = open_document(&v_serial_bytes(), Rows::Grid, BUDGET);
    let stop = transient(netlist_of(&session)).stop.as_ps();
    let before = session.export();
    assert_eq!(
        session.apply(&send("v1", 0, 9600, &[])),
        Err(Rejected::NoChange)
    );
    assert_eq!(
        session.apply(&send("v1", stop, 9600, &[])),
        Err(Rejected::NoChange)
    );
    assert_eq!(session.export(), before);
    assert_eq!(
        session.apply(&send("v9", 0, 9600, &[0x31])),
        Err(Rejected::UnknownPart)
    );
    assert_eq!(
        session.apply(&send("v1", stop, 9600, &[0x31])),
        Err(Rejected::PastStop)
    );
    assert_eq!(session.export(), before);
    let overflow = vec![0u8; 1_844_675];
    assert_eq!(
        session.apply(&send("v1", 0, 1, &overflow)),
        Err(Rejected::PastStop)
    );
    assert_eq!(session.export(), before);
    session.advance_to(transient(netlist_of(&session)).stop);
    assert_eq!(
        session.apply(&send("v1", 0, 9600, &[0x31])),
        Err(Rejected::Ended)
    );
}

#[test]
fn a_serial_send_on_the_wrong_part_is_refused() {
    let cases: &[(&str, &[u8], &str, Rejected)] = &[
        ("kinds", KINDS, "vs", Rejected::NotSettable),
        ("kinds", KINDS, "vp", Rejected::NotSettable),
        ("kinds", KINDS, "r1", Rejected::NotSettable),
        ("kinds", KINDS, "v1", Rejected::WrongSetting),
        ("kinds", KINDS, "sw1", Rejected::WrongSetting),
        ("kinds", KINDS, "rv1", Rejected::WrongSetting),
        ("levels", RC_LEVELS, "i1", Rejected::WrongSetting),
        ("levels", RC_LEVELS, "c1", Rejected::NotSettable),
        ("plain", RC_SWITCHES, "v1", Rejected::WrongSetting),
        ("plain", RC_SWITCHES, "c1", Rejected::NotSettable),
    ];
    for (name, input, part, why) in cases {
        let mut session = open_document(input, Rows::Grid, BUDGET);
        let before = session.export();
        assert_eq!(
            session.apply(&send(part, 0, 9600, &[0x31])),
            Err(*why),
            "{name} {part}"
        );
        assert_eq!(session.export(), before, "{name} {part}");
    }
    let diode = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "transient": { "stop_ps": 1000, "max_step_ps": 1000, "grid_ps": 1000 } },
      "ground": "gnd",
      "nets": ["gnd", "a"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "a", "n": "gnd" },
          "voltage": { "value": 0.0, "unit": "V", "source": "user" }
        },
        {
          "name": "d1",
          "kind": "diode",
          "pins": { "anode": "a", "cathode": "gnd" },
          "saturation_current": { "value": 1e-14, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 1.0, "unit": "1", "source": "user" },
          "series_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null }
        }
      ]
    }"#;
    let mut session = open_document(diode, Rows::Grid, BUDGET);
    let before = session.export();
    assert_eq!(
        session.apply(&send("d1", 0, 9600, &[0x31])),
        Err(Rejected::NotSettable)
    );
    assert_eq!(session.export(), before);

    let chip = example("ne555-led", 1_000_000);
    let mut session = open_document(&chip, Rows::Grid, BUDGET);
    let before = session.export();
    assert_eq!(
        session.apply(&send("u1", 0, 9600, &[0x31])),
        Err(Rejected::NotSettable)
    );
    assert_eq!(
        session.apply(&send("led1", 0, 9600, &[0x31])),
        Err(Rejected::NotSettable)
    );
    assert_eq!(session.export(), before);
}

/// A contact or a level on the serial source is the wrong setting, and the events stay.
#[test]
fn a_level_or_a_contact_on_a_serial_source_is_refused() {
    let mut session = open_document(&v_serial_bytes(), Rows::Grid, BUDGET);
    let before = session.export();
    assert_eq!(
        session.apply(&level("v1", 0, 1.0)),
        Err(Rejected::WrongSetting)
    );
    assert_eq!(
        session.apply(&set("v1", 0, Contact::Closed)),
        Err(Rejected::WrongSetting)
    );
    assert_eq!(session.export(), before);
}
