//! Pulse targets that are no chain: test-only devices on a line that rests high. `Widths` keeps
//! each width the host times and each idle, and shows nothing, so it has no `leds` entry and no
//! `leds.csv` row, though it takes `Pulses` records. `Answer` pulls its line low through an open
//! drain: it queues one train, and the host plays it.

use std::sync::Arc;

use orbe_model::{Cx, Device, Drives, Error, Idle, Params, PulseTarget, Pulses, State, Stop, Time};

use super::active::{NS, US, at, capacitor, circuit, net, resistor, run_active, volts};
use super::fixtures::*;
use super::session::finish;
use crate::active::native::Native;
use crate::active::pulse::Pulse;
use crate::active::{self, Core, Output, Stage, Takes};
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

/// When `Answer` queues its train, and when the train starts.
const QUEUE_AT: u64 = US;
const PLAY_AT: u64 = 5 * US;
/// Low, released, low, from `PLAY_AT`. The line is released after it.
const ANSWER: [(bool, u64); 3] = [(false, 3 * US), (true, 5 * US), (false, 2 * US)];
/// The pull-up and the line's load: a 1 us rise.
const PULL_UP: f64 = 10_000.0;
const LOAD: f64 = 100e-12;
/// The open drain's low stage.
const DRAIN: f64 = 25.0;

#[derive(Clone, Copy, Debug)]
struct Answer {
    queued: bool,
    pulses: [(u64, bool, u64); 8],
    len: usize,
    idles: [u64; 2],
    idled: usize,
}

impl State for Answer {}

impl Device for Answer {
    fn new(_params: &Params<'_>) -> Result<Answer, Error> {
        Ok(Answer {
            queued: false,
            pulses: [(0, false, 0); 8],
            len: 0,
            idles: [0; 2],
            idled: 0,
        })
    }

    /// Stops once, at `QUEUE_AT`, and queues the train there.
    fn run_until(&mut self, cx: &mut Cx, limit: Time) -> Option<Stop> {
        if self.queued || limit.as_ps() < QUEUE_AT {
            return None;
        }
        self.queued = true;
        cx.pulses(
            Time::from_ps(PLAY_AT),
            Pulses {
                idle: true,
                pulses: Arc::from(ANSWER),
            },
        );
        Some(Stop {
            at: Time::from_ps(QUEUE_AT),
            sample: false,
            record: false,
        })
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }
}

impl PulseTarget for Answer {
    const IDLE: Idle = Idle {
        level: true,
        after: 10 * US,
    };
    const OPEN_DRAIN: bool = true;

    fn pulse(&mut self, _cx: &mut Cx, at: Time, level: bool, width: u64) {
        self.pulses[self.len] = (at.as_ps(), level, width);
        self.len += 1;
    }

    fn idle(&mut self, _cx: &mut Cx, at: Time, _level: bool) {
        self.idles[self.idled] = at.as_ps();
        self.idled += 1;
    }
}

/// `line` pulled up by `PULL_UP` and loaded by `LOAD`.
fn pulled_up() -> Netlist {
    circuit(
        40 * US,
        &["line"],
        &[
            resistor("rp", "vcc", "line", PULL_UP),
            capacitor("cl", "line", LOAD),
        ],
    )
}

