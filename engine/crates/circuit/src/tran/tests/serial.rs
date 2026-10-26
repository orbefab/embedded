//! The bytes a session delivers from each ATmega328P's USART, against each core run alone.

use sim_core::finite::Finite;
use sim_core::logic::Level;
use sim_core::name::{Name, Ref};
use sim_core::netlist::{Contact, Netlist};
use sim_core::run_folder;
use sim_core::serial::{Serial, SerialByte};
use sim_core::time::Time;
use sim_core::timestamp::Timestamp;

use super::active::{US, circuit, resistor};
use super::avr::{ANALOG_SERIAL, FUSES, MHZ_14_7456, MHZ_16, atmega, ps};
use super::fixtures::*;
use super::session::{Rng, batch, example, serial_of_one_call};
use crate::active::{self, Actives};
use crate::newton::{Circuit, Conditions, Iterate};
use crate::op;
use crate::session::{BUDGET, Budget, Input, SerialCursor, Session, Setting, State, Status};

const MHZ_8: u32 = 8_000_000;
const MHZ_3_2: u32 = 3_200_000;
/// `analog_serial.S` leaves `U2X0` clear and writes `UBRR0` = 103, so a bit is 16 * 104 cycles,
/// and an 8N1 frame is 10 bits.
const FRAME_CYCLES: u64 = 10 * 16 * 104;

/// A byte a core sent, and the picosecond its stop bit ended.
#[derive(Clone, Copy, Debug)]
struct Frame {
    byte: SerialByte,
    end: Time,
}

/// Each ATmega328P's core run alone to `stop`, chip `k` at `clocks[k]` Hz, every sample read on
/// the operating point at t = 0. Its frames in the order their stop bits ended, then in part
/// order.
fn alone(netlist: &Netlist, clocks: &[u32], stop: Time) -> Vec<Frame> {
    let circuit = Circuit::new(netlist.clone()).unwrap();
    let start = settled(&circuit);
    let mut frames = Vec::new();
    for (k, chip) in circuit.actives.iter().enumerate() {
        let active::Core::Avr(core) = &chip.core else {
            unreachable!("an ATmega328P");
        };
        let mut core = (**core).clone();
        let volts: Vec<f64> = chip
            .samples
            .iter()
            .map(|signal| signal.at(&start.iterate.solution))
            .collect();
        while let Some(halt) = core.run_until(stop) {
            if halt.sample {
                core.sample(halt.at, &volts);
            }
        }
        let mut cpu = core.cpu().clone();
        while let Some(sent) = cpu.take_sent(u64::MAX) {
            let time = |cycle| Time::from_ps(ps(cycle, clocks[k]));
            frames.push(Frame {
                byte: SerialByte {
                    at: time(sent.start),
                    usart: u16::try_from(k).unwrap(),
                    byte: sent.byte,
                },
                end: time(sent.end),
            });
        }
    }
    frames.sort_by_key(|frame| (frame.end, frame.byte.usart));
    frames
}

fn settled(circuit: &Circuit) -> op::Settled {
    op::settle(
        circuit,
        None,
        Conditions::NOMINAL,
        Iterate::initial(circuit),
        circuit.power_on(),
        &Actives::power_on(circuit),
        None,
        op::OP_BUDGET,
    )
    .unwrap()
}

fn rows(frames: &[Frame]) -> Vec<SerialByte> {
    frames.iter().map(|frame| frame.byte).collect()
}

/// How many of `frames` ended at or before `at`.
fn ended_by(frames: &[Frame], at: Time) -> usize {
    frames.partition_point(|frame| frame.end <= at)
}

fn log(session: &Session) -> Vec<SerialByte> {
    session
        .serial_since(SerialCursor::default())
        .expect("cursor still held")
        .0
        .to_vec()
}

/// `traces/serial.csv` of a batch run on `netlist` under the step limit of `budget`.
fn batch_csv(netlist: &Netlist, budget: Budget) -> Option<Vec<u8>> {
    let results = batch(netlist, budget);
    let folder = run_folder::render(
        b"{}",
        Some(netlist),
        &results,
        Timestamp::from_unix_seconds(0),
    );
    folder.file("traces/serial.csv").map(<[u8]>::to_vec)
}

