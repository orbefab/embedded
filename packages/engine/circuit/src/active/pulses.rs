//! The `pulses_replay` chip behind the seam. `OUT` holds `idle` for the whole run, and the
//! core stops at each event's end, where it sends the event as a `Pulses` record.

use std::sync::Arc;

use orbe_engine_core::link::{Pulses, Record, RecordKind};
use orbe_engine_core::netlist::{PartId, PulsesReplay, Value};
use orbe_engine_core::time::Time;

use super::{Chip, Drive, Output, Stage, Stop, Takes};

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
        core: super::Core::PulsesReplay(Core {
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
