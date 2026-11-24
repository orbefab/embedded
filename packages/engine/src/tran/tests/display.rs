//! The `st77916` panel on records: its rows and image in any chunking, its reset pin, its
//! backlight duty, and the transfers it drops with a warning.

use crate::netlist::{Contact, Netlist};

use crate::diagnostic::{Diagnostic, ProtocolRule};
use crate::display::Brightness;
use crate::name::Name;

use crate::run_folder::{self, RunFolder};
use crate::timestamp::Timestamp;

use super::super::*;
use super::active::{NS, US, net, scripted};
use super::fixtures::*;
use super::session::{Rng, example, finish};
use crate::active::script::{self, Op, Taken};
use crate::active::{Core, SpiTarget};
use crate::session::{DeviceEvent, Input, Rejected, Setting, State};

const STOP: u64 = 20 * US;
/// One byte on one lane at 10 MHz.
const BYTE_PS: u64 = 800 * NS;

fn folder(session: Session) -> RunFolder {
    run_folder::render(
        b"{}",
        None,
        &finish(session),
        Timestamp::from_unix_seconds(0),
    )
}

/// Advances to the end in seeded chunks of 1 ps, a step, or a stride.
fn chunked(session: &mut Session, stop: u64, seed: u64) {
    let mut rng = Rng(seed);
    while let State::Running { now, next, .. } = &session.state {
        let (now, landing) = (now.as_ps(), next.at.as_ps());
        let to = match rng.below(4) {
            0 => now + 1,
            1 => landing,
            2 => landing - 1,
            _ => now + 1 + rng.below(stop / 20),
        };
        session.advance_to(Time::from_ps(to));
    }
}

/// `packages/engine/tests/circuits/st77916-replay` in one call and in seeded chunks writes the same `display.csv`
/// and frame memory, and `Session::display` reads that frame memory at the stop.
#[test]
fn the_example_writes_the_same_rows_and_image_in_any_chunking() {
    let json = example("st77916-replay", u64::MAX);
    let netlist = crate::netlist::parse(&json).unwrap();
    let whole = transient(&netlist);
    let stop = whole.stop.as_ps();
    let lcd1 = Name::new("lcd1").unwrap();
    let mut folders = Vec::new();
    for seed in [None, Some(3), Some(17)] {
        let mut session = record(netlist.clone(), &whole, Rows::Grid).unwrap();
        match seed {
            None => {
                session.advance_to(Time::from_ps(stop));
            }
            Some(seed) => chunked(&mut session, stop, seed),
        }
        let image = session.display(&lcd1).unwrap().image();
        let folder = folder(session);
        assert_eq!(
            folder.file("traces/display-lcd1.rgb565").unwrap(),
            image,
            "{seed:?}"
        );
        folders.push(folder);
    }
    let csv = std::str::from_utf8(folders[0].file("traces/display.csv").unwrap()).unwrap();
    assert_eq!(csv.lines().count(), 8, "{csv}");
    for folder in &folders[1..] {
        for path in ["traces/display.csv", "traces/display-lcd1.rgb565"] {
            assert_eq!(folder.file(path), folders[0].file(path), "{path}");
        }
    }
}

/// One register or pixel command, 32 bits on one lane, then `bytes` on `lanes`.
fn command(at: u64, opcode: u8, command: u8, lanes: u8, bytes: &[u8], cs: &str) -> String {
    let value = u32::from(command) << 8;
    format!(
        r#"{{ "t_ps": {at}, "phases": [{{ "bits": 8, "lanes": 1, "value": {opcode} }},
            {{ "bits": 24, "lanes": 1, "value": {value} }}], "lanes": {lanes},
            "bytes": {bytes:?}, "cs": "{cs}" }}"#
    )
}

