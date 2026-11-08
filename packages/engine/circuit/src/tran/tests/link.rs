//! Records between active chips: a receiver takes each record once, at its end, whatever the
//! part order, the chunking, or the rewinds a landing makes.

use orbe_engine_core::name::Name;
use orbe_engine_core::netlist::{Contact, Netlist};
use orbe_engine_core::run_folder;
use orbe_engine_core::serial::SerialByte;
use orbe_engine_core::timestamp::Timestamp;

use super::super::*;
use super::active::{NS, US, circuit, net, resistor, scripted};
use super::fixtures::*;
use super::session::{Rng, finish};
use crate::active::script::{self, Op, Taken};
use crate::active::{self, Core, Drive, SpiTarget};
use crate::session::{Budget, Input, SerialCursor, Setting, State, Status};

const STOP: u64 = 10 * US;
const T0: u64 = 2 * US + 333;
const T1: u64 = 5 * US + 77;
const T2: u64 = 6 * US + 5;
const T3: u64 = 7 * US;

/// Three outputs and a shared input, each through 1 kohm to ground, and the bus nets.
fn netlist() -> Netlist {
    circuit(
        STOP,
        &["o1", "o2", "o3", "x", "cs", "sclk"],
        &[
            resistor("r1", "o1", "gnd", 1000.0),
            resistor("r2", "o2", "gnd", 1000.0),
            resistor("r3", "o3", "gnd", 1000.0),
            resistor("rx", "x", "gnd", 1000.0),
            resistor("rcs", "cs", "gnd", 1000.0),
            resistor("rclk", "sclk", "gnd", 1000.0),
        ],
    )
}

fn bus(netlist: &Netlist) -> SpiTarget {
    SpiTarget {
        cs: net(netlist, "cs"),
        sclk: net(netlist, "sclk"),
    }
}

/// `zeta` sends A0 to A2 and `alpha` B0 to B2. A0 and B0 end together, and so do A2 and B2.
/// `zeta` comes before `alpha` in part order but after it by name.
fn zeta(netlist: &Netlist) -> active::Chip {
    let sends = [
        (T0, Op::Send(0xa0)),
        (T1, Op::Send(0xa1)),
        (T3, Op::Send(0xa2)),
    ];
    script::sender(
        scripted(netlist, "o1", "x", &sends, None),
        "zeta",
        bus(netlist),
    )
}

fn alpha(netlist: &Netlist) -> active::Chip {
    let sends = [
        (T0, Op::Send(0xb0)),
        (T2, Op::Send(0xb1)),
        (T3, Op::Send(0xb2)),
    ];
    script::sender(
        scripted(netlist, "o2", "x", &sends, None),
        "alpha",
        bus(netlist),
    )
}

fn receiver(netlist: &Netlist) -> active::Chip {
    script::receiver(scripted(netlist, "o3", "x", &[], None), bus(netlist))
}

/// What the receiver takes: at each end, by sender name, then by sequence.
fn expected() -> Vec<Taken> {
    [
        (T0, 0xb0, 0),
        (T0, 0xa0, 0),
        (T1, 0xa1, 1),
        (T2, 0xb1, 1),
        (T3, 0xb2, 2),
        (T3, 0xa2, 2),
    ]
    .map(|(at, tag, seq)| (Time::from_ps(at), vec![tag], seq))
    .to_vec()
}

fn taken(session: &Session, k: usize) -> Vec<Taken> {
    let Core::Script(core) = &session.actives.cores()[k] else {
        panic!("chip {k} is a script");
    };
    core.taken().to_vec()
}

fn script_core(session: &Session, k: usize) -> &script::Core {
    let Core::Script(core) = &session.actives.cores()[k] else {
        panic!("chip {k} is a script");
    };
    core
}

/// `traces/link.csv` of a finished session.
fn link_csv(netlist: &Netlist, session: Session) -> String {
    let folder = run_folder::render(
        b"{}",
        Some(netlist),
        &finish(session),
        Timestamp::from_unix_seconds(0),
    );
    String::from_utf8(folder.file("traces/link.csv").unwrap().to_vec()).unwrap()
}

