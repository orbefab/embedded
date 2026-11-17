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
use orbe_model::{Drive, Stop, Time};

use crate::active::{Chip, Output, Takes};
use crate::link::{Record, RecordKind, SpiNets};
use crate::netlist::{NetId, PartId};
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
            records: self.records.clone(),
            made: self.made,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.mcu.clone_from(&source.mcu);
        if !Arc::ptr_eq(&self.nets, &source.nets) {
            self.nets = Arc::clone(&source.nets);
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
                records,
                made,
            } = self;
            mcu.take_sent(|sent| {
                records.push(record(nets, sent, *made));
                *made += 1;
            });
        }
        Some(stop)
    }

    fn drain_records(&mut self, through: Time, sent: impl FnMut(Record)) {
        let ended = self.records.partition_point(|record| record.end <= through);
        self.records.drain(..ended).for_each(sent);
    }

    fn in_flight(&self, ahead: &Self, at: Time) -> Vec<Record> {
        self.mcu
            .in_flight(&ahead.mcu, at)
            .into_iter()
            .map(|sent| record(&self.nets, sent, 0))
            .collect()
    }
}

/// `sent` as the `seq`th record of its chip, on the nets of the outputs it went out on.
fn record(nets: &[NetId], sent: Sent, seq: u64) -> Record {
    let net = |output: Option<usize>| output.map(|k| nets[k]);
    Record {
        seq,
        start: sent.start,
        end: sent.end,
        pin: sent.pin,
        kind: match sent.kind {
            SentKind::Spi { spi, pins } => RecordKind::Spi {
                spi,
                nets: SpiNets {
                    cs: net(pins.cs),
                    sclk: net(pins.sclk),
                    data: pins.data.map(net),
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

/// The active chip of an MCU part: what the circuit reads, from its part, and `mcu` at
/// power-on, whose outputs are `outputs` in order.
pub(crate) fn chip<M: orbe_mcu::Mcu>(
    part: PartId,
    outputs: Vec<Output>,
    watches: Vec<Watch>,
    signals: Vec<(&'static str, usize)>,
    samples: Vec<Signal>,
    mcu: M,
) -> Chip
where
    Mcu: From<Host<M>>,
{
    let nets = outputs.iter().map(|output| output.pin).collect();
    let host = Host {
        mcu,
        nets,
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
    pub(crate) fn held(&self) -> bool {
        each!(self, host => host.mcu.held())
    }

    pub(crate) fn sample(&mut self, at: Time, volts: &[f64]) {
        each!(self, host => host.mcu.sample(at, volts))
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