fn data(at: u64, bytes: &[u8], cs: &str) -> String {
    format!(r#"{{ "t_ps": {at}, "phases": [], "lanes": 4, "bytes": {bytes:?}, "cs": "{cs}" }}"#)
}

/// A 10 MHz replay of `events` on `cs`, `sclk` and `d0` to `d3`, except that its D1 is on
/// `d1`, a panel `lcd1` on the same nets with RST on `rst` and BLK on `blk`, RST pulled up
/// through 10 kohm, and a switch `sw1` from `rst` to ground, open. Steps are at most
/// `max_step` long.
fn panel_netlist(events: &[String], d1: &str, max_step: u64) -> Netlist {
    let stage = r#"{
        "high_drop": { "value": 0.0, "unit": "V" },
        "high_resistance": { "value": 25.0, "unit": "ohm" },
        "low_drop": { "value": 0.0, "unit": "V" },
        "low_resistance": { "value": 25.0, "unit": "ohm" } }"#;
    let json = format!(
        r#"{{
          "version": 1,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {STOP}, "max_step_ps": {max_step}, "grid_ps": {max_step} }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "cs", "sclk", "d0", "d1", "d2", "d3", "dx", "rst", "blk", "te", "o1", "o2", "x"],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{ "value": 3.3, "unit": "V" }} }},
            {{ "name": "rp1", "kind": "spi_replay",
              "pins": {{ "VCC": "vcc", "GND": "gnd", "CS": "cs", "SCLK": "sclk",
                        "D0": "d0", "D1": "{d1}", "D2": "d2", "D3": "d3" }},
              "clock_hz": {{ "value": 10000000, "unit": "Hz" }},
              "output": {stage}, "events": [{}] }},
            {{ "name": "lcd1", "kind": "st77916",
              "pins": {{ "VCC": "vcc", "GND": "gnd", "CS": "cs", "SCLK": "sclk", "D0": "d0",
                        "D1": "d1", "D2": "d2", "D3": "d3", "RST": "rst", "TE": "te",
                        "BLK": "blk" }},
              "output": {stage},
              "input_threshold": {{ "low": {{ "value": 0.3, "unit": "1" }},
                                   "high": {{ "value": 0.7, "unit": "1" }} }} }},
            {{ "name": "rrst", "kind": "resistor", "pins": {{ "a": "vcc", "b": "rst" }},
              "resistance": {{ "value": 10000.0, "unit": "ohm" }},
              "ratings": {{ "power": null }} }},
            {{ "name": "sw1", "kind": "switch", "pins": {{ "a": "rst", "b": "gnd" }},
              "on_resistance": {{ "value": 10.0, "unit": "ohm" }},
              "off_resistance": {{ "value": 1e6, "unit": "ohm" }},
              "initial": "open", "events": [], "bounce": null }},
            {{ "name": "rblk", "kind": "resistor", "pins": {{ "a": "blk", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }},
              "ratings": {{ "power": null }} }},
            {{ "name": "rdx", "kind": "resistor", "pins": {{ "a": "dx", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }},
              "ratings": {{ "power": null }} }},
            {{ "name": "ro1", "kind": "resistor", "pins": {{ "a": "o1", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }},
              "ratings": {{ "power": null }} }},
            {{ "name": "ro2", "kind": "resistor", "pins": {{ "a": "o2", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }},
              "ratings": {{ "power": null }} }},
            {{ "name": "rx", "kind": "resistor", "pins": {{ "a": "x", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }},
              "ratings": {{ "power": null }} }}
          ]
        }}"#,
        events.join(", "),
    );
    crate::netlist::parse(json.as_bytes()).unwrap()
}

fn bus(netlist: &Netlist) -> SpiTarget {
    SpiTarget {
        cs: net(netlist, "cs"),
        sclk: net(netlist, "sclk"),
        pins: None,
    }
}

fn lcd1() -> Name {
    Name::new("lcd1").unwrap()
}

fn image(session: &Session) -> Vec<u8> {
    session.display(&lcd1()).unwrap().image()
}