/// Advances to the end in seeded chunks. A chunk ends on the step about to land, 1 ps either
/// side of it, or a short or long stride on.
fn chunked(session: &mut Session, seed: u64) {
    let mut rng = Rng(seed);
    while let State::Running { now, next, .. } = &session.state {
        let (now, landing) = (now.as_ps(), next.at.as_ps());
        let to = match rng.below(5) {
            0 => landing,
            1 => landing - 1,
            2 => landing + 1,
            3 => now + 1 + rng.below(1_000),
            _ => now + 1 + rng.below(STOP / 10),
        };
        session.advance_to(Time::from_ps(to));
    }
}

/// Every order of the receiver and the two senders, in one call and in seeded chunks, gives the
/// receiver the same log and writes the same `link.csv`.
#[test]
fn a_receiver_takes_each_record_once_in_any_part_order_and_any_chunking() {
    let netlist = netlist();
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut csvs = Vec::new();
    for order in orders {
        for seed in [None, Some(1), Some(7), Some(42)] {
            let chips: Vec<active::Chip> = order
                .iter()
                .map(|&k| [receiver, zeta, alpha][k](&netlist))
                .collect();
            let at = order.iter().position(|&k| k == 0).unwrap();
            let mut session = open(&netlist, chips, BUDGET);
            match seed {
                None => {
                    session.advance_to(Time::from_ps(STOP));
                }
                Some(seed) => chunked(&mut session, seed),
            }
            assert_eq!(taken(&session, at), expected(), "{order:?} {seed:?}");
            csvs.push(link_csv(&netlist, session));
        }
    }
    let rows: Vec<&str> = csvs[0].lines().collect();
    assert_eq!(rows.len(), 7, "{}", csvs[0]);
    assert_eq!(
        rows[1],
        format!("{T0},{T0},alpha,CS,spi,released,,1,{},1", hash(&[0xb0]))
    );
    assert_eq!(
        rows[2],
        format!("{T0},{T0},zeta,CS,spi,released,,1,{},1", hash(&[0xa0]))
    );
    assert!(csvs.iter().all(|csv| *csv == csvs[0]));
}

fn hash(bytes: &[u8]) -> String {
    orbe_engine_core::link::hash(bytes)
}

/// The receiver comes first in part order, `zeta` after it and `alpha` last. Both send at T0.
/// At the one landing there the receiver takes both, by name, both senders are committed at
/// T0, and every record cursor is T0.
#[test]
fn a_receiver_ahead_of_both_senders_takes_both_at_their_landing() {
    let netlist = netlist();
    let chips = vec![receiver(&netlist), zeta(&netlist), alpha(&netlist)];
    let mut session = open(&netlist, chips, BUDGET);
    session.advance_to(Time::from_ps(T0));
    assert_eq!(taken(&session, 0), expected()[..2]);
    let t0 = Time::from_ps(T0);
    for k in [1, 2] {
        let sender = script_core(&session, k);
        assert_eq!(sender.now(), t0, "sender {k} is committed at T0");
        assert!(sender.records().is_empty(), "sender {k} holds no record");
    }
    assert_eq!(session.actives.record_cursors(), [t0; 3]);
}

/// A driver flips `x`, which every script watches, 300 ps before each send, and each chunk ends
/// on the step about to land, so every core is kept at every landing. A sender that ran on to
/// its send is run again from its last commit to the flip, and sends again later. Each record
/// still reaches the receiver once.
#[test]
fn a_sender_run_again_after_a_rewind_delivers_each_record_once() {
    let netlist = netlist();
    let flips: Vec<(u64, Op)> = [T0, T1, T2, T3]
        .iter()
        .zip([Drive::High, Drive::Low].iter().cycle())
        .map(|(&at, &drive)| (at - 300, Op::Drive(drive)))
        .collect();
    let driver = scripted(&netlist, "x", "o3", &flips, None);
    let (mut zeta, mut alpha) = (zeta(&netlist), alpha(&netlist));
    let sends = [script::sends(&mut zeta), script::sends(&mut alpha)];
    let chips = vec![driver, zeta, alpha, receiver(&netlist)];
    let mut session = open(&netlist, chips, BUDGET);
    while let State::Running { next, .. } = &session.state {
        let landing = next.at;
        session.advance_to(landing);
    }
    let made: u64 = sends.iter().map(script::Sends::get).sum();
    assert!(made > 6, "a sender ran again over a send, {made} sends");
    assert_eq!(taken(&session, 3), expected());
    let csv = link_csv(&netlist, session);
    assert_eq!(csv.lines().count(), 7, "{csv}");
}