/// The rows of a `serial.csv`, each naming its USART by its index in `usarts`.
fn read_csv(csv: &[u8], usarts: &[(Name, &'static str)]) -> Vec<SerialByte> {
    let text = std::str::from_utf8(csv).unwrap();
    let rows = text.strip_prefix("t_ps,part,usart,byte\n").unwrap();
    assert!(rows.is_empty() || rows.ends_with('\n'));
    rows.lines()
        .map(|row| {
            let [at, part, usart, byte] =
                <[&str; 4]>::try_from(row.split(',').collect::<Vec<_>>()).unwrap();
            let index = usarts
                .iter()
                .position(|(p, u)| p.as_str() == part && *u == usart)
                .unwrap();
            SerialByte {
                at: Time::from_ps(at.parse().unwrap()),
                usart: u16::try_from(index).unwrap(),
                byte: byte.parse().unwrap(),
            }
        })
        .collect()
}

fn analog_serial() -> Netlist {
    sim_core::netlist::parse(&example("atmega-analog-serial", u64::MAX)).unwrap()
}

/// The async frames `core` holds.
fn held(core: &active::Core) -> usize {
    let active::Core::Avr(core) = core else {
        unreachable!("an ATmega328P");
    };
    let mut cpu = core.cpu().clone();
    std::iter::from_fn(|| cpu.take_sent(u64::MAX)).count()
}

/// `examples/atmega-analog-serial` for its 19 ms. `analog_serial.S` prints ADC0 on TXD, one
/// line a reading. Each landing that commits the core delivers every byte whose stop bit ended
/// by then, so neither copy holds one after it. The last `\n` ends after the last commit, so
/// only the delivery at `stop` takes it.
#[test]
fn a_landing_delivers_what_its_core_sent_and_stop_delivers_the_rest() {
    let netlist = analog_serial();
    let stop = transient(&netlist).stop;
    let want = alone(&netlist, &[MHZ_16], stop);
    assert_eq!(want.last().unwrap().byte.byte, b'\n');

    let circuit = Circuit::new(netlist).unwrap();
    let start = settled(&circuit);
    let (x, levels) = (&start.iterate.solution, &start.levels);
    let mut actives = Actives::new(&circuit, levels);
    let (mut at, mut committed) = (Time::ZERO, Time::ZERO);
    while at < stop {
        at = actives.horizon(stop);
        let (_, stops) = actives.land(&circuit, at, x, levels, levels, false);
        if stops == 0 {
            continue;
        }
        committed = at;
        for core in actives.cores().iter().chain(actives.ahead_cores()) {
            assert_eq!(held(core), 0, "a frame left on a copy at {at:?}");
        }
    }
    let landed = ended_by(&want, committed);
    assert!(
        landed < want.len(),
        "the last byte ends after the last commit"
    );
    let landings: Vec<_> = actives.deliver(committed).collect();
    assert_eq!(landings, rows(&want[..landed]));
    let rest: Vec<_> = actives.deliver(stop).collect();
    assert_eq!(rest, rows(&want[landed..]));
    assert_eq!(
        actives.deliver(stop).count(),
        0,
        "a second delivery at the same time adds nothing"
    );
}

/// Each call reads the bytes whose stop bit ended after the last call's accepted step and by
/// its own, with the start the core run alone gives them. The calls end on stop-bit ends, 1 ps
/// before them, and short and long strides on.
#[test]
fn each_call_reads_the_bytes_that_ended_by_the_accepted_step() {
    let netlist = analog_serial();
    let stop = transient(&netlist).stop;
    let want = alone(&netlist, &[MHZ_16], stop);
    let mut session = Session::open(netlist).unwrap();
    let mut rng = Rng(0x5eed_3001);
    let (mut cursor, mut read) = (SerialCursor::default(), Vec::new());
    while session.status() == Status::Running {
        let now = session.now();
        let next = want
            .get(ended_by(&want, now))
            .map_or(stop, |frame| frame.end)
            .as_ps();
        let to = match rng.below(4) {
            0 => next,
            1 => (next - 1).max(now.as_ps() + 1),
            2 => now.as_ps() + 1 + rng.below(1_000),
            _ => now.as_ps() + 1 + rng.below(stop.as_ps() / 50),
        };
        session.advance_to(Time::from_ps(to));
        let (bytes, after) = session.serial_since(cursor).expect("cursor still held");
        read.extend_from_slice(bytes);
        cursor = after;
        let accepted = session.accepted();
        assert_eq!(
            read,
            rows(&want[..ended_by(&want, accepted)]),
            "at {accepted:?}"
        );
        assert_eq!(
            session.serial_since(cursor),
            Some((&[][..], cursor)),
            "at {accepted:?}"
        );
    }
    assert_eq!(session.status(), Status::Stopped);
    assert_eq!(read, rows(&want));
    assert_eq!(
        session
            .serial_since(SerialCursor::default())
            .expect("cursor still held")
            .0,
        &read[..]
    );
}

/// A cursor from a session with more bytes reads nothing on one with fewer, and stays.
#[test]
fn a_cursor_past_the_end_reads_nothing() {
    let netlist = analog_serial();
    let mut longer = Session::open(netlist.clone()).unwrap();
    longer.advance_to(Time::from_ps(12_000 * US));
    let (bytes, past) = longer
        .serial_since(SerialCursor::default())
        .expect("cursor still held");
    assert!(!bytes.is_empty());
    let shorter = Session::open(netlist).unwrap();
    assert_eq!(shorter.serial_since(past), Some((&[][..], past)));
}

/// The step limit ends the run partway, and the bytes are the ones whose stop bit ended by
/// `now`, where it ended.
#[test]
fn a_fault_ends_the_bytes_at_now() {
    let netlist = analog_serial();
    let stop = transient(&netlist).stop;
    let want = alone(&netlist, &[MHZ_16], stop);
    let mut whole = Session::open(netlist.clone()).unwrap();
    whole.advance_to(stop);
    let budget = Budget {
        steps: whole.stats.landings / 2,
        ..BUDGET
    };
    let mut session = open(&netlist, Vec::new(), budget);
    assert_eq!(session.advance_to(stop), Status::Faulted);
    let ended = ended_by(&want, session.now());
    assert!(0 < ended && ended < want.len(), "{ended} of {}", want.len());
    assert_eq!(log(&session), rows(&want[..ended]));
    let csv = batch_csv(&netlist, budget).expect("a faulted run writes serial.csv");
    assert_eq!(read_csv(&csv, session.usarts()), log(&session));
}

/// A call that ends where the first line's `\n` ends does not read it, because no step lands
/// there. Then the re-solve after a new wiper position hits the step limit, and the session ends
/// at the accepted step, with the log it had.
#[test]
fn a_fault_on_a_re_solve_keeps_the_log() {
    let netlist = analog_serial();
    let want = alone(&netlist, &[MHZ_16], transient(&netlist).stop);
    let newline = want
        .iter()
        .position(|frame| frame.byte.byte == b'\n')
        .unwrap();
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.advance_to(want[newline].end);
    let accepted = session.accepted();
    assert!(accepted < want[newline].end, "{accepted:?}");
    let (bytes, cursor) = session
        .serial_since(SerialCursor::default())
        .expect("cursor still held");
    assert_eq!(bytes, rows(&want[..newline]));
    session.budget.steps = session.stats.landings;
    let input = Input {
        part: Name::new("rv1").unwrap(),
        at: session.now(),
        setting: Setting::Level(Finite::new(0.25).unwrap()),
    };
    assert_eq!(session.apply(&input).unwrap().status, Status::Faulted);
    assert_eq!(session.now(), accepted);
    assert_eq!(log(&session), rows(&want[..newline]));
    assert_eq!(session.serial_since(cursor), Some((&[][..], cursor)));
}

/// `examples/atmega-analog-serial` with 10 kohm from `PD2` to VCC and a switch from `PD2` to GND,
/// which `analog_serial.S` never reads. `sw2` and `sw3` each close 10 kohm from VCC to GND,
/// on a net no chip watches. `swr` pulls `RESET` to GND.
fn analog_serial_with_a_switch() -> Netlist {
    let mut json: serde_json::Value =
        serde_json::from_slice(&example("atmega-analog-serial", u64::MAX)).unwrap();
    let parts = json["parts"].as_array_mut().unwrap();
    parts.push(serde_json::from_str(&resistor("rp", "vcc", "pd2", 10e3)).unwrap());
    parts.push(serde_json::from_str(&resistor("rq", "vcc", "q", 10e3)).unwrap());
    for (name, a) in [("sw1", "pd2"), ("sw2", "q"), ("sw3", "q"), ("swr", "pc6")] {
        parts.push(serde_json::json!({
            "name": name, "kind": "switch", "pins": { "a": a, "b": "gnd" },
            "on_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
            "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
            "initial": "open", "events": [], "bounce": null
        }));
    }
    json["nets"].as_array_mut().unwrap().push("q".into());
    sim_core::netlist::parse(&serde_json::to_vec(&json).unwrap()).unwrap()
}

fn close(switch: &str, at: Time) -> Input {
    Input {
        part: Name::new(switch).unwrap(),
        at,
        setting: Setting::Contact(Contact::Closed),
    }
}

/// `sw2` lands a step 1 ps before the first `\n` ends, and the `\n` is not read. `sw3` lands a
/// step where it ends, which delivers it from the copy run ahead, since the core has not
/// stopped since before it. `sw1` then closes 1 ps later, `PD2` flips on the restart's held
/// solve, and the core runs again from its last commit and sends the `\n` again. It is in the
/// log once.
#[test]
fn a_rewind_past_a_delivered_byte_delivers_it_once() {
    let netlist = analog_serial_with_a_switch();
    let stop = transient(&netlist).stop;
    let want = alone(&netlist, &[MHZ_16], stop);
    let newline = want
        .iter()
        .position(|frame| frame.byte.byte == b'\n')
        .unwrap();
    let end = want[newline].end;
    let before = Time::from_ps(end.as_ps() - 1);
    let mut session = Session::open(netlist).unwrap();
    session.advance_to(before);
    assert_eq!(session.apply(&close("sw2", before)).unwrap().at, before);
    assert_eq!(session.accepted(), before);
    assert_eq!(log(&session), rows(&want[..newline]));
    assert_eq!(session.apply(&close("sw3", end)).unwrap().at, end);
    session.advance_to(end);
    assert_eq!(session.accepted(), end);
    assert_eq!(log(&session), rows(&want[..=newline]));
    let after = Time::from_ps(end.as_ps() + 1);
    assert_eq!(session.apply(&close("sw1", after)).unwrap().at, after);
    session.advance_to(after);
    assert_eq!(session.accepted(), after);
    assert_eq!(log(&session), rows(&want[..=newline]));
    assert_eq!(session.advance_to(stop), Status::Stopped);
    assert_eq!(log(&session), rows(&want));
}

/// `examples/atmega-analog-serial` with `RESET` through 5 nF from a node that 12.1 kohm and 5 nF
/// filter off `vd`, and 10 kohm from `RESET` to VCC. `vd` falls from 13.609 V to 0 V at `fall`,
/// and `RESET` dips under its low threshold for about 1.2 us, 41.7 us later. `sw2` closes 10 kohm
/// from VCC to GND, on a net no chip watches.
fn analog_serial_with_a_reset_dip(fall: Time) -> Vec<u8> {
    let mut json: serde_json::Value =
        serde_json::from_slice(&example("atmega-analog-serial", u64::MAX)).unwrap();
    let parts = json["parts"].as_array_mut().unwrap();
    parts.push(serde_json::json!({
        "name": "vd", "kind": "voltage_source", "pins": { "p": "d", "n": "gnd" },
        "voltage": {
            "kind": "steps",
            "initial": { "value": 13.609, "unit": "V", "source": "user" },
            "events": [{ "t_ps": fall.as_ps(), "level": { "value": 0.0, "unit": "V", "source": "user" } }]
        }
    }));
    parts.push(serde_json::from_str(&resistor("rd", "d", "a", 12.1e3)).unwrap());
    parts.push(serde_json::from_str(&resistor("rr", "vcc", "pc6", 10e3)).unwrap());
    parts.push(serde_json::from_str(&resistor("rq", "vcc", "q", 10e3)).unwrap());
    for (name, a, b) in [("cd1", "a", "gnd"), ("cd2", "a", "pc6")] {
        parts.push(serde_json::json!({
            "name": name, "kind": "capacitor", "pins": { "a": a, "b": b },
            "capacitance": { "value": 5e-9, "unit": "F", "source": "user" }, "initial": null
        }));
    }
    parts.push(serde_json::json!({
        "name": "sw2", "kind": "switch", "pins": { "a": "q", "b": "gnd" },
        "on_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
        "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
        "initial": "open", "events": [], "bounce": null
    }));
    let nets = json["nets"].as_array_mut().unwrap();
    nets.extend(["d".into(), "a".into(), "q".into()]);
    serde_json::to_vec(&json).unwrap()
}

/// `RESET` dips over the first `\n`'s end, and the step held over that end never shows it. A
/// call that ends there stands inside the dip. `sw2` then lands there, and the re-solve's step
/// ends inside the dip, so `RESET` flips before the `\n` ends, and the reset drops the frame on
/// the wire. The session never read the `\n`. Its bytes are the first bytes of one call on its
/// export, and once it has ended they are all of them.
///
/// The 13.609 V and the 41.7 us lead came from a search over `vd`'s level near where the dip
/// first crosses 2.1 V, with 12.1 kohm, for a dip that no step end falls inside, with margin each
/// side. A change to the steps around that end fails one of the premise assertions, and
/// re-tuning is the same search.
#[test]
fn a_byte_after_the_accepted_step_waits_for_the_steps_an_input_moves() {
    let end = alone(
        &analog_serial(),
        &[MHZ_16],
        transient(&analog_serial()).stop,
    )
    .into_iter()
    .find(|frame| frame.byte.byte == b'\n')
    .unwrap()
    .end;
    let json = analog_serial_with_a_reset_dip(Time::from_ps(end.as_ps() - 41_700_000));
    let netlist = sim_core::netlist::parse(&json).unwrap();
    let stop = transient(&netlist).stop;
    let want = alone(&netlist, &[MHZ_16], stop);
    let newline = ended_by(&want, end) - 1;
    assert_eq!((want[newline].byte.byte, want[newline].end), (b'\n', end));
    let reset = Ref::Logic(Name::new("u1").unwrap(), Name::new("PC6").unwrap());
    let pc6 = Ref::Net(Name::new("pc6").unwrap());

    let mut session = Session::open_document(&json).unwrap();
    session.advance_to(end);
    let State::Running { accepted, next, .. } = &session.state else {
        panic!("the session ended");
    };
    assert!(
        accepted.at < end && end < next.at,
        "{:?} {:?}",
        accepted.at,
        next.at
    );
    assert_eq!(session.level(&reset), Some(Level::High));
    let dip = session.probe().get(&pc6).unwrap().get();
    assert!(dip < 0.42 * 5.0, "RESET at {dip} V");
    assert_eq!(log(&session), rows(&want[..newline]));
    assert_eq!(session.apply(&close("sw2", end)).unwrap().at, end);
    assert_eq!(session.level(&reset), Some(Level::Low));
    let exported = sim_core::netlist::parse(&session.export().unwrap()).unwrap();
    assert!(serial_of_one_call(&exported, BUDGET).starts_with(&log(&session)));
    assert_eq!(session.advance_to(stop), Status::Stopped);
    let exported = sim_core::netlist::parse(&session.export().unwrap()).unwrap();
    assert_eq!(log(&session), serial_of_one_call(&exported, BUDGET));
    assert!(!log(&session).contains(&want[newline].byte));
}

/// A switch closes `PD2` to GND on the start bit of the third byte. The core stops there, so the
/// landing commits it, and the restart's held solve flips `PD2` on that twin. `analog_serial.S`
/// never reads `PD2`, so the bytes are the ones the core sends alone, each once.
#[test]
fn an_input_on_a_twin_delivers_no_byte_twice() {
    let netlist = analog_serial_with_a_switch();
    let stop = transient(&netlist).stop;
    let want = alone(&netlist, &[MHZ_16], stop);
    let start = want[2].byte.at;
    let mut session = Session::open(netlist).unwrap();
    session.advance_to(Time::from_ps(start.as_ps() - US));
    assert_eq!(session.apply(&close("sw1", start)).unwrap().at, start);
    assert_eq!(session.advance_to(stop), Status::Stopped);
    assert_eq!(log(&session), rows(&want));
}

/// `swr` pulls `RESET` low 1 us after the first `\n` ends, and no step has landed since that
/// end. The core runs again from its last commit to the reset, which keeps the frames that
/// ended, so the `\n` is read. The core then stays in reset.
#[test]
fn a_byte_that_ended_before_a_reset_is_read() {
    let netlist = analog_serial_with_a_switch();
    let stop = transient(&netlist).stop;
    let want = alone(&netlist, &[MHZ_16], stop);
    let newline = want
        .iter()
        .position(|frame| frame.byte.byte == b'\n')
        .unwrap();
    let end = want[newline].end;
    let reset = Time::from_ps(end.as_ps() + US);
    let mut session = Session::open(netlist).unwrap();
    session.advance_to(Time::from_ps(end.as_ps() - 1));
    assert_eq!(log(&session), rows(&want[..newline]));
    assert_eq!(session.apply(&close("swr", reset)).unwrap().at, reset);
    session.advance_to(Time::from_ps(reset.as_ps() - 1));
    assert!(session.accepted() < end, "a step landed before the reset");
    assert_eq!(log(&session), rows(&want[..newline]));
    session.advance_to(reset);
    assert_eq!(session.accepted(), reset);
    let pin = Ref::Logic(Name::new("u1").unwrap(), Name::new("PC6").unwrap());
    assert_eq!(session.level(&pin), Some(Level::Low));
    assert_eq!(log(&session), rows(&want[..=newline]));
    assert_eq!(session.advance_to(stop), Status::Stopped);
    assert_eq!(log(&session), rows(&want[..=newline]));
}

/// `part` with its name and every net but `vcc` and `gnd` prefixed by `prefix`.
fn renamed(part: &str, prefix: &str) -> (String, Vec<String>) {
    let mut part: serde_json::Value = serde_json::from_str(part).unwrap();
    part["name"] = format!("{prefix}1").into();
    let mut nets = Vec::new();
    for net in part["pins"].as_object_mut().unwrap().values_mut() {
        if net != "vcc" && net != "gnd" {
            let name = format!("{prefix}_{}", net.as_str().unwrap());
            if !nets.contains(&name) {
                nets.push(name.clone());
            }
            *net = name.into();
        }
    }
    (part.to_string(), nets)
}

/// One ATmega328P per `(prefix, clock)` printing ADC0 off its own divider, each named
/// `<prefix>1` on nets prefixed `<prefix>_`. `sw<prefix>` closes `PD2`, which `analog_serial.S`
/// never reads, to GND, and 10 kohm pulls it up.
fn boards(stop: u64, chips: &[(&str, u32)]) -> Netlist {
    boards_with(stop, chips, Vec::new())
}

/// `boards` with `extra` parts, on its nets.
fn boards_with(stop: u64, chips: &[(&str, u32)], extra: Vec<String>) -> Netlist {
    let mut parts = extra;
    let mut nets = Vec::new();
    for &(prefix, clock) in chips {
        let (part, pins) = renamed(&atmega("vcc", ANALOG_SERIAL, clock, FUSES), prefix);
        let pc0 = format!("{prefix}_pc0");
        let pd2 = format!("{prefix}_pd2");
        parts.push(resistor(&format!("rt{prefix}"), "vcc", &pc0, 3300.0));
        parts.push(resistor(&format!("rb{prefix}"), &pc0, "gnd", 6800.0));
        parts.push(resistor(&format!("rp{prefix}"), "vcc", &pd2, 10e3));
        parts.push(
            serde_json::json!({
                "name": format!("sw{prefix}"), "kind": "switch", "pins": { "a": pd2, "b": "gnd" },
                "on_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
                "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
                "initial": "open", "events": [], "bounce": null
            })
            .to_string(),
        );
        parts.push(part);
        nets.extend(pins);
    }
    let nets: Vec<&str> = nets.iter().map(String::as_str).collect();
    circuit(stop, &nets, &parts)
}

/// Two ATmega328Ps print ADC0 at 16 MHz and at 8 MHz, so `u`'s frames are half as long as
/// `w`'s, and some start after one of `w`'s and end before it. The log is in the order stop
/// bits ended, then in part order, and `usarts` names each chip's USART0.
#[test]
fn two_chips_interleave_by_stop_bit_end_then_part() {
    let stop = 19_000 * US;
    let netlist = boards(stop, &[("u", MHZ_16), ("w", MHZ_8)]);
    let want = alone(&netlist, &[MHZ_16, MHZ_8], Time::from_ps(stop));
    let mut by_start = want.clone();
    by_start.sort_by_key(|frame| (frame.byte.at, frame.byte.usart));
    assert_ne!(rows(&by_start), rows(&want), "a later start ends first");
    assert!(want.iter().any(|frame| frame.byte.usart == 0));
    assert!(want.iter().any(|frame| frame.byte.usart == 1));

    let mut session = Session::open(netlist).unwrap();
    let name = |part: &str| Name::new(part).unwrap();
    assert_eq!(
        session.usarts(),
        [(name("u1"), "USART0"), (name("w1"), "USART0")]
    );
    session.advance_to(Time::from_ps(stop));
    assert_eq!(log(&session), rows(&want));
}

/// `sim run`'s `serial.csv` on two chips at 16 MHz and 8 MHz is the log of a session advanced to
/// `stop` through the writer, byte for byte, and its rows are the log's, which a later start
/// can end ahead of.
#[test]
fn the_batch_writes_the_log_of_a_session_advanced_to_stop() {
    let stop = 19_000 * US;
    let netlist = boards(stop, &[("u", MHZ_16), ("w", MHZ_8)]);
    let mut session = Session::open(netlist.clone()).unwrap();
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    let csv = batch_csv(&netlist, BUDGET).unwrap();
    let log = log(&session);
    assert_eq!(
        csv,
        Serial::new(session.usarts().to_vec(), log.clone()).csv()
    );
    assert_eq!(read_csv(&csv, session.usarts()), log);
    assert!(!log.is_sorted_by_key(|byte| byte.at));
}

/// Two ATmega328Ps print ADC0 at 16 MHz and at 3.2 MHz, whose frames are five times as long.
/// `swrw` holds `w` in reset until 24,979,375,000 ps, which a search chose so that a byte of
/// each ends at 96,454,062,500 ps. `w`'s started first, and part order puts `u`'s first.
#[test]
fn a_tie_goes_in_part_order_whichever_byte_started_first() {
    let stop = 97_000 * US;
    let release = serde_json::json!({
        "name": "swrw", "kind": "switch", "pins": { "a": "w_pc6", "b": "gnd" },
        "on_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
        "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
        "initial": "closed", "events": [{ "t_ps": 24_979_375_000u64, "state": "open" }],
        "bounce": null
    });
    let extra = vec![resistor("rrw", "vcc", "w_pc6", 10e3), release.to_string()];
    let clocks = [MHZ_16, MHZ_3_2];
    let netlist = boards_with(stop, &[("u", clocks[0]), ("w", clocks[1])], extra);
    let mut session = Session::open(netlist).unwrap();
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    let ended: Vec<(u64, SerialByte)> = log(&session)
        .into_iter()
        .map(|byte| {
            let clock = clocks[usize::from(byte.usart)];
            (byte.at.as_ps() + ps(FRAME_CYCLES, clock), byte)
        })
        .collect();
    let tie = ended
        .windows(2)
        .position(|pair| pair[0].0 == pair[1].0)
        .expect("two bytes end together");
    let ((end, u), (_, w)) = (ended[tie], ended[tie + 1]);
    assert_eq!(end, 96_454_062_500);
    assert!(w.at < u.at, "w's byte started first");
    assert_eq!((u.usart, w.usart), (0, 1));
    assert!(ended.is_sorted_by_key(|&(end, byte)| (end, byte.usart)));
}

/// Three ATmega328Ps print ADC0 at 16 MHz, so each byte ends on all three at once, and the log
/// holds the three in part order.
#[test]
fn three_bytes_that_end_together_go_in_part_order() {
    let stop = 2_000 * US;
    let netlist = boards(stop, &[("u", MHZ_16), ("w", MHZ_16), ("x", MHZ_16)]);
    let mut session = Session::open(netlist).unwrap();
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    let got: Vec<_> = log(&session)
        .iter()
        .map(|byte| (byte.at.as_ps(), byte.usart, byte.byte))
        .collect();
    assert_eq!(
        got,
        [
            (254_062_500, 0, b'6'),
            (254_062_500, 1, b'6'),
            (254_062_500, 2, b'6')
        ]
    );
}

/// Two ATmega328Ps print ADC0 at 16 MHz, so their bytes end on the same picoseconds, and part
/// order puts `u`'s first. `sww` lands a step where two `\n` bytes end and no byte starts, so
/// `w` commits there and `u` does not, and `w`'s byte is delivered before `u`'s.
#[test]
fn two_bytes_that_end_together_go_in_part_order() {
    let stop = 19_000 * US;
    let netlist = boards(stop, &[("u", MHZ_16), ("w", MHZ_16)]);
    let want = alone(&netlist, &[MHZ_16, MHZ_16], Time::from_ps(stop));
    let tie = want
        .windows(2)
        .position(|pair| pair[0].end == pair[1].end && pair[0].byte.byte == b'\n')
        .unwrap();
    let end = want[tie].end;
    assert!(
        want.iter().all(|frame| frame.byte.at != end),
        "a byte starts at the tie"
    );
    let mut session = Session::open(netlist).unwrap();
    session.advance_to(Time::from_ps(end.as_ps() - 1));
    assert!(session.accepted() < end);
    assert_eq!(session.apply(&close("sww", end)).unwrap().at, end);
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    assert_eq!(log(&session), rows(&want));
}

/// At 14.7456 MHz a cycle is not a whole number of picoseconds. A byte's time is its start
/// bit's cycle rounded up to the next picosecond, where the core stops for that `TXD` fall, and
/// so is its stop bit's end. `swu` lands a step on the picosecond before an end, which the end's
/// cycle has not reached, so the byte is read only at a later step, once.
#[test]
fn a_time_between_picoseconds_rounds_up() {
    let stop = 19_000 * US;
    let netlist = boards(stop, &[("u", MHZ_14_7456)]);
    let want = alone(&netlist, &[MHZ_14_7456], Time::from_ps(stop));
    let on_a_picosecond =
        |at: Time| u128::from(at.as_ps()) * u128::from(MHZ_14_7456) % 1_000_000_000_000 == 0;
    assert!(want.iter().any(|frame| !on_a_picosecond(frame.byte.at)));
    let k = want
        .iter()
        .position(|frame| !on_a_picosecond(frame.end))
        .unwrap();
    let before = Time::from_ps(want[k].end.as_ps() - 1);
    let mut session = Session::open(netlist).unwrap();
    session.advance_to(Time::from_ps(before.as_ps() - 1));
    assert!(session.accepted() < before);
    assert_eq!(session.apply(&close("swu", before)).unwrap().at, before);
    session.advance_to(before);
    assert_eq!(session.accepted(), before);
    assert_eq!(log(&session), rows(&want[..k]));
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    assert_eq!(log(&session), rows(&want));
}