/// `Answer` on `line`, which it watches at 2.5 V and pulls low through `DRAIN`.
fn answer(netlist: &Netlist) -> active::Chip {
    let line = net(netlist, "line");
    let stage = Stage {
        drop: 0.0,
        resistance: DRAIN,
    };
    active::Chip {
        part: None,
        outputs: vec![Output {
            pin: line,
            vcc: net(netlist, "vcc"),
            gnd: net(netlist, "gnd"),
            high: stage,
            low: stage,
            pull_up: None,
            leakage: None,
        }],
        watches: vec![Watch::comparator(Signal {
            terms: vec![(Node::from(line), 1.0)],
            offset: 2.5,
        })],
        signals: Vec::new(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        takes: Takes::default(),
        core: Core::Device(Box::new(Native::new(
            Answer::new(&orbe_model::Fields::NONE).unwrap(),
            Pulse::new(),
            0,
        ))),
    }
}

/// The edges `ANSWER` puts on the line from `PLAY_AT`: each time and whether the drain pulls.
fn edges() -> Vec<(u64, bool)> {
    let mut t = PLAY_AT;
    let mut edges = Vec::new();
    for (level, ps) in ANSWER {
        edges.push((t, !level));
        t += ps;
    }
    edges.push((t, false));
    edges
}

/// The open drain pulls the line to `5 V * DRAIN / (PULL_UP + DRAIN)` with a time constant of
/// `PULL_UP || DRAIN` over `LOAD`, about 2.5 ns, and a release lets `PULL_UP` charge `LOAD`
/// with `tau = PULL_UP * LOAD`, 1 us. A step lands on each edge of the train at its picosecond,
/// and from the line's value there it follows the matching exponential to the next edge. The watch reads the
/// line at 2.5 V, so the device sees each fall about 1.7 ns after its edge and each rise
/// `tau * ln((5 V - low) / 2.5 V)` after it, and times each level between those crossings.
#[test]
fn a_queued_train_lands_each_edge_on_its_picosecond_and_rises_with_the_pull_up() {
    let netlist = pulled_up();
    let (trace, diagnostics) = run_active(&netlist, vec![answer(&netlist)]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let line = volts(&trace, "line");
    let low = 5.0 * DRAIN / (PULL_UP + DRAIN);
    let tau = PULL_UP * LOAD * 1e12;
    let tau_low = PULL_UP * DRAIN / (PULL_UP + DRAIN) * LOAD * 1e12;
    let edges = edges();
    assert!(
        (at(&line, PLAY_AT) - 5.0).abs() < 1e-6,
        "the line rests at 5 V until the train"
    );
    for (k, &(edge, pulls)) in edges.iter().enumerate() {
        let next = edges.get(k + 1).map_or(40 * US, |&(next, _)| next);
        let mut checked = 0;
        let from = at(&line, edge);
        for &(t, v) in line.iter().filter(|&&(t, _)| t > edge && t <= next) {
            let x = (t - edge) as f64;
            let want = if pulls {
                low + (from - low) * libm::exp(-x / tau_low)
            } else {
                5.0 - (5.0 - from) * libm::exp(-x / tau)
            };
            assert!(
                (v - want).abs() < 0.02,
                "{} ps after edge {k}: {v} V, the exponential gives {want} V",
                t - edge
            );
            checked += 1;
        }
        assert!(checked >= 3, "edge {k}: {checked} steps checked");
    }

    let mut session = open(&netlist, vec![answer(&netlist)], BUDGET);
    session.advance_to(Time::from_ps(40 * US));
    let seen = session
        .actives
        .cores()
        .iter()
        .find_map(|core| match core {
            Core::Device(dev) => dev
                .as_any()
                .downcast_ref::<Native<Answer, Pulse<Answer>>>()
                .map(|native| *native.device()),
            _ => None,
        })
        .expect("the answer device is a core");
    let fall = tau_low * libm::log((5.0 - low) / (2.5 - low));
    let rise = tau * libm::log((5.0 - low) / 2.5);
    let crossings: Vec<f64> = edges
        .iter()
        .map(|&(edge, pulls)| edge as f64 + if pulls { fall } else { rise })
        .collect();
    assert_eq!(seen.len, 4, "{:?}", &seen.pulses[..seen.len]);
    let mut since = 0.0;
    for (k, (&(t, level, width), &crossing)) in seen.pulses.iter().zip(&crossings).enumerate() {
        assert!(
            (t as f64 - crossing).abs() < NS as f64,
            "crossing {k} at {t} ps, the exponential gives {crossing} ps"
        );
        assert_eq!(level, k % 2 == 0, "crossing {k} ends a {level} level");
        assert!(
            (width as f64 - (crossing - since)).abs() < 2.0 * NS as f64,
            "level {k} held {width} ps"
        );
        since = crossing;
    }
    let last = seen.pulses[3].0;
    assert_eq!(
        &seen.idles[..seen.idled],
        &[last + 10 * US],
        "idle after the last rise"
    );
}
