//! A test-only active chip: a script of drives and samples on one output pin, `out`, and one
//! input pin, `in`. A drive at 0 is its power-on drive. A script can also send records and
//! console bytes, take records, and log what it took.

use orbe_mcu::Frame;
use orbe_model::{Cs, Drive, Duty, Lanes, Spi, Stop, Time};

use crate::netlist::NetId;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::link::{LinkRow, Record, RecordKind, SpiNets};
use crate::name::Name;

use super::{Chip, Output, SpiTarget, Stage, Takes};
use crate::mna::Node;
use crate::watch::{Signal, Watch};

/// What the script does at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Op {
    Drive(Drive),
    Sample(Sample),
    /// A `Spi` record that starts and ends here, whose one data byte is the tag.
    Send(u8),
    /// A `Duty` record of this many of 1024 steps at 5 kHz on `out`'s net, which starts and
    /// ends here.
    Duty(u32),
    /// `count` console bytes from `first` up, which all start and end here.
    Bytes {
        first: u8,
        count: u8,
    },
}

/// One record a script took: when, its data and its sequence.
pub(crate) type Taken = (Time, Vec<u8>, u64);

/// How many sends every copy of one script made, runs again included. Every copy shares it, so
/// it is no part of a core's state, and two cores equal on everything else are equal.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sends(Arc<AtomicU64>);

