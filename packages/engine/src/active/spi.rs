//! The host's SPI target engine. It takes each `Spi` record routed to its target, drops one while
//! the target is in reset or when a lane it used sits on another net than the target's data pin,
//! and for the rest asks [`SpiTarget::answer`] for each byte the record reads, then calls
//! [`SpiTarget::transaction`].
//!
//! One [`Hosted::take`](super::native::Hosted::take) enters per landing. The loop below then calls
//! the device on the concrete type, so a transaction's bytes and pixels never cross a `dyn` call.

use orbe_model::{Drive, Miso, Spi, SpiTarget, Time};

use super::native::{Hosting, Port};
use crate::link::{Answers, LinkRow, RecordKind, SpiNets};
use crate::netlist::NetId;
use crate::world::World;

/// A target's data pins and its reset watch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bus {
    /// The nets of D0 to D3.
    pub(crate) data: [NetId; 4],
    /// The watch of the target's reset pin, active low, when it has one.
    pub(crate) reset: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Engine {
    bus: Bus,
    /// The reset pin reads low.
    reset: bool,
    /// The first transaction it dropped for a lane on another net, and its sender.
    miswired: Option<(Time, u16)>,
}

impl Engine {
    pub(crate) const fn new(bus: Bus) -> Engine {
        Engine {
            bus,
            reset: false,
            miswired: None,
        }
    }

    /// Whether each lane the transaction uses sits on the target's own data pin. A read uses
    /// D1, MISO.
    fn wired(&self, spi: &Spi, nets: &SpiNets) -> bool {
        let read = if spi.miso == Miso::None { 1 } else { 2 };
        let used = spi
            .phases
            .iter()
            .map(|phase| phase.lanes.count())
            .chain([spi.lanes.count(), read])
            .max()
            .unwrap_or(1);
        (0..usize::from(used)).all(|k| nets.data[k] == Some(self.bus.data[k]))
    }
}

/// It drives no pin: a record carries what MISO reads.
impl<D: SpiTarget> Port<D> for Engine {
    const PINS: usize = 0;

    fn drive(&self, _image: &mut [Drive]) {}

    fn due_at(&self) -> Option<Time> {
        None
    }

    fn take_due(&mut self, _dev: &mut Hosting<D>, _world: &World, _at: Time) -> bool {
        unreachable!("the SPI engine has nothing due")
    }

    fn input(&mut self, _dev: &mut Hosting<D>, _world: &World, _at: Time, levels: &[bool]) {
        self.reset = self.bus.reset.is_some_and(|watch| !levels[watch]);
    }

    fn take_records(
        &mut self,
        dev: &mut Hosting<D>,
        world: &World,
        at: Time,
        records: &[LinkRow],
        answers: &mut Answers,
    ) {
        for row in records {
            let RecordKind::Spi { spi, nets, .. } = &row.record.kind else {
                continue;
            };
            if self.reset {
                continue;
            }
            if !self.wired(spi, nets) {
                self.miswired.get_or_insert((at, row.sender));
                continue;
            }
            let before = match spi.miso {
                Miso::Duplex => 0,
                Miso::None | Miso::Read(_) => spi.data.len(),
            };
            for read in 0..spi.reads() {
                let index = before + read;
                let data = &spi.data[..index.min(spi.data.len())];
                let byte = dev.call(world, at, |device, cx| {
                    device.answer(cx, at, &spi.phases, data, index)
                });
                if let Some(byte) = byte {
                    answers.drive(row, read, byte);
                }
            }
            dev.call(world, at, |device, cx| device.transaction(cx, at, spi));
        }
    }

    fn miswired(&self) -> Option<(Time, u16)> {
        self.miswired
    }
}
