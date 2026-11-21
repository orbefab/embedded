//! The sensor through its host core, as the engine runs it, in a parsed netlist's world. A
//! host pulls `DATA` low and releases it, and the test decodes the reply from the drive the
//! core plays on `DATA`, as a host reading the line would.

use orbe_model::{Device, Drive, Fields, Time};

use super::model::{self, Dht22};
use crate::active::native::{Hosted, Native};
use crate::active::pulse::Pulse;
use crate::diagnostic::ProtocolRule;
use crate::world::World;

const US: u64 = 1_000_000;
const MS: u64 = 1_000 * US;
const S: u64 = 1_000 * MS;

fn level(value: f64, unit: &str) -> String {
    format!(r#"{{ "value": {value:?}, "unit": "{unit}" }}"#)
}

/// `temperature` from `initial` C, and to `to` C at `at` ps when given. Humidity holds `rh`.
fn world(initial: f64, step: Option<(u64, f64)>, rh: f64) -> World {
    let temperature = match step {
        None => level(initial, "degC"),
        Some((at, to)) => format!(
            r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": {at}, "level": {} }}] }}"#,
            level(initial, "degC"),
            level(to, "degC")
        ),
    };
    let json = format!(
        r#"{{
          "version": 1, "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": 10000000000000, "max_step_ps": 1000000000, "grid_ps": 1000000000 }} }},
          "ground": "gnd", "nets": ["gnd", "vcc", "data"],
          "world": {{ "temperature": {temperature}, "humidity": {} }},
          "parts": [{{ "name": "u1", "kind": "dht22", "pins": {{ "VCC": "vcc", "DATA": "data", "GND": "gnd" }},
            "low_resistance": {}, "input_threshold": {{ "low": {}, "high": {} }} }}]
        }}"#,
        level(rh, "%RH"),
        level(100.0, "ohm"),
        level(0.3, "1"),
        level(0.7, "1"),
    );
    crate::netlist::parse(json.as_bytes())
        .expect("a dht22 netlist")
        .world()
        .clone()
}

/// A host on `DATA` and the sensor's core: the line's level as the host sees it, with no
/// analog delay, and every drive edge the core played.
struct Host {
    core: Native<Dht22, Pulse<Dht22>>,
    world: World,
    /// Each time the core's drive on `DATA` changed, and whether it pulls low after.
    edges: Vec<(u64, bool)>,
}

impl Host {
    fn new(world: World) -> Host {
        let mut core = Native::new(Dht22::new(&Fields::NONE).unwrap(), Pulse::new(), 0);
        core.input(&world, Time::ZERO, &[true]);
        Host {
            core,
            world,
            edges: Vec::new(),
        }
    }

    /// Runs the core to `limit`, landing each stop.
    fn run(&mut self, limit: u64) {
        while let Some(stop) = self.core.run_until(&self.world, Time::from_ps(limit)) {
            self.edges
                .push((stop.at.as_ps(), self.core.drive()[0] == Drive::Low));
        }
    }

    /// The host pulls `DATA` low at `at` for `low` ps and releases it.
    fn start(&mut self, at: u64, low: u64) {
        self.run(at);
        self.core.input(&self.world, Time::from_ps(at), &[false]);
        self.run(at + low);
        self.core
            .input(&self.world, Time::from_ps(at + low), &[true]);
    }

    fn broke(&self) -> Vec<(ProtocolRule, u64)> {
        self.core
            .broke()
            .into_iter()
            .map(|(rule, at)| (rule, at.as_ps()))
            .collect()
    }
}

/// One decoded reply: its first edge's time after the host's release, the response's low and
/// high, the 40 bits, and the last low.
#[derive(Debug, PartialEq)]
struct Reply {
    go: u64,
    response: (u64, u64),
    bytes: [u8; 5],
    end_low: u64,
}

/// The reply in `edges` from the host's release at `released`. A bit's high over 48 us is a 1,
/// halfway between the sheet's 26 us and 70 us.
fn decode(edges: &[(u64, bool)], released: u64) -> Reply {
    let edges: Vec<(u64, bool)> = edges
        .iter()
        .copied()
        .filter(|&(at, _)| at > released)
        .collect();
    assert_eq!(edges.len(), 2 + 80 + 2, "{edges:?}");
    let spans: Vec<u64> = edges.windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(edges.iter().step_by(2).all(|&(_, low)| low), "{edges:?}");
    assert!(
        edges.iter().skip(1).step_by(2).all(|&(_, low)| !low),
        "{edges:?}"
    );
    let mut bytes = [0u8; 5];
    for bit in 0..40 {
        assert_eq!(spans[2 + 2 * bit], model::BIT_LOW_PS, "bit {bit}'s low");
        let high = spans[3 + 2 * bit];
        assert!(
            high == model::ZERO_HIGH_PS || high == model::ONE_HIGH_PS,
            "bit {bit}'s high {high}"
        );
        bytes[bit / 8] |= u8::from(high > 48 * US) << (7 - bit % 8);
    }
    Reply {
        go: edges[0].0 - released,
        response: (spans[0], spans[1]),
        bytes,
        end_low: spans[82],
    }
}