/// A call that ends at T1 keeps every core at the landing there. `zeta` stops on its record,
/// and the driver has no stop near T1. Both are committed at T1 with their cursors there, and
/// the receiver takes A1 once, there and over the rest of the run.
#[test]
fn a_kept_landing_commits_a_sender_and_a_core_with_no_stop() {
    let netlist = netlist();
    let driver = scripted(
        &netlist,
        "x",
        "o3",
        &[(9 * US, Op::Drive(Drive::Off))],
        None,
    );
    let chips = vec![driver, zeta(&netlist), receiver(&netlist)];
    let mut session = open(&netlist, chips, BUDGET);
    session.advance_to(Time::from_ps(T1 - NS));
    session.advance_to(Time::from_ps(T1));
    let t1 = Time::from_ps(T1);
    assert_eq!(script_core(&session, 0).now(), t1);
    assert_eq!(script_core(&session, 1).now(), t1);
    assert_eq!(session.actives.record_cursors(), [t1; 3]);
    let a = |at, tag, seq| (Time::from_ps(at), vec![tag], seq);
    assert_eq!(taken(&session, 2), [a(T0, 0xa0, 0), a(T1, 0xa1, 1)]);
    session.advance_to(Time::from_ps(STOP));
    assert_eq!(
        taken(&session, 2),
        [a(T0, 0xa0, 0), a(T1, 0xa1, 1), a(T3, 0xa2, 2)]
    );
}

/// Two consoles send bytes that all start and end at T0: `u2` 64 bytes, ahead in part order,
/// and `u1` three. Every run, in one call or in chunks, delivers `u1`'s by name first, then
/// `u2`'s, each in the order its chip sent them.
#[test]
fn bytes_that_end_together_come_by_part_name_then_in_the_order_sent() {
    let netlist = netlist();
    let console = |out, name, first, count| {
        let bytes = [(T0, Op::Bytes { first, count })];
        script::console(scripted(&netlist, out, "x", &bytes, None), name)
    };
    let expected: Vec<u8> = (100..103).chain(0..64).collect();
    for seed in [None, Some(3), Some(11)] {
        let chips = vec![console("o1", "u2", 0, 64), console("o2", "u1", 100, 3)];
        let mut session = open(&netlist, chips, BUDGET);
        match seed {
            None => {
                session.advance_to(Time::from_ps(STOP));
            }
            Some(seed) => chunked(&mut session, seed),
        }
        let (bytes, _) = session.serial_since(SerialCursor::default()).unwrap();
        let got: Vec<u8> = bytes.iter().map(|byte| byte.byte).collect();
        assert_eq!(got, expected, "{seed:?}");
        assert!(bytes.iter().all(|byte| byte.at == Time::from_ps(T0)));
        let usarts: Vec<u16> = bytes.iter().map(|byte: &SerialByte| byte.usart).collect();
        assert_eq!(usarts, [[1; 3].as_slice(), &[0; 64]].concat());
    }
}

