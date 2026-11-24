//! The table of microcontroller kinds. Each chip crate runs its chip on the `orbe-mcu`
//! contract, and the engine keeps what the circuit sees: each kind's netlist part and the
//! outputs, watches and samples it builds from it, the net each output sits on, and the records
//! a chip sent until a commit stages them. [docs/engine/mcu.md](../../../../docs/engine/mcu.md)
//! says what a chip implements.
//!
//! [`Mcu`] holds one chip of any kind in the table. Its calls dispatch with a `match` on the
//! kind, so no call on a chip goes through a `dyn`.

pub(crate) mod atmega328p;
pub(crate) mod esp32s3;
mod report;

use std::sync::Arc;

use orbe_mcu::{Frame, Mcu as _, Note, Phase, Sent, SentKind};
use orbe_model::{Drive, Readings, Stop, Time};

use crate::active::{Chip, Output, Stage, Takes};
use crate::link::{Record, RecordKind, SpiNets, SpiPinNames};
use crate::mna::Node;
use crate::netlist::{NetId, PartId, Threshold};
use crate::output_stage::OutputStage;
use crate::watch::{Signal, Watch};

pub use atmega328p::{Atmega328p, Atmega328pRatings, Firmware, Fuses};
pub use esp32s3::{Esp32s3, Esp32s3Firmware};
pub(crate) use report::{fault, note, symbolize};

/// A chip, and the net of each of its outputs and the records it sent that it still holds.
#[derive(Debug)]
pub(crate) struct Host<M> {
    pub(crate) mcu: M,
    /// Shared with every copy, so a copy copies a pointer.
    nets: Arc<[NetId]>,
    /// The pin name of each output, in the same order as `nets`.
    pins: Arc<[&'static str]>,
    /// In the order it sent them.
    records: Vec<Record>,
    /// How many records it has made.
    made: u64,
}

impl<M: Clone> Clone for Host<M> {
    fn clone(&self) -> Self {
        Host {
            mcu: self.mcu.clone(),
            nets: Arc::clone(&self.nets),
            pins: Arc::clone(&self.pins),
            records: self.records.clone(),
            made: self.made,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.mcu.clone_from(&source.mcu);
        if !Arc::ptr_eq(&self.nets, &source.nets) {
            self.nets = Arc::clone(&source.nets);
        }
        if !Arc::ptr_eq(&self.pins, &source.pins) {
            self.pins = Arc::clone(&source.pins);
        }
        self.records.clone_from(&source.records);
        self.made = source.made;
    }
}

impl<M: orbe_mcu::Mcu> Host<M> {
    /// Takes what the chip sent at a stop with `record` set, as records on its outputs' nets.
    #[inline]
    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let stop = self.mcu.run_until(limit)?;
        if M::SENDS && stop.record {
            let Host {
                mcu,
                nets,
                pins,
                records,
                made,
            } = self;
            mcu.take_sent(|sent| {
                records.push(record(nets, pins, sent, *made));
                *made += 1;
            });
        }
        Some(stop)
    }

    fn sample(&mut self, world: &crate::world::World, at: Time, volts: &[f64]) {
        let readings = M::WORLD.iter().fold(Readings::NONE, |readings, &quantity| {
            match world.before(quantity, at) {
                Some(value) => readings.with(quantity, value),
                None => readings,
            }
        });
        self.mcu.sample(at, volts, &readings);
    }

    fn drain_records(&mut self, through: Time, sent: impl FnMut(Record)) {
        let ended = self.records.partition_point(|record| record.end <= through);
        self.records.drain(..ended).for_each(sent);
    }

