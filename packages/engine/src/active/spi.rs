//! The host's SPI target engine. It takes each `Spi` record routed to its target, drops one while
//! the target is in reset or when a lane it used sits on another net than the target's data pin,
//! and calls [`SpiTarget::transaction`] for the rest.
//!
//! One [`Hosted::take`](super::native::Hosted::take) enters per landing. The loop below then calls
//! the device on the concrete type, so a transaction's bytes and pixels never cross a `dyn` call.

use orbe_model::{Drive, Spi, SpiTarget, Time};

use super::native::{Hosting, Port};
use crate::link::{LinkRow, RecordKind, SpiNets};
use crate::netlist::NetId;

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

    /// Whether each lane the transaction uses sits on the target's own data pin.
    fn wired(&self, spi: &Spi, nets: &SpiNets) -> bool {
        let used = spi
            .phases
            .iter()
            .map(|phase| phase.lanes.count())
            .chain([spi.lanes.count()])
            .max()
            .unwrap_or(1);
        (0..usize::from(used)).all(|k| nets.data[k] == Some(self.bus.data[k]))
    }
}

/// It drives no pin: MISO is not modelled.
impl<D: SpiTarget> Port<D> for Engine {
    const PINS: usize = 0;

    fn drive(&self, _image: &mut [Drive]) {}

    fn due_at(&self) -> Option<Time> {
        None
    }

    fn take_due(&mut self, _dev: &mut Hosting<D>, _at: Time) -> bool {
        unreachable!("the SPI engine has nothing due")
    }

    fn input(&mut self, _dev: &mut Hosting<D>, _at: Time, levels: &[bool]) {
        self.reset = self.bus.reset.is_some_and(|watch| !levels[watch]);
    }

    fn take_records(&mut self, dev: &mut Hosting<D>, at: Time, records: &[LinkRow]) {
        for row in records {
            let RecordKind::Spi { spi, nets } = &row.record.kind else {
                continue;
            };
            if self.reset {
                continue;
            }
            if !self.wired(spi, nets) {
                self.miswired.get_or_insert((at, row.sender));
                continue;
            }
            dev.call(at, |device, cx| device.transaction(cx, at, spi));
        }
    }

    fn miswired(&self) -> Option<(Time, u16)> {
        self.miswired
    }
}
