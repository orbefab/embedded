//! The USB Serial/JTAG unit's serial port, the console. The firmware writes bytes to the EP1
//! FIFO, and the unit sends them as one packet when the firmware writes `WR_DONE` or the FIFO
//! holds its 64th byte. The host takes each packet at once, so the FIFO always has room.
//!
//! Each byte the host sends is readable from the tick it arrives, in the order sent.
//! `SERIAL_OUT_EP_DATA_AVAIL` reads 1 while one is, and a load of EP1 takes it. EP1 reads 0
//! while none is.

use std::collections::VecDeque;

use super::clock::Tick;

const PACKET: usize = 64;
const WR_DONE: u32 = 1 << 0;
const SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;
const SERIAL_OUT_EP_DATA_AVAIL: u32 = 1 << 2;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UsbSerialJtag {
    fifo: Vec<u8>,
    /// Each sent byte and its packet's flush, in the order sent.
    sent: VecDeque<(Tick, u8)>,
    /// Each byte the host sends and the tick it arrives, in the order sent.
    received: VecDeque<(Tick, u8)>,
}

impl UsbSerialJtag {
    pub(crate) fn read(&self, now: Tick, offset: u32) -> Option<u32> {
        let arrived = self.received.front().filter(|&&(at, _)| at <= now);
        match offset {
            0x00 => Some(arrived.map_or(0, |&(_, byte)| u32::from(byte))),
            0x04 if arrived.is_some() => Some(SERIAL_IN_EP_DATA_FREE | SERIAL_OUT_EP_DATA_AVAIL),
            0x04 => Some(SERIAL_IN_EP_DATA_FREE),
            _ => None,
        }
    }

    /// Takes the received byte EP1 reads at `now`, after a load of it.
    pub(crate) fn take_received(&mut self, now: Tick) {
        if self.received.front().is_some_and(|&(at, _)| at <= now) {
            self.received.pop_front();
        }
    }

    /// Queues `bytes`, which the host sends to arrive at `at`, after every byte queued before.
    pub(crate) fn receive(&mut self, at: Tick, bytes: &[u8]) {
        self.received.extend(bytes.iter().map(|&byte| (at, byte)));
    }

    pub(crate) fn write(&mut self, now: Tick, offset: u32, value: u32) -> bool {
        match offset {
            0x00 => {
                self.fifo.push(value as u8);
                if self.fifo.len() == PACKET {
                    self.flush(now);
                }
            }
            0x04 => {
                if value & WR_DONE != 0 {
                    self.flush(now);
                }
            }
            _ => return false,
        }
        true
    }

    fn flush(&mut self, now: Tick) {
        self.sent
            .extend(self.fifo.drain(..).map(|byte| (now, byte)));
    }

    /// The first sent byte not taken yet, when its packet flushed at or before `through`.
    pub(crate) fn take_sent(&mut self, through: Tick) -> Option<(Tick, u8)> {
        let &(at, byte) = self.sent.front()?;
        if at > through {
            return None;
        }
        self.sent.pop_front();
        Some((at, byte))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packet_goes_at_wr_done_or_its_64th_byte() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(1), 0x00, u32::from(b'h'));
        usb.write(Tick(2), 0x00, u32::from(b'i'));
        assert_eq!(usb.take_sent(Tick(100)), None, "held in the FIFO");
        usb.write(Tick(3), 0x04, WR_DONE);
        assert_eq!(usb.take_sent(Tick(2)), None);
        assert_eq!(usb.take_sent(Tick(3)), Some((Tick(3), b'h')));
        assert_eq!(usb.take_sent(Tick(3)), Some((Tick(3), b'i')));
        for k in 0..PACKET as u64 {
            usb.write(Tick(10 + k), 0x00, 0x41);
        }
        let flush = Tick(10 + PACKET as u64 - 1);
        assert!((0..PACKET).all(|_| usb.take_sent(flush) == Some((flush, 0x41))));
        assert_eq!(usb.take_sent(Tick(u64::MAX)), None);
    }

    #[test]
    fn a_received_byte_is_readable_from_its_arrival_until_a_load_takes_it() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(10), b"go");
        assert_eq!(usb.read(Tick(9), 0x04), Some(SERIAL_IN_EP_DATA_FREE));
        assert_eq!(usb.read(Tick(9), 0x00), Some(0));
        usb.take_received(Tick(9));
        let avail = Some(SERIAL_IN_EP_DATA_FREE | SERIAL_OUT_EP_DATA_AVAIL);
        assert_eq!(usb.read(Tick(10), 0x04), avail);
        assert_eq!(usb.read(Tick(10), 0x00), Some(u32::from(b'g')));
        usb.take_received(Tick(10));
        assert_eq!(usb.read(Tick(11), 0x00), Some(u32::from(b'o')));
        usb.take_received(Tick(11));
        assert_eq!(usb.read(Tick(12), 0x04), Some(SERIAL_IN_EP_DATA_FREE));
        assert_eq!(usb.read(Tick(12), 0x00), Some(0));
    }
}