fn taken(session: &Session, k: usize) -> Vec<Taken> {
    let Core::Script(core) = &session.actives.cores()[k] else {
        panic!("chip {k} is a script");
    };
    core.taken().to_vec()
}

/// The panel takes a held pixel write, then RST falls while the data-only rest is in flight.
/// The reset drops the open write with no row, the pixel already written stays, and the rest
/// is dropped while RST is low. A script sender watching RST holds a record past the fall. The
/// fall lands more than one step limit before the call's target, so the landing keeps no core,
/// RST flips only on the held solve, and the input runs the sender again from its last commit.
/// A script receiver on the bus still takes that record once.
#[test]
fn a_reset_drops_the_open_write_and_a_sender_watching_rst_delivers_once() {
    let events = [
        command(US, 0x02, 0x3a, 1, &[0x55], "released"),
        command(6 * US, 0x32, 0x2c, 4, &[1, 2], "held"),
        data(12 * US, &[3, 4], "released"),
    ];
    let netlist = panel_netlist(&events, "d1", 4 * US);
    let send = 11 * US;
    let mut sender = script::sender(
        scripted(&netlist, "o1", "rst", &[(send, Op::Send(0xa0))], None),
        "zeta",
        bus(&netlist),
    );
    let sends = script::sends(&mut sender);
    let receiver = script::receiver(scripted(&netlist, "o2", "x", &[], None), bus(&netlist));
    let mut session = open(&netlist, vec![sender, receiver], BUDGET);
    let fall = 10 * US + 500 * NS;
    session.advance_to(Time::from_ps(9 * US));
    assert_eq!(session.now().as_ps(), 9 * US);
    let ahead = sends.get();
    assert!(ahead >= 1, "the sender ran ahead to its send");
    let applied = session
        .apply(&Input {
            part: Name::new("sw1").unwrap(),
            at: Time::from_ps(fall),
            setting: Setting::Contact(Contact::Closed),
        })
        .unwrap();
    assert_eq!(applied.at.as_ps(), fall);
    session.advance_to(Time::from_ps(STOP));
    assert_eq!(
        sends.get(),
        ahead + 1,
        "the input ran the sender again over its send"
    );
    let from_zeta: Vec<Taken> = taken(&session, 3)
        .into_iter()
        .filter(|(_, bytes, _)| *bytes == [0xa0])
        .collect();
    assert_eq!(from_zeta, [(Time::from_ps(send), vec![0xa0], 0)]);
    let panel = session.display(&lcd1()).unwrap().panel();
    assert!(panel.flags.reset);
    let image = image(&session);
    assert_eq!(image[..4], [1, 2, 0, 0]);
    let folder = folder(session);
    assert_eq!(
        folder.file("traces/display.csv").unwrap(),
        b"t_ps,part,x0,y0,x1,y1,bytes,hash\n"
    );
}

/// A script on BLK's net sets a duty, and the panel reports it as its brightness.
#[test]
fn blk_takes_a_duty_record_as_the_brightness() {
    let netlist = panel_netlist(&[], "d1", US);
    let sender = script::sender(
        scripted(&netlist, "blk", "x", &[(3 * US, Op::Duty(700))], None),
        "zeta",
        bus(&netlist),
    );
    let mut session = open(&netlist, vec![sender], BUDGET);
    session.advance_to(Time::from_ps(3 * US - 1));
    assert_eq!(session.display(&lcd1()).unwrap().panel().brightness, None);
    session.advance_to(Time::from_ps(STOP));
    assert_eq!(
        session.display(&lcd1()).unwrap().panel().brightness,
        Some(Brightness {
            duty: 700,
            steps: 1024,
            hz: 5000,
        })
    );
}