/// A `spi_replay` part sends each event at its end, and a script on its CS and SCLK nets takes
/// each one there, in order, once, in seeded chunks.
#[test]
fn a_replay_part_reaches_a_receiver_on_its_nets() {
    let replay = r#"{ "name": "rp1", "kind": "spi_replay",
          "pins": { "VCC": "vcc", "GND": "gnd", "CS": "cs", "SCLK": "sclk",
                    "D0": "o1", "D1": "o2", "D2": "x", "D3": "x" },
          "clock_hz": { "value": 10000000, "unit": "Hz", "source": "user" },
          "output": {
            "high_drop": { "value": 0.0, "unit": "V", "source": "user" },
            "high_resistance": { "value": 25.0, "unit": "ohm", "source": "user" },
            "low_drop": { "value": 0.0, "unit": "V", "source": "user" },
            "low_resistance": { "value": 25.0, "unit": "ohm", "source": "user" } },
          "events": [
            { "t_ps": 1000000, "phases": [{ "bits": 8, "lanes": 1, "value": 2 }], "lanes": 1,
              "bytes": [17, 34], "cs": "held" },
            { "t_ps": 4000000, "phases": [], "lanes": 4, "bytes": [51], "cs": "released" }
          ] }"#;
    let netlist = circuit(
        STOP,
        &["o1", "o2", "o3", "x", "cs", "sclk"],
        &[
            replay.to_string(),
            resistor("r3", "o3", "gnd", 1000.0),
            resistor("rx", "x", "gnd", 1000.0),
        ],
    );
    let mut session = open(&netlist, vec![receiver(&netlist)], BUDGET);
    chunked(&mut session, 5);
    // 8 + 16 cycles at 10 MHz from 1 us, then 2 cycles from 4 us.
    assert_eq!(
        taken(&session, 1),
        [
            (Time::from_ps(US + 2400 * NS), vec![17, 34], 0),
            (Time::from_ps(4 * US + 200 * NS), vec![51], 1)
        ]
    );
}

/// One SPI byte at 10 MHz is 16 cycles, 1.6 us. `end` is `start` plus that.
fn replay_end(start: u64) -> u64 {
    start + 16 * 1_000_000_000_000 / 10_000_000
}

fn replay_event(start: u64, byte: u8, cs: &str) -> String {
    format!(
        r#"{{ "t_ps": {start}, "phases": [{{ "bits": 8, "lanes": 1, "value": 2 }}],
            "lanes": 1, "bytes": [{byte}], "cs": "{cs}" }}"#
    )
}

/// A 10 MHz replay of `events`, on the script receiver's bus, with `max_step_ps` steps.
fn replay_netlist(stop: u64, max_step: u64, events: &str, extra: &[String]) -> Netlist {
    let replay = format!(
        r#"{{ "name": "rp1", "kind": "spi_replay",
          "pins": {{ "VCC": "vcc", "GND": "gnd", "CS": "cs", "SCLK": "sclk",
                    "D0": "o1", "D1": "o2", "D2": "x", "D3": "x" }},
          "clock_hz": {{ "value": 10000000, "unit": "Hz", "source": "user" }},
          "output": {{
            "high_drop": {{ "value": 0.0, "unit": "V", "source": "user" }},
            "high_resistance": {{ "value": 25.0, "unit": "ohm", "source": "user" }},
            "low_drop": {{ "value": 0.0, "unit": "V", "source": "user" }},
            "low_resistance": {{ "value": 25.0, "unit": "ohm", "source": "user" }} }},
          "events": [ {events} ] }}"#
    );
    let nets: Vec<String> = ["gnd", "vcc", "o1", "o2", "o3", "x", "cs", "sclk", "sw"]
        .iter()
        .map(|net| format!("{net:?}"))
        .collect();
    let mut parts = vec![
        replay,
        resistor("r3", "o3", "gnd", 1000.0),
        resistor("rx", "x", "gnd", 1000.0),
        resistor("rsw", "sw", "gnd", 1000.0),
    ];
    parts.extend_from_slice(extra);
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {max_step}, "grid_ps": {max_step} }} }},
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
    orbe_engine_core::netlist::parse(json.as_bytes()).unwrap()
}

fn link_times(csv: &str) -> Vec<u64> {
    csv.lines()
        .skip(1)
        .map(|row| row.split(',').next().unwrap().parse().unwrap())
        .collect()
}

