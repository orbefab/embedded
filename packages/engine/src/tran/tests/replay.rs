use crate::netlist::{Baud, Contact, Frame};

use crate::logic::{Change, Level};
use crate::name::Name;

use crate::run_folder::{self, RunFolder};
use crate::serial::SerialByte;
use crate::timestamp::Timestamp;

use super::super::*;
use super::fixtures::transient;
use super::inputs::without_events;
use super::session::{Rng, example, finish};
use crate::active;
use crate::mcu::Mcu;
use crate::session::{EdgeCursor, Input, Serial, SerialCursor, Setting, State, Status};

/// One 60 Hz frame at 1x.
const FRAME_PS: u64 = 16_666_666_667;

/// `packages/engine/tests/circuits/atmega-button` with no switch event, 1 s at 10 ms steps. With the switch open the
/// firmware changes no pin.
fn quiet_button() -> Vec<u8> {
    without_events(&example("atmega-button", u64::MAX))
}

/// `quiet_button` with a second LED charged from a 5 V step through 1 Mohm, with 1 uF across
/// it. Its error check rejects steps near some frames, and each retry lands short of where the
/// copy ran.
fn charging_button() -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(&quiet_button()).unwrap();
    let volts = |value: f64| serde_json::json!({"value": value, "unit": "V", "source": "user"});
    let nets = json["nets"].as_array_mut().unwrap();
    nets.extend(["vchg".into(), "vcap".into()]);
    let parts = json["parts"].as_array_mut().unwrap();
    let mut led = parts
        .iter()
        .find(|part| part["name"] == "led1")
        .unwrap()
        .clone();
    led["name"] = "led2".into();
    led["pins"] = serde_json::json!({"anode": "vcap", "cathode": "gnd"});
    parts.extend([
        serde_json::json!({
            "name": "v2",
            "kind": "voltage_source",
            "pins": {"p": "vchg", "n": "gnd"},
            "voltage": {"kind": "step", "initial": volts(0.0), "final": volts(5.0), "at_ps": 1000},
        }),
        serde_json::json!({
            "name": "r2",
            "kind": "resistor",
            "pins": {"a": "vchg", "b": "vcap"},
            "resistance": {"value": 1e6, "unit": "ohm", "source": "user"},
            "ratings": {"power": null},
        }),
        serde_json::json!({
            "name": "c2",
            "kind": "capacitor",
            "pins": {"a": "vcap", "b": "gnd"},
            "capacitance": {"value": 1e-6, "unit": "F", "source": "user"},
            "initial": null,
        }),
        led,
    ]);
    serde_json::to_vec(&json).unwrap()
}

/// The oscillator cycle the committed core of the run's only chip has reached.
fn committed_cycles(session: &Session) -> u64 {
    let active::Core::Mcu(Mcu::Atmega328p(host)) = &session.actives.cores()[0] else {
        panic!("an atmega328p core");
    };
    host.mcu.cpu().cycles()
}

fn contact(at: Time, to: Contact) -> Input {
    Input {
        part: Name::new("sw1").unwrap(),
        at,
        setting: Setting::Contact(to),
    }
}

/// A rewind runs the core again from a landing near the last call's target, so a contact after
/// any number of quiet frames runs the cycles of one step at most, not those since power-on.
/// Frames 9 and 10 of the charging scene end on steps a retry landed short of the copy.
#[test]
fn a_contact_after_quiet_frames_runs_the_core_again_for_one_step_at_most() {
    for (name, input) in [
        ("atmega-button", quiet_button()),
        ("charging", charging_button()),
    ] {
        let netlist = crate::netlist::parse(&input).unwrap();
        // The example's clock is 16 MHz.
        let step_cycles = transient(&netlist).max_step * 16 / 1_000_000;
        for frames in [1, 4, 9, 10, 16, 40] {
            let mut session = Session::open(netlist.clone()).unwrap();
            for k in 1..=frames {
                session.advance_to(Time::from_ps(k * FRAME_PS));
            }
            let before = committed_cycles(&session);
            let now = session.now();
            let applied = session.apply(&contact(now, Contact::Closed)).unwrap();
            assert_eq!(applied.at, now, "{name} {frames} frames");
            let replayed = committed_cycles(&session) - before;
            assert!(
                replayed <= step_cycles,
                "{name} {frames} frames: the contact ran {replayed} cycles again"
            );
        }
    }
}