impl Sends {
    pub(crate) fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl PartialEq for Sends {
    fn eq(&self, _: &Sends) -> bool {
        true
    }
}

/// Sample `in`. `after` ps later drive `out` high if it read above `threshold`, otherwise low.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    pub(crate) after: u64,
    pub(crate) threshold: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Core {
    script: Arc<[(Time, Op)]>,
    /// `out` follows `in`'s level this long after it takes it. `None` ignores `in`.
    echo: Option<u64>,
    /// Drives `out` at a read itself, which breaks the contract, so a test sees the seam catch it.
    eager: bool,
    /// Everything up to here is done.
    now: Time,
    /// The next op.
    cursor: usize,
    /// Drives it set itself, by time, in the order it set them.
    pending: Vec<(Time, Drive)>,
    sampling: Option<Sample>,
    drive: [Drive; 1],
    /// Whether `out` pulls down. A script that does not reports none.
    pulled: [bool; 1],
    /// The part name a sender or a console sorts by.
    name: Option<&'static str>,
    /// The nets its records name.
    bus: Option<SpiTarget>,
    /// The net of `out`, which its `Duty` records name.
    out: NetId,
    /// How many records it has sent.
    sent: u64,
    sends: Sends,
    /// The records it sent that it still holds.
    records: Vec<Record>,
    /// The console bytes it sent that it still holds, each with its time.
    bytes: Vec<(Time, u8)>,
    taken: Vec<Taken>,
}

pub(crate) fn chip(
    (vcc, gnd, out, input): (NetId, NetId, NetId, NetId),
    script: Vec<(Time, Op)>,
    echo: Option<u64>,
) -> Chip {
    let stage = Stage {
        drop: 0.0,
        resistance: 25.0,
    };
    let v_in = Signal {
        terms: vec![(Node::from(input), 1.0)],
        offset: 0.0,
    };
    Chip {
        part: None,
        outputs: vec![Output {
            pin: out,
            vcc,
            gnd,
            high: stage,
            low: stage,
            pull_up: None,
            pull_down: None,
            leakage: None,
        }],
        watches: vec![Watch::comparator(Signal {
            offset: 2.5,
            ..v_in.clone()
        })],
        signals: Vec::new(),
        samples: vec![v_in],
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: None,
        takes: Takes::default(),
        core: super::Core::Script(Box::new(Core::new(script, echo, out))),
    }
}

fn script(chip: &mut Chip) -> &mut Core {
    let super::Core::Script(core) = &mut chip.core else {
        unreachable!("a script chip");
    };
    core
}

/// `chip` sending as part `name`, its records naming `bus`'s CS and SCLK.
pub(crate) fn sender(mut chip: Chip, name: &'static str, bus: SpiTarget) -> Chip {
    chip.sends = true;
    let core = script(&mut chip);
    core.name = Some(name);
    core.bus = Some(bus);
    chip
}

/// The send count every copy of `chip`'s core shares.
pub(crate) fn sends(chip: &mut Chip) -> Sends {
    script(chip).sends.clone()
}

/// `chip` taking each `Spi` record sent on `bus`.
pub(crate) fn receiver(mut chip: Chip, bus: SpiTarget) -> Chip {
    chip.takes.spi = Some(bus);
    chip
}

/// `chip` sending its console bytes as part `name`, on one USART.
pub(crate) fn console(mut chip: Chip, name: &'static str) -> Chip {
    chip.usarts = &["USB_SERIAL_JTAG"];
    script(&mut chip).name = Some(name);
    chip
}

impl Core {
    fn new(script: Vec<(Time, Op)>, echo: Option<u64>, out: NetId) -> Core {
        assert!(script.is_sorted_by_key(|&(at, _)| at));
        let mut core = Core {
            script: script.into(),
            echo,
            now: Time::ZERO,
            cursor: 0,
            pending: Vec::new(),
            sampling: None,
            eager: false,
            drive: [Drive::Off],
            pulled: [false; 1],
            name: None,
            bus: None,
            out,
            sent: 0,
            sends: Sends::default(),
            records: Vec::new(),
            bytes: Vec::new(),
            taken: Vec::new(),
        };
        while let Some(&(Time::ZERO, op)) = core.script.get(core.cursor) {
            let Op::Drive(drive) = op else {
                panic!("a sample at 0 has no step to land on");
            };
            core.drive = [drive];
            core.cursor += 1;
        }
        core
    }

    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        assert!(limit >= self.now, "a core runs only past where it reached");
        let scripted = self.script.get(self.cursor).map(|&(at, _)| at);
        let pending = self.pending.iter().map(|&(at, _)| at).min();
        let Some(at) = scripted
            .into_iter()
            .chain(pending)
            .min()
            .filter(|&at| at <= limit)
        else {
            self.now = limit;
            return None;
        };
        self.now = at;
        let mut record = false;
        while let Some(&(when, op)) = self.script.get(self.cursor)
            && when == at
        {
            match op {
                Op::Drive(drive) => self.drive = [drive],
                Op::Sample(sample) => self.sampling = Some(sample),
                Op::Send(tag) => {
                    let bus = self.bus.expect("a sender has a bus");
                    self.send(at, spi(bus, tag));
                    record = true;
                }
                Op::Duty(duty) => {
                    let duty = Duty {
                        duty,
                        resolution: 10,
                        hz: 5000,
                    };
                    self.send(
                        at,
                        RecordKind::Duty {
                            duty,
                            net: self.out,
                        },
                    );
                    record = true;
                }
                Op::Bytes { first, count } => {
                    self.bytes
                        .extend((first..first + count).map(|byte| (at, byte)));
                }
            }
            self.cursor += 1;
        }
        while let Some(k) = self.pending.iter().position(|&(when, _)| when == at) {
            self.drive = [self.pending.remove(k).1];
        }
        Some(Stop {
            at,
            sample: self.sampling.is_some(),
            record,
        })
    }