/// A switch contact is applied while the first record has started and the receiver has not
/// taken it. The receiver still takes each record once, at its end. No chip watches the switch,
/// so the input reaches no sender's `input`. The test pins the landing at the input's time while
/// the sender is stopped past `now`.
#[test]
fn an_input_while_a_record_is_in_flight_delivers_each_record_once() {
    let first = 2 * US;
    let second = 6 * US;
    let (end1, end2) = (replay_end(first), replay_end(second));
    let events = format!(
        "{}, {}",
        replay_event(first, 17, "released"),
        replay_event(second, 34, "released")
    );
    let switch = r#"{ "name": "sw1", "kind": "switch", "pins": { "a": "vcc", "b": "sw" },
         "on_resistance": { "value": 10.0, "unit": "ohm", "source": "user" },
         "off_resistance": { "value": 1e6, "unit": "ohm", "source": "user" },
         "initial": "open", "events": [], "bounce": null }"#;
    let netlist = replay_netlist(STOP, STOP, &events, &[switch.to_string()]);
    let mut session = open(&netlist, vec![receiver(&netlist)], BUDGET);
    let mid = first + 500 * NS;
    session.advance_to(Time::from_ps(mid));
    assert_eq!(session.now().as_ps(), mid);
    assert_eq!(taken(&session, 1), []);
    session
        .apply(&Input {
            part: Name::new("sw1").unwrap(),
            at: Time::from_ps(mid),
            setting: Setting::Contact(Contact::Closed),
        })
        .unwrap();
    session.advance_to(Time::from_ps(STOP));
    let want = [
        (Time::from_ps(end1), vec![17], 0),
        (Time::from_ps(end2), vec![34], 1),
    ];
    assert_eq!(taken(&session, 1), want);
    assert_eq!(link_times(&link_csv(&netlist, session)), vec![end1, end2]);
}

/// A record that ends at `stop_ps` is in `link.csv`. One that ends after `stop_ps` is not.
#[test]
fn a_record_that_ends_at_stop_is_kept_and_one_after_stop_is_not() {
    let start = US;
    let end = replay_end(start);
    let later = replay_end(end);
    let events = format!(
        "{}, {}",
        replay_event(start, 17, "held"),
        replay_event(end, 34, "released")
    );
    let netlist = replay_netlist(end, US, &events, &[]);
    let mut session = open(&netlist, vec![receiver(&netlist)], BUDGET);
    assert_eq!(session.advance_to(Time::from_ps(later)), Status::Stopped);
    assert_eq!(session.now().as_ps(), end);
    assert_eq!(taken(&session, 1), [(Time::from_ps(end), vec![17], 0)]);
    assert_eq!(link_times(&link_csv(&netlist, session)), vec![end]);
}

/// The step limit ends the run on the first record. `link.csv` keeps that record and not the
/// one that ends after `end_ps`.
#[test]
fn an_error_keeps_the_records_that_ended_by_end_ps() {
    let first = 2 * US;
    let second = 6 * US;
    let (end1, end2) = (replay_end(first), replay_end(second));
    let events = format!(
        "{}, {}",
        replay_event(first, 17, "released"),
        replay_event(second, 34, "released")
    );
    let netlist = replay_netlist(STOP, US, &events, &[]);
    let want = [
        (Time::from_ps(end1), vec![17], 0),
        (Time::from_ps(end2), vec![34], 1),
    ];
    let mut whole = open(&netlist, vec![receiver(&netlist)], BUDGET);
    let mut steps = None;
    while whole.status() == Status::Running {
        let State::Running { next, .. } = &whole.state else {
            break;
        };
        let at = next.at;
        whole.advance_to(at);
        if steps.is_none() && whole.now().as_ps() >= end1 {
            assert_eq!(whole.now().as_ps(), end1);
            steps = Some(whole.stats.landings);
        }
    }
    assert_eq!(taken(&whole, 1), want);
    let steps = steps.expect("the first record landed");
    let mut session = open(
        &netlist,
        vec![receiver(&netlist)],
        Budget { steps, ..BUDGET },
    );
    assert_eq!(session.advance_to(Time::from_ps(STOP)), Status::Faulted);
    assert_eq!(session.fault().unwrap().code(), "solver.step_limit");
    assert_eq!(session.now().as_ps(), end1);
    assert_eq!(link_times(&link_csv(&netlist, session)), vec![end1]);
}