/// An input asked for at a frame, relative to `now` there.
#[derive(Clone, Copy)]
enum Ask {
    Contact(Contact),
    /// Halfway from `now` to the end of the held step.
    ContactInsideTheHeldStep(Contact),
    Serial(&'static [u8]),
}

fn input(session: &Session, ask: Ask) -> Input {
    let now = session.now();
    let (part, at, setting) = match ask {
        Ask::Contact(to) => ("sw1", now, Setting::Contact(to)),
        Ask::ContactInsideTheHeldStep(to) => {
            let State::Running { next, .. } = &session.state else {
                panic!("the session runs");
            };
            let at = Time::from_ps(now.as_ps() + (next.at.as_ps() - now.as_ps()) / 2);
            ("sw1", at, Setting::Contact(to))
        }
        Ask::Serial(bytes) => (
            "vrx",
            now,
            Setting::Serial(Serial {
                baud: Baud::new(9600).unwrap(),
                frame: Frame::EightN1,
                bytes: bytes.to_vec(),
            }),
        ),
    };
    Input {
        part: Name::new(part).unwrap(),
        at,
        setting,
    }
}

/// What a caller reads after a frame.
#[derive(Debug, PartialEq)]
struct Read {
    at: Time,
    probe: Vec<u64>,
    interpolated: bool,
    levels: Vec<Level>,
    edges: Vec<Change>,
    serial: Vec<SerialByte>,
    landed: Vec<Time>,
    status: Status,
}

/// `session` advanced frame by frame to its end, with `script`'s inputs applied after the frames
/// they name. `split` gives the targets of the calls that reach each frame, the frame last.
fn frames(
    session: &mut Session,
    frame_ps: u64,
    script: &[(u64, &[Ask])],
    mut split: impl FnMut(&Session, u64) -> Vec<u64>,
) -> Vec<Read> {
    let mut reads = Vec::new();
    let (mut edges, mut serial) = (EdgeCursor::default(), SerialCursor::default());
    let mut frame = 0;
    while session.status() == Status::Running {
        frame += 1;
        for to in split(session, frame * frame_ps) {
            session.advance_to(Time::from_ps(to));
        }
        let landed = script
            .iter()
            .filter(|&&(at, _)| at == frame)
            .flat_map(|(_, asks)| asks.iter())
            .map(|&ask| session.apply(&input(session, ask)).unwrap().at)
            .collect();
        let probe = session.probe();
        let (new_edges, edges_after) = session.edges_since(edges).unwrap();
        let (new_serial, serial_after) = session.serial_since(serial).unwrap();
        reads.push(Read {
            at: probe.at(),
            probe: probe.values().iter().map(|v| v.get().to_bits()).collect(),
            interpolated: probe.interpolated(),
            levels: session.levels().to_vec(),
            edges: new_edges.to_vec(),
            serial: new_serial.to_vec(),
            landed,
            status: session.status(),
        });
        (edges, serial) = (edges_after, serial_after);
    }
    reads
}

/// One call per frame.
fn whole(_: &Session, to: u64) -> Vec<u64> {
    vec![to]
}

/// Up to four seeded calls before each frame: on the step about to land, 1 ps either side of
/// it, and at a random time before the frame.
fn chunked(seed: u64) -> impl FnMut(&Session, u64) -> Vec<u64> {
    let mut rng = Rng(seed);
    move |session, to| {
        let State::Running { now, next, .. } = &session.state else {
            return vec![to];
        };
        let (now, landing) = (now.as_ps(), next.at.as_ps());
        let mut targets: Vec<u64> = (0..rng.below(4))
            .map(|_| match rng.below(4) {
                0 => landing,
                1 => landing - 1,
                2 => landing + 1,
                _ => now + 1 + rng.below(to - now),
            })
            .filter(|&at| at < to)
            .collect();
        targets.sort_unstable();
        targets.push(to);
        targets
    }
}

fn files(folder: &RunFolder) -> Vec<(String, Vec<u8>)> {
    folder
        .files()
        .map(|(name, bytes)| (name.to_string(), bytes.to_vec()))
        .collect()
}

/// A recorded session advanced one call per frame, the same with seeded calls inside each
/// frame, and an unrecorded session with those calls read the same after every frame, with
/// the same inputs at the same frames. The recorded ones write one folder, and a recorded run
/// of the ended session's netlist, which holds each input it took, writes it too. The bytes read
/// across the frames are the bytes of that run, each once, and are returned.
fn splits_read_the_same(
    name: &str,
    input: &[u8],
    frame_ps: u64,
    script: &[(u64, &[Ask])],
) -> Vec<SerialByte> {
    let netlist = crate::netlist::parse(input).unwrap();
    let span = transient(&netlist);
    let recorded = || record(netlist.clone(), &span, Rows::Grid).unwrap();
    let folder_of = |session: Session| {
        let results = finish(session);
        files(&run_folder::render(
            input,
            None,
            &results,
            Timestamp::from_unix_seconds(0),
        ))
    };
    let mut one = recorded();
    let want = frames(&mut one, frame_ps, script, whole);
    let landed: usize = want.iter().map(|read| read.landed.len()).sum();
    let asked: usize = script.iter().map(|(_, asks)| asks.len()).sum();
    assert_eq!(landed, asked, "{name}");
    assert_eq!(want.last().unwrap().status, Status::Stopped, "{name}");
    assert!(want.iter().any(|read| !read.edges.is_empty()), "{name}");
    let landed = one.circuit.netlist.clone();
    let folder = folder_of(one);
    for seed in [0x5eed_2001, 0x5eed_2002] {
        let mut chunked_recorded = recorded();
        let mut unrecorded = Session::open(netlist.clone()).unwrap();
        for session in [&mut chunked_recorded, &mut unrecorded] {
            let got = frames(session, frame_ps, script, chunked(seed));
            assert_eq!(got.len(), want.len(), "{name} {seed:#x}");
            for (got, want) in got.iter().zip(&want) {
                assert_eq!(got, want, "{name} {seed:#x} at {:?}", want.at);
            }
        }
        assert!(
            folder_of(chunked_recorded) == folder,
            "{name} {seed:#x}: the folder differs"
        );
    }
    let mut batch = record(landed, &span, Rows::Grid).unwrap();
    batch.advance_to(Time::from_ps(u64::MAX));
    let read: Vec<SerialByte> = want.iter().flat_map(|read| read.serial.clone()).collect();
    let sent = batch.serial_since(SerialCursor::default()).unwrap().0;
    assert_eq!(read, sent, "{name}");
    assert!(
        folder_of(batch) == folder,
        "{name}: the folder of its landings differs"
    );
    read
}

/// A press after 40 quiet frames and a release right after it, a press asked for inside the
/// held step, and two more pairs a frame or two apart. Then contacts in the charging scene,
/// after frames that end on a short retry.
#[test]
fn contacts_read_the_same_in_any_split() {
    use Ask::{Contact as At, ContactInsideTheHeldStep as Inside};
    let script: &[(u64, &[Ask])] = &[
        (40, &[At(Contact::Closed), At(Contact::Open)]),
        (44, &[Inside(Contact::Closed)]),
        (47, &[At(Contact::Open)]),
        (50, &[At(Contact::Closed)]),
        (51, &[Inside(Contact::Open)]),
        (53, &[At(Contact::Closed), At(Contact::Open)]),
    ];
    splits_read_the_same("atmega-button", &quiet_button(), FRAME_PS, script);
    let script: &[(u64, &[Ask])] = &[
        (9, &[At(Contact::Closed)]),
        (10, &[At(Contact::Open)]),
        (14, &[Inside(Contact::Closed)]),
        (20, &[At(Contact::Open), At(Contact::Closed)]),
        (40, &[At(Contact::Open)]),
    ];
    splits_read_the_same("charging", &charging_button(), FRAME_PS, script);
}

/// The netlist's own sends, a send that waits for them to leave the wire, and two later sends,
/// in frames of 0.5 ms.
#[test]
fn serial_sends_read_the_same_in_any_split() {
    let input = example("atmega-serial-echo", u64::MAX);
    let script: &[(u64, &[Ask])] = &[
        (8, &[Ask::Serial(b"a")]),
        (19, &[Ask::Serial(b"xy")]),
        (20, &[Ask::Serial(b"z")]),
    ];
    let read = splits_read_the_same("atmega-serial-echo", &input, 500_000_000, script);
    assert!(read.len() > 4, "an echo of an applied send");
}