    fn in_flight(&self, ahead: &Self, at: Time) -> Vec<Record> {
        self.mcu
            .in_flight(&ahead.mcu, at)
            .into_iter()
            .map(|sent| record(&self.nets, &self.pins, sent, 0))
            .collect()
    }
}

/// `sent` as the `seq`th record of its chip, on the nets of the outputs it went out on.
fn record(nets: &[NetId], output_pins: &[&'static str], sent: Sent, seq: u64) -> Record {
    let net = |output: Option<usize>| output.map(|k| nets[k]);
    let pin = |output: Option<usize>| output.map(|k| output_pins[k]);
    Record {
        seq,
        start: sent.start,
        end: sent.end,
        peripheral: sent.peripheral,
        pin: sent.pin,
        kind: match sent.kind {
            SentKind::Spi { spi, pins } => RecordKind::Spi {
                spi,
                nets: SpiNets {
                    cs: net(pins.cs),
                    sclk: net(pins.sclk),
                    data: pins.data.map(net),
                },
                pins: SpiPinNames {
                    cs: pin(pins.cs),
                    sclk: pin(pins.sclk),
                    data: pins.data.map(pin),
                },
            },
            SentKind::Pulses { pulses, output } => RecordKind::Pulses {
                pulses,
                net: nets[output],
            },
            SentKind::Duty { duty, output } => RecordKind::Duty {
                duty,
                net: nets[output],
            },
        },
    }
}

/// A chip's pads as its part wires them: what every pad shares, and each pad.
pub(crate) struct Pads<'a> {
    pub(crate) vcc: NetId,
    pub(crate) gnd: NetId,
    pub(crate) output: &'a OutputStage,
    /// Ohm. Every pad's to `vcc` and again to `gnd`, in every drive state.
    pub(crate) leakage: f64,
    pub(crate) pads: Vec<Pad<'a>>,
}

/// One pad with a net.
pub(crate) struct Pad<'a> {
    pub(crate) net: NetId,
    /// Ohm.
    pub(crate) pull_up: f64,
    /// Ohm. `None` on a pad that never pulls down.
    pub(crate) pull_down: Option<f64>,
    /// The fractions of its supply it reads a level at.
    pub(crate) threshold: &'a Threshold,
}

impl Pads<'_> {
    /// One output and one watch per pad, in its order. A pad drives through the output stage
    /// from `vcc` and into `gnd`, and reads high above `threshold.high` of its supply and low
    /// below `threshold.low`.
    pub(crate) fn build(&self) -> (Vec<Output>, Vec<Watch>) {
        let stage = |drop: &crate::netlist::Value, resistance: &crate::netlist::Value| Stage {
            drop: drop.si.get(),
            resistance: resistance.si.get(),
        };
        let high = stage(&self.output.high_drop, &self.output.high_resistance);
        let low = stage(&self.output.low_drop, &self.output.low_resistance);
        let outputs = self
            .pads
            .iter()
            .map(|pad| Output {
                pin: pad.net,
                vcc: self.vcc,
                gnd: self.gnd,
                high,
                low,
                pull_up: Some(pad.pull_up),
                pull_down: pad.pull_down,
                leakage: Some(self.leakage),
            })
            .collect();
        // V(pin) - f * V(VCC) - (1 - f) * V(GND): the pin against a fraction of its own supply.
        let against = |net, fraction: f64| Signal {
            terms: vec![
                (Node::from(net), 1.0),
                (Node::from(self.vcc), -fraction),
                (Node::from(self.gnd), fraction - 1.0),
            ],
            offset: 0.0,
        };
        let watches = self
            .pads
            .iter()
            .map(|pad| Watch {
                rise: against(pad.net, pad.threshold.high.si.get()),
                fall: against(pad.net, pad.threshold.low.si.get()),
            })
            .collect();
        (outputs, watches)
    }
}

