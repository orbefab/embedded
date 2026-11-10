//! The `pulses_replay` chip behind the seam. `OUT` holds `idle` for the whole run, and the
//! core stops at each event's end, where it sends the event as a `Pulses` record.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::active::{Chip, Drive, Output, Stage, Stop, Takes};
use crate::diagnostic::{Diagnostic, ScheduleRule, ValueRule};
use crate::link::{Pulses, Record, RecordKind};
use crate::name::{Name, Ref};
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Value;
use crate::netlist::json::waveform::serial_schedule;
use crate::netlist::json::{PinMap, resolve_pins};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};
use crate::time::Time;

const VCC: usize = 0;
const GND: usize = 1;
const OUT: usize = 2;

const HIGH: [Drive; 1] = [Drive::High];
const LOW: [Drive; 1] = [Drive::Low];

/// `OUT` to `VCC` and to `GND`. One of the two is always stamped.
pub(crate) const DC_PATHS: [(usize, usize); 2] = [(OUT, VCC), (OUT, GND)];

pub(crate) fn chip(part: PartId, replay: &PulsesReplay) -> Chip {
    let pins = replay.pins;
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let output = Output {
        pin: pins[OUT],
        vcc: pins[VCC],
        gnd: pins[GND],
        high: stage(&replay.output.high_drop, &replay.output.high_resistance),
        low: stage(&replay.output.low_drop, &replay.output.low_resistance),
        pull_up: None,
        leakage: None,
    };
    let records = (0u64..)
        .zip(&replay.events)
        .map(|(seq, event)| Record {
            seq,
            start: event.at,
            end: event.end,
            pin: PulsesReplay::PINS[OUT],
            kind: RecordKind::Pulses(Pulses {
                idle: replay.idle,
                pulses: event.pulses.clone(),
                net: pins[OUT],
            }),
        })
        .collect();
    Chip {
        part: Some(part),
        outputs: vec![output],
        watches: Vec::new(),
        signals: Vec::new(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: true,
        takes: Takes::default(),
        core: crate::active::Core::PulsesReplay(Core {
            idle: replay.idle,
            records,
            made: 0,
            sent: 0,
        }),
    }
}

/// The events as records, and how far it has walked them. `OUT` stays at `idle`, so no stop
/// changes the image.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Core {
    idle: bool,
    records: Arc<[Record]>,
    made: usize,
    sent: usize,
}

impl Core {
    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let end = self.records.get(self.made)?.end;
        if end > limit {
            return None;
        }
        self.made += 1;
        Some(Stop {
            at: end,
            sample: false,
            record: true,
        })
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        if self.idle { &HIGH } else { &LOW }
    }

    pub(crate) fn input(&mut self, _at: Time, levels: &[bool]) {
        assert!(levels.is_empty(), "a replay watches no pin");
    }

    pub(crate) fn records(&self) -> &[Record] {
        &self.records[self.sent..self.made]
    }

    /// The record that started by `at` and ends after it, when its stops up to `at` are done.
    pub(crate) fn in_flight(&self, at: Time) -> Option<&Record> {
        let record = self.records.get(self.made)?;
        debug_assert!(record.end > at, "a record that ended by `at` was made");
        (record.start <= at).then_some(record)
    }

    pub(crate) fn drain_records(&mut self, through: Time, mut sent: impl FnMut(Record)) {
        while self.sent < self.made && self.records[self.sent].end <= through {
            sent(self.records[self.sent].clone());
            self.sent += 1;
        }
    }
}

/// A replay of pulse trains, an active chip with no clock of its own. It drives `OUT` at
/// `idle` through `output` from `VCC` and into `GND` for the whole run, and sends each of
/// `events` as a `Pulses` record at the event's end. Only parse builds one, so the events
/// start in order and none starts before the one before it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct PulsesReplay {
    pub pins: [NetId; 3],
    pub idle: bool,
    pub output: OutputStage,
    pub events: Vec<PulseTrain>,
}

/// One train of a `pulses_replay`, from `at` to `end`. `end` is `at` plus the pulse durations.
#[derive(Clone, Debug, PartialEq)]
pub struct PulseTrain {
    pub at: Time,
    pub end: Time,
    pub pulses: Arc<[(bool, u64)]>,
}

impl PulsesReplay {
    pub const PINS: [&'static str; 3] = ["VCC", "GND", "OUT"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WirePulsesReplay {
    pub(crate) name: String,
    pins: PinMap,
    idle: bool,
    output: WireOutputStage,
    events: Vec<WireEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvent {
    t_ps: u64,
    pulses: Vec<WirePulse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePulse {
    level: bool,
    ps: u64,
}

pub(crate) fn resolve_pulses_replay(
    name: &Name,
    part_ref: &Ref,
    wire: WirePulsesReplay,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<PulsesReplay> {
    let pins = resolve_pins(name, PulsesReplay::PINS, &wire.pins.0, nets, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let events = resolve_events(part_ref, wire.events, out);
    Some(PulsesReplay {
        pins: pins?,
        idle: wire.idle,
        output: output?,
        events: events?,
    })
}

fn resolve_events(
    part: &Ref,
    wire: Vec<WireEvent>,
    out: &mut Vec<Diagnostic>,
) -> Option<Vec<PulseTrain>> {
    let mut ok = true;
    let mut events = Vec::with_capacity(wire.len());
    let mut previous_start = None;
    let mut previous_end = None;
    for (index, event) in wire.into_iter().enumerate() {
        let at = event.t_ps;
        let mut pulses = Vec::with_capacity(event.pulses.len());
        for (pulse, wire) in event.pulses.into_iter().enumerate() {
            if wire.ps == 0 {
                ok = false;
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: format!("events[{index}].pulses[{pulse}].ps").into(),
                    value: 0.0,
                    rule: ValueRule::Positive,
                });
                continue;
            }
            pulses.push((wire.level, wire.ps));
        }
        let end = end_of(at, &pulses);
        let rule = serial_schedule(at, pulses.is_empty(), previous_start, previous_end, end);
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                field: "events",
                event: index,
                rule,
                end_ps: if rule == ScheduleRule::Overlap {
                    previous_end.unwrap_or(0)
                } else {
                    0
                },
            });
            if rule == ScheduleRule::Overflow {
                break;
            }
        }
        if let Some(end) = end
            && !pulses.is_empty()
        {
            events.push(PulseTrain {
                at: Time::from_ps(at),
                end: Time::from_ps(end),
                pulses: Arc::from(pulses),
            });
        }
        previous_start = Some(at);
        previous_end = end;
    }
    ok.then_some(events)
}

fn end_of(at: u64, pulses: &[(bool, u64)]) -> Option<u64> {
    let mut t = at;
    for &(_, ps) in pulses {
        t = t.checked_add(ps)?;
    }
    Some(t)
}
