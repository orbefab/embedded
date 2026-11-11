//! A pulse target that is no chain: a test-only device on a line that rests high. It keeps each
//! width the host times and each idle, and shows nothing, so it has no `leds` entry and no
//! `leds.csv` row, though it takes `Pulses` records.

use orbe_model::{Cx, Device, Drives, Error, Idle, Params, PulseTarget, State, Stop, Time};

use super::active::{US, circuit, net};
use super::fixtures::*;
use super::session::finish;
use crate::active::native::Native;
use crate::active::pulse::Pulse;
use crate::active::{self, Core, Takes};
use crate::mna::Node;
use crate::netlist::Netlist;
use crate::run_folder::{self, Observations};
use crate::session::{BUDGET, Session};
use crate::timestamp::Timestamp;
use crate::watch::{Signal, Watch};

const STOP: u64 = 40 * US;
const START: u64 = 5 * US;
/// Low, high, low, from `START`. The line rests high around them.
const TRAIN: [(bool, u64); 3] = [(false, 2 * US), (true, US), (false, 3 * US)];

#[derive(Clone, Copy, Debug)]
struct Widths {
    pulses: [(u64, bool, u64); 8],
    len: usize,
    idles: [(u64, bool); 2],
    idled: usize,
}

impl State for Widths {}

impl Device for Widths {
    fn new(_params: &Params<'_>) -> Result<Widths, Error> {
        Ok(Widths {
            pulses: [(0, false, 0); 8],
            len: 0,
            idles: [(0, false); 2],
            idled: 0,
        })
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }
}

impl PulseTarget for Widths {
    const IDLE: Idle = Idle {
        level: true,
        after: 10 * US,
    };

    fn pulse(&mut self, _cx: &mut Cx, at: Time, level: bool, width: u64) {
        self.pulses[self.len] = (at.as_ps(), level, width);
        self.len += 1;
    }

    fn idle(&mut self, _cx: &mut Cx, at: Time, level: bool) {
        self.idles[self.idled] = (at.as_ps(), level);
        self.idled += 1;
    }
}

/// The device watching `line` at 2.5 V, and taking `Pulses` records on it when `records`.
fn widths(netlist: &Netlist, records: bool) -> active::Chip {
    let line = net(netlist, "line");
    let device = Widths::new(&orbe_model::Fields::NONE).unwrap();
    active::Chip {
        part: None,
        outputs: Vec::new(),
        watches: vec![Watch::comparator(Signal {
            terms: vec![(Node::from(line), 1.0)],
            offset: 2.5,
        })],
        signals: Vec::new(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        takes: Takes {
            pulses: records.then_some(line),
            ..Takes::default()
        },
        core: Core::Device(Box::new(Native::new(device, Pulse::new(), 0))),
    }
}

/// `TRAIN` as one record from a `pulses_replay` whose `OUT` rests high.
fn replayed() -> Netlist {
    let pulses: Vec<String> = TRAIN
        .iter()
        .map(|&(level, ps)| format!(r#"{{ "level": {level}, "ps": {ps} }}"#))
        .collect();
    let ohm = r#"{ "value": 25.0, "unit": "ohm", "source": "user" }"#;
    let volt = r#"{ "value": 0.0, "unit": "V", "source": "user" }"#;
    circuit(
        STOP,
        &["line"],
        &[format!(
            r#"{{ "name": "rp1", "kind": "pulses_replay",
              "pins": {{ "VCC": "vcc", "GND": "gnd", "OUT": "line" }}, "idle": true,
              "output": {{ "high_drop": {volt}, "high_resistance": {ohm},
                          "low_drop": {volt}, "low_resistance": {ohm} }},
              "events": [{{ "t_ps": {START}, "pulses": [{}] }}] }}"#,
            pulses.join(", ")
        )],
    )
}

/// `TRAIN` as the steps of a source on the line, which rests high.
fn stepped() -> Netlist {
    let mut t = START;
    let steps: Vec<String> = TRAIN
        .iter()
        .chain([&(true, 0)])
        .map(|&(level, ps)| {
            let volts = if level { 5.0 } else { 0.0 };
            let step = format!(
                r#"{{ "t_ps": {t}, "level": {{ "value": {volts:?}, "unit": "V", "source": "user" }} }}"#
            );
            t += ps;
            step
        })
        .collect();
    circuit(
        STOP,
        &["line"],
        &[format!(
            r#"{{ "name": "v2", "kind": "voltage_source", "pins": {{ "p": "line", "n": "gnd" }},
              "voltage": {{ "kind": "steps",
                "initial": {{ "value": 5.0, "unit": "V", "source": "user" }},
                "events": [{}] }} }}"#,
            steps.join(", ")
        )],
    )
}

fn device(session: &Session) -> Widths {
    session
        .actives
        .cores()
        .iter()
        .find_map(|core| match core {
            Core::Device(dev) => dev
                .as_any()
                .downcast_ref::<Native<Widths, Pulse<Widths>>>()
                .map(|native| *native.device()),
            _ => None,
        })
        .expect("the widths device is a core")
}

/// The device after the whole run, and the run folder's files. The run has no `leds`.
fn run(netlist: &Netlist, records: bool) -> (Widths, Vec<String>) {
    let mut session = open(netlist, vec![widths(netlist, records)], BUDGET);
    session.advance_to(Time::from_ps(STOP));
    let seen = device(&session);
    let results = finish(session);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let Observations::Transient { leds, end, .. } = results.observations() else {
        panic!("a transient");
    };
    assert_eq!(*end, Time::from_ps(STOP));
    assert!(leds.is_none(), "{leds:?}");
    let folder = run_folder::render(
        b"{}",
        Some(netlist),
        &results,
        Timestamp::from_unix_seconds(0),
    );
    let files = folder.files().map(|(path, _)| path.to_string()).collect();
    (seen, files)
}

#[test]
fn a_pulse_target_that_rests_high_sees_each_width_and_idles_high_and_shows_no_leds() {
    let end = START + TRAIN.iter().map(|&(_, ps)| ps).sum::<u64>();
    let expected = [
        (START, true, START),
        (START + 2 * US, false, 2 * US),
        (START + 3 * US, true, US),
        (end, false, 3 * US),
    ];
    for (netlist, records) in [(replayed(), true), (stepped(), false)] {
        let (seen, files) = run(&netlist, records);
        assert_eq!(&seen.pulses[..seen.len], &expected, "records {records}");
        assert_eq!(
            &seen.idles[..seen.idled],
            &[(end + 10 * US, true)],
            "records {records}"
        );
        assert!(
            !files.iter().any(|path| path.ends_with("leds.csv")),
            "{files:?}"
        );
    }
}