/// A replay whose D1 sits on another net than the panel's sends COLMOD on D0 alone, then a
/// pixel write on four lanes. The panel runs COLMOD, drops the pixel write, and warns
/// `chip.link_miswired` once.
#[test]
fn a_transaction_on_other_data_nets_is_dropped_with_one_warning() {
    let events = [
        command(US, 0x02, 0x3a, 1, &[0x55], "released"),
        command(6 * US, 0x32, 0x2c, 4, &[1, 2], "released"),
    ];
    let netlist = panel_netlist(&events, "dx", US);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.advance_to(Time::from_ps(STOP));
    assert_eq!(
        session
            .display(&lcd1())
            .unwrap()
            .panel()
            .settings
            .get("colmod")
            .unwrap(),
        0x55
    );
    let first = 6 * US + (32 + 4) * BYTE_PS / 8;
    let chips: Vec<Diagnostic> = session
        .diagnostics()
        .into_iter()
        .filter(|found| found.code().starts_with("chip."))
        .collect();
    assert_eq!(
        chips,
        [Diagnostic::ChipLinkMiswired {
            part: "#lcd1".parse().unwrap(),
            at: Time::from_ps(first),
            sender: Name::new("rp1").unwrap(),
        }]
    );
    assert!(image(&session).iter().all(|&byte| byte == 0));
    let folder = folder(session);
    let diagnostics = std::str::from_utf8(folder.file("diagnostics.json").unwrap()).unwrap();
    assert!(
        diagnostics.contains("\"code\": \"chip.link_miswired\""),
        "{diagnostics}"
    );
}

/// Command words a release cuts to 8 bits and to 24, and pixels before COLMOD picks 16 bits,
/// warn `chip.protocol` once a rule, at its first time, and write nothing. The pixel write after
/// COLMOD lands.
#[test]
fn each_broken_rule_warns_once_at_its_first_time() {
    let events = [
        data(US, &[9], "released"),
        data(2 * US, &[9], "released"),
        format!(
            r#"{{ "t_ps": {}, "phases": [{{ "bits": 24, "lanes": 1, "value": 2 }}], "lanes": 1,
                "bytes": [], "cs": "released" }}"#,
            3 * US
        ),
        command(6 * US, 0x32, 0x2c, 4, &[7, 7], "released"),
        command(10 * US, 0x02, 0x3a, 1, &[0x55], "released"),
        command(15 * US, 0x32, 0x2c, 4, &[5, 6], "released"),
    ];
    let netlist = panel_netlist(&events, "d1", US);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    chunked(&mut session, STOP, 9);
    let ends = [US + 2 * BYTE_PS / 8, 6 * US + (32 + 4) * BYTE_PS / 8];
    let rule = |rule, at| Diagnostic::ChipProtocol {
        part: "#lcd1".parse().unwrap(),
        at: Time::from_ps(at),
        rule,
    };
    assert_eq!(
        session.diagnostics(),
        [
            rule(ProtocolRule::Command, ends[0]),
            rule(ProtocolRule::PixelFormat, ends[1]),
        ]
    );
    assert_eq!(image(&session)[..4], [5, 6, 0, 0]);
}

/// A chip that is no display part has no display.
#[test]
fn a_part_that_is_no_display_reads_none() {
    let netlist = panel_netlist(&[], "d1", US);
    let session = open(&netlist, Vec::new(), BUDGET);
    assert!(session.display(&Name::new("rp1").unwrap()).is_none());
    assert!(session.display(&lcd1()).is_some());
}

/// A panel and a replay declare no event, so an event setting on either is not settable, as on
/// a part that takes no setting of any kind.
#[test]
fn an_event_on_a_device_that_declares_none_is_not_settable() {
    let netlist = panel_netlist(&[], "d1", US);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    for part in ["lcd1", "rp1"] {
        let touch = Input {
            part: Name::new(part).unwrap(),
            at: Time::from_ps(US),
            setting: Setting::Event(DeviceEvent {
                name: "touch".to_string(),
                fields: Vec::new(),
            }),
        };
        assert_eq!(session.apply(&touch), Err(Rejected::NotSettable), "{part}");
    }
}
