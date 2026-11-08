//! The `spi_replay` chip behind the seam. Its pins hold their idle levels for the whole run, and
//! its core stops at each event's end, where it sends the event as a `Spi` record.

use std::sync::Arc;

use orbe_engine_core::link::{Record, RecordKind, Spi, SpiNets};
use orbe_engine_core::netlist::{PartId, SpiReplay, Value};
use orbe_engine_core::time::Time;

use super::{Chip, Drive, Output, Stage, Stop, Takes};

const VCC: usize = 0;
const GND: usize = 1;
const CS: usize = 2;
const SCLK: usize = 3;
const D0: usize = 4;

/// The outputs, by package index, in image order: `CS`, `SCLK`, then `D0` to `D3`.
const OUTPUTS: [usize; 6] = [CS, SCLK, D0, D0 + 1, D0 + 2, D0 + 3];

/// SPI mode 0's idle levels: `CS` high, `SCLK` and the data lanes low.
const IDLE: [Drive; 6] = [
    Drive::High,
    Drive::Low,
    Drive::Low,
    Drive::Low,
    Drive::Low,
    Drive::Low,
];

/// The pin pairs the chip joins at DC, for `net.floating`: each output to `VCC` and to `GND`,
/// one of which its drive always stamps.
pub(crate) const DC_PATHS: [(usize, usize); 12] = {
    let mut paths = [(0, 0); 12];
    let mut k = 0;
    while k < OUTPUTS.len() {
        paths[2 * k] = (OUTPUTS[k], VCC);
        paths[2 * k + 1] = (OUTPUTS[k], GND);
        k += 1;
    }
    paths
};

pub(crate) fn chip(part: PartId, replay: &SpiReplay) -> Chip {
    let pins = replay.pins;
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let high = stage(&replay.output.high_drop, &replay.output.high_resistance);
    let low = stage(&replay.output.low_drop, &replay.output.low_resistance);
    let outputs = OUTPUTS
        .iter()
        .map(|&k| Output {
            pin: pins[k],
            vcc: pins[VCC],
            gnd: pins[GND],
            high,
            low,
            pull_up: None,
            leakage: None,
        })
        .collect();
    let nets = SpiNets {
        cs: Some(pins[CS]),
        sclk: Some(pins[SCLK]),
        data: [D0, D0 + 1, D0 + 2, D0 + 3].map(|k| Some(pins[k])),
    };
    let clock_hz = u32::try_from(replay.clock_hz.value).expect("parse keeps the clock in u32");
    let records = (0u64..)
        .zip(&replay.events)
        .map(|(seq, event)| Record {
            seq,
            start: event.at,
            end: event.end,
            pin: SpiReplay::PINS[CS],
            kind: RecordKind::Spi(Spi {
                phases: event.phases.clone(),
                lanes: event.lanes,
                data: event.data.clone(),
                clock_hz,
                cs: event.cs,
                nets,
            }),
        })
        .collect();
    Chip {
        part: Some(part),
        outputs,
        watches: Vec::new(),
        signals: Vec::new(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: true,
        takes: Takes::default(),
        core: super::Core::SpiReplay(Core {
            records,
            made: 0,
            sent: 0,
        }),
    }
}

/// The events as records, and how far it has walked them. Every stop is a record's end, so no
/// stop changes the image.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Core {
    records: Arc<[Record]>,
    /// The records before this one have ended where the core reached.
    made: usize,
    /// The records before this one have left the core.
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
        &IDLE
    }

    /// It watches nothing, so it takes only the power-on input, which has no level.
    pub(crate) fn input(&mut self, _at: Time, levels: &[bool]) {
        assert!(levels.is_empty(), "a replay watches no pin");
    }

    pub(crate) fn records(&self) -> &[Record] {
        &self.records[self.sent..self.made]
    }

    pub(crate) fn drain_records(&mut self, through: Time, mut sent: impl FnMut(Record)) {
        while self.sent < self.made && self.records[self.sent].end <= through {
            sent(self.records[self.sent].clone());
            self.sent += 1;
        }
    }
}