/// The active chip of an MCU part: what the circuit reads, from its part, and `mcu` at
/// power-on, whose outputs are `outputs` in order.
pub(crate) fn chip<M: orbe_mcu::Mcu>(
    part: PartId,
    outputs: Vec<Output>,
    output_pins: Vec<&'static str>,
    watches: Vec<Watch>,
    signals: Vec<(&'static str, usize)>,
    samples: Vec<Signal>,
    mcu: M,
) -> Chip
where
    Mcu: From<Host<M>>,
{
    debug_assert_eq!(outputs.len(), output_pins.len());
    let nets = outputs.iter().map(|output| output.pin).collect();
    let host = Host {
        mcu,
        nets,
        pins: output_pins.into(),
        records: Vec::new(),
        made: 0,
    };
    Chip {
        part: Some(part),
        outputs,
        watches,
        signals,
        samples,
        usarts: M::USARTS,
        twi: M::TWI,
        sends: M::SENDS,
        supply: None,
        draw: None,
        takes: Takes::default(),
        core: crate::active::Core::Mcu(Mcu::from(host)),
    }
}

/// The table: one row per kind, its variant and its chip crate's type on the contract. The `$`
/// lets the table define `each`, which runs one expression on the chip of whichever kind an
/// `Mcu` holds.
macro_rules! table {
    ($d:tt $($kind:ident($mcu:ty),)*) => {
        /// A chip of one of the table's kinds.
        #[derive(Debug)]
        pub(crate) enum Mcu {
            $($kind(Box<Host<$mcu>>),)*
        }

        $(impl From<Host<$mcu>> for Mcu {
            fn from(host: Host<$mcu>) -> Mcu {
                Mcu::$kind(Box::new(host))
            }
        })*

        impl Clone for Mcu {
            fn clone(&self) -> Self {
                match self {
                    $(Mcu::$kind(host) => Mcu::$kind(host.clone()),)*
                }
            }

            /// Reuses the buffers of a chip of the same kind.
            fn clone_from(&mut self, source: &Self) {
                match (self, source) {
                    $((Mcu::$kind(host), Mcu::$kind(source)) => host.clone_from(source),)*
                    (host, source) => *host = source.clone(),
                }
            }
        }

        impl Mcu {
            /// The records it started by `at` and has not ended, where the run ends at `at`.
            /// `ahead` is its copy run on from here.
            pub(crate) fn in_flight(&self, ahead: &Mcu, at: Time) -> Vec<Record> {
                match (self, ahead) {
                    $((Mcu::$kind(host), Mcu::$kind(ahead)) => host.in_flight(ahead, at),)*
                    _ => unreachable!("a copy is of its chip's kind"),
                }
            }
        }

        /// `$body` with `$host` bound to the `Host` that `$mcu` holds.
        macro_rules! each {
            ($d mcu:expr, $d host:ident => $d body:expr) => {
                match $d mcu {
                    $(Mcu::$kind($d host) => $d body,)*
                }
            };
        }
    };
}

table! { $
    Atmega328p(orbe_microchip::Bridge),
    Esp32s3(orbe_espressif::s3::Bridge),
}

/// The calls `active::Core` makes, each on the chip's own type.
impl Mcu {
    #[inline]
    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        each!(self, host => host.run_until(limit))
    }

    #[inline]
    pub(crate) fn drive(&self) -> &[Drive] {
        each!(self, host => host.mcu.drive())
    }

    #[inline]
    pub(crate) fn pulled_down(&self) -> &[bool] {
        each!(self, host => host.mcu.pulled_down())
    }

    #[inline]
    pub(crate) fn held(&self) -> bool {
        each!(self, host => host.mcu.held())
    }

    /// The volts of its samples at `at`, and each world quantity it reads that the scene
    /// declares.
    pub(crate) fn sample(&mut self, world: &crate::world::World, at: Time, volts: &[f64]) {
        each!(self, host => host.sample(world, at, volts))
    }

    pub(crate) fn input(&mut self, at: Time, levels: &[bool]) {
        each!(self, host => host.mcu.input(at, levels))
    }

    pub(crate) fn deliver(
        &mut self,
        through: Time,
        sent: impl FnMut(Frame),
        saw: impl FnMut(Phase),
    ) {
        each!(self, host => host.mcu.deliver(through, sent, saw))
    }

    pub(crate) fn answered(&mut self, at: Time, miso: &[u8]) {
        each!(self, host => host.mcu.answered(at, miso))
    }

    pub(crate) fn records(&self) -> &[Record] {
        each!(self, host => &host.records)
    }

    pub(crate) fn drain_records(&mut self, through: Time, sent: impl FnMut(Record)) {
        each!(self, host => host.drain_records(through, sent))
    }

    pub(crate) fn fault(&self) -> Option<(Time, orbe_mcu::Fault)> {
        each!(self, host => host.mcu.fault())
    }

    pub(crate) fn notes(&self, through: Time) -> Vec<(Time, Note)> {
        each!(self, host => host.mcu.notes(through))
    }

    pub(crate) fn read(&self, output: usize) -> Option<Time> {
        each!(self, host => host.mcu.read(output))
    }
}
