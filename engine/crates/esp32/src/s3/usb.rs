//! The USB Serial/JTAG unit's serial TX, the console. The firmware writes bytes to the EP1 FIFO,
//! and the unit sends them as one packet when the firmware writes `WR_DONE` or the FIFO holds
//! its 64th byte. The host takes each packet at once, so the FIFO always has room. The host
//! sends nothing, so `SERIAL_OUT_EP_DATA_AVAIL` reads 0 and EP1 reads 0.

use std::collections::VecDeque;

use sim_xtensa::Cycle;

const PACKET: usize = 64;
const WR_DONE: u32 = 1 << 0;
const SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UsbSerialJtag {
    fifo: Vec<u8>,
    /// Each sent byte and its packet's flush, in the order sent.
    sent: VecDeque<(Cycle, u8)>,
}

impl UsbSerialJtag {
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        match offset {
            0x00 => Some(0),
            0x04 => Some(SERIAL_IN_EP_DATA_FREE),
            _ => None,
        }
    }

    pub(crate) fn write(&mut self, now: Cycle, offset: u32, value: u32) -> bool {
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

    fn flush(&mut self, now: Cycle) {
        self.sent
            .extend(self.fifo.drain(..).map(|byte| (now, byte)));
    }

    /// The first sent byte not taken yet, when its packet flushed at or before `through`.
    pub(crate) fn take_sent(&mut self, through: Cycle) -> Option<(Cycle, u8)> {
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
        usb.write(Cycle(1), 0x00, u32::from(b'h'));
        usb.write(Cycle(2), 0x00, u32::from(b'i'));
        assert_eq!(usb.take_sent(Cycle(100)), None, "held in the FIFO");
        usb.write(Cycle(3), 0x04, WR_DONE);
        assert_eq!(usb.take_sent(Cycle(2)), None);
        assert_eq!(usb.take_sent(Cycle(3)), Some((Cycle(3), b'h')));
        assert_eq!(usb.take_sent(Cycle(3)), Some((Cycle(3), b'i')));
        for k in 0..PACKET as u64 {
            usb.write(Cycle(10 + k), 0x00, 0x41);
        }
        let flush = Cycle(10 + PACKET as u64 - 1);
        assert!((0..PACKET).all(|_| usb.take_sent(flush) == Some((flush, 0x41))));
        assert_eq!(usb.take_sent(Cycle(u64::MAX)), None);
    }
}