/// The sheet's example 1 is 65.8 %RH and 26.9 C: 0x0292, 0x010D and checksum 0xA2 (AM2302
/// manual v1.1, table 5).
#[test]
fn a_read_answers_with_the_sheets_bytes_and_timing() {
    let mut host = Host::new(world(26.9, None, 65.8));
    host.start(2 * S, MS);
    host.run(3 * S);
    let reply = decode(&host.edges, 2 * S + MS);
    assert_eq!(
        reply,
        Reply {
            go: 30 * US,
            response: (80 * US, 80 * US),
            bytes: [0x02, 0x92, 0x01, 0x0D, 0xA2],
            end_low: 50 * US,
        }
    );
    assert!(host.broke().is_empty());
}

/// The sheet's -10.1 C sets the sign bit over 101: 0x8065 (AM2302 manual v1.1, after table 5).
/// 0 %RH is 0x0000, so the checksum is 0x80 + 0x65.
#[test]
fn a_temperature_below_zero_sets_the_top_bit() {
    let mut host = Host::new(world(-10.1, None, 0.0));
    host.start(2 * S, MS);
    host.run(3 * S);
    assert_eq!(
        decode(&host.edges, 2 * S + MS).bytes,
        [0x00, 0x00, 0x80, 0x65, 0xE5]
    );
}

/// Each read sends what the sensor measured at the end of the read before (the note under the
/// manual's figure 6). The world goes from 25 C to 30 C at 3 s. The read at 2 s sends the
/// warm-up's 25 C, and the read at 4 s the 25 C measured as the 2 s read ended. Only the read
/// at 6 s sends 30 C, measured as the 4 s read ended.
#[test]
fn a_read_sends_the_measurement_the_read_before_took() {
    let mut host = Host::new(world(25.0, Some((3 * S, 30.0)), 50.0));
    let mut temperatures = Vec::new();
    for at in [2 * S, 4 * S, 6 * S] {
        host.start(at, MS);
        host.run(at + 100 * MS);
        let bytes = decode(&host.edges, at + MS).bytes;
        temperatures.push(u16::from_be_bytes([bytes[2], bytes[3]]));
        host.edges.clear();
    }
    assert_eq!(temperatures, [250, 250, 300]);
}

/// The engine runs a copy of each core ahead, and when a landing comes first it copies the
/// committed core over that copy and runs it to the landing. A copy rewound to its commit at
/// the host's release, before the reply's first edge, or at 200 us or 1 ms into the reply, plays
/// the edges the run that was never rewound plays.
#[test]
fn a_copy_rewound_inside_a_reply_plays_the_same_edges() {
    let released = 2 * S + MS;
    let mut straight = Host::new(world(26.9, None, 65.8));
    straight.start(2 * S, MS);
    straight.run(3 * S);
    assert_eq!(straight.edges.len(), 84);
    for commit in [released, released + 200 * US, released + MS] {
        let mut host = Host::new(world(26.9, None, 65.8));
        host.start(2 * S, MS);
        host.run(commit);
        let committed = host.core.clone();
        host.run(3 * S);
        host.core.clone_from(&committed);
        host.edges.retain(|&(at, _)| at <= commit);
        host.run(3 * S);
        assert_eq!(host.edges, straight.edges, "committed at {commit} ps");
    }
}

/// A start inside the 2 s warm-up, or within 2 s of the last start the sensor answered, gets no
/// reply and breaks rule `interval` there. A low under 0.8 ms is no start.
#[test]
fn a_start_too_soon_or_too_short_gets_no_reply() {
    let mut host = Host::new(world(25.0, None, 50.0));
    host.start(S, MS);
    host.run(2 * S);
    assert!(host.edges.is_empty(), "{:?}", host.edges);
    assert_eq!(host.broke(), [(ProtocolRule::Interval, S + MS)]);

    host.start(2 * S, 700 * US);
    host.run(2 * S + 100 * MS);
    assert!(host.edges.is_empty(), "a 0.7 ms low: {:?}", host.edges);

    host.start(3 * S, MS);
    host.run(3 * S + 100 * MS);
    assert_eq!(host.edges.len(), 84, "the first read");
    host.edges.clear();
    host.start(4 * S, MS);
    host.run(5 * S);
    assert!(
        host.edges.is_empty(),
        "1 s after the last: {:?}",
        host.edges
    );
    host.start(5 * S + MS, MS);
    host.run(6 * S);
    assert_eq!(host.edges.len(), 84, "2 s after the last");
}