    fn send(&mut self, at: Time, kind: RecordKind) {
        self.records.push(Record {
            seq: self.sent,
            start: at,
            end: at,
            pin: "CS",
            kind,
        });
        self.sent += 1;
        self.sends.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Everything up to here is done.
    pub(crate) fn now(&self) -> Time {
        self.now
    }

    pub(crate) fn name(&self) -> Name {
        Name::new(self.name.expect("a named script")).expect("a script's name is a name")
    }

    pub(crate) fn records(&self) -> &[Record] {
        &self.records
    }

    pub(crate) fn drain_records(&mut self, through: Time, mut sent: impl FnMut(Record)) {
        let held = self.records.partition_point(|record| record.end <= through);
        self.records.drain(..held).for_each(&mut sent);
    }

    pub(crate) fn deliver(&mut self, through: Time, mut sent: impl FnMut(Frame)) {
        let held = self.bytes.partition_point(|&(at, _)| at <= through);
        for (at, byte) in self.bytes.drain(..held) {
            sent(Frame {
                usart: 0,
                byte,
                start: at,
                end: at,
            });
        }
    }

    pub(crate) fn take(&mut self, at: Time, records: &[LinkRow]) {
        assert_eq!(at, self.now, "a record arrives where the core is");
        for row in records {
            let RecordKind::Spi { spi, .. } = &row.record.kind else {
                unreachable!("a script takes only SPI");
            };
            self.taken.push((at, spi.data.to_vec(), row.record.seq));
        }
    }

    /// Each record it took, in the order it took them.
    pub(crate) fn taken(&self) -> &[Taken] {
        &self.taken
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        &self.drive
    }

    /// `out`'s pull-down, or empty when it does not pull down.
    pub(crate) fn pulled_down(&self) -> &[bool] {
        if self.pulled[0] { &self.pulled } else { &[] }
    }

    pub(crate) fn sample(&mut self, at: Time, volts: &[f64]) {
        assert_eq!(at, self.now, "a sample arrives where the core is");
        let Sample { after, threshold } = self.sampling.take().expect("a stop named the sample");
        let drive = if volts[0] > threshold {
            Drive::High
        } else {
            Drive::Low
        };
        self.act(at, after, drive);
    }

    pub(crate) fn input(&mut self, at: Time, levels: &[bool]) {
        assert_eq!(at, self.now, "an input arrives where the core is");
        if let Some(echo) = self.echo {
            let drive = if levels[0] { Drive::High } else { Drive::Low };
            self.act(at, echo, drive);
        }
    }

    fn act(&mut self, at: Time, after: u64, drive: Drive) {
        if self.eager {
            self.drive = [drive];
        } else {
            self.pending
                .push((Time::from_ps(at.as_ps() + after), drive));
        }
    }
}

/// One byte, `tag`, on `bus`, on a lane the script names no pin for.
fn spi(bus: SpiTarget, tag: u8) -> RecordKind {
    RecordKind::Spi {
        spi: Spi {
            phases: Vec::new(),
            lanes: Lanes::One,
            data: vec![tag].into(),
            clock_hz: 1,
            cs: Cs::Released,
        },
        nets: SpiNets {
            cs: Some(bus.cs),
            sclk: Some(bus.sclk),
            data: [None; 4],
        },
    }
}

pub(crate) fn eager(mut chip: Chip) -> Chip {
    script(&mut chip).eager = true;
    chip
}

/// Gives `out` its pull resistors, in ohm. A pull-down makes the core report one from the start.
pub(crate) fn pulls(mut chip: Chip, up: Option<f64>, down: Option<f64>) -> Chip {
    let output = &mut chip.outputs[0];
    output.pull_up = up;
    output.pull_down = down;
    script(&mut chip).pulled[0] = down.is_some();
    chip
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> Core {
        let script = vec![
            (Time::from_ps(10), Op::Drive(Drive::High)),
            (
                Time::from_ps(20),
                Op::Sample(Sample {
                    after: 5,
                    threshold: 1.0,
                }),
            ),
            (Time::from_ps(40), Op::Drive(Drive::Off)),
        ];
        Core::new(script, None, NetId::GROUND)
    }

    #[test]
    fn a_run_split_anywhere_is_the_same_run() {
        let whole = {
            let mut core = core();
            let mut stops = Vec::new();
            while let Some(stop) = core.run_until(Time::from_ps(50)) {
                stops.push(stop);
                if stop.sample {
                    core.sample(stop.at, &[2.0]);
                }
            }
            (core, stops)
        };
        for split in 0..50 {
            let mut core = core();
            let mut stops = Vec::new();
            for limit in [split, 50] {
                while let Some(stop) = core.run_until(Time::from_ps(limit)) {
                    stops.push(stop);
                    if stop.sample {
                        core.sample(stop.at, &[2.0]);
                    }
                }
            }
            assert_eq!((core, stops), whole, "split at {split} ps");
        }
    }
}
