//! The USB Serial/JTAG unit's serial port, the console. The firmware writes bytes to the EP1
//! FIFO, and the unit sends them as one packet when the firmware writes `WR_DONE` or the FIFO
//! holds its 64th byte. The host takes each packet at once, so the FIFO always has room.
//!
//! Each byte the host sends is readable from the tick it arrives, in the order sent.
//! `SERIAL_OUT_EP_DATA_AVAIL` reads 1 while one is, and a load of EP1 takes it. EP1 reads 0
//! while none is.
//!
//! `SERIAL_IN_EMPTY` latches at reset and at every flush. `SERIAL_OUT_RECV_PKT` latches when a
//! byte has arrived after that bit was last cleared. A read or a write of the unit, and
//! `take_received`, apply a due rise. Queueing a later byte does not. The poll applies a due
//! rise only while `INT_ENA` is nonzero, so while `INT_ENA` is 0 the poll is one test. `INT_RAW`
//! also reads SOF once a 1 ms frame has started after its clear, and that read is the only one
//! that divides. The level is `raw & ena != 0`.

use std::collections::VecDeque;

use super::clock::Tick;

const PACKET: usize = 64;
/// `USB_SERIAL_JTAG_EP1_CONF_REG`, whose bits say whether EP1 takes or holds a byte.
pub(crate) const EP1_CONF: u32 = 0x04;
const WR_DONE: u32 = 1 << 0;
const SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;
const SERIAL_OUT_EP_DATA_AVAIL: u32 = 1 << 2;

/// Ticks in one USB frame, 1 ms at 480 MHz.
const FRAME: u64 = 480_000;
const SOF: u32 = 1 << 1;
const OUT_RECV: u32 = 1 << 2;
const IN_EMPTY: u32 = 1 << 3;
const BUS_RESET: u32 = 1 << 9;
/// `INT_ENA` bits the model runs. Any other bit set in a write ends the run.
pub(crate) const ENA_ALLOWED: u32 = OUT_RECV | IN_EMPTY | BUS_RESET;
/// `ETS_USB_SERIAL_JTAG_INTR_SOURCE`.
pub(crate) const SOURCE: u32 = 96;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsbSerialJtag {
    fifo: Vec<u8>,
    /// Each sent byte and its packet's flush, in the order sent.
    sent: VecDeque<(Tick, u8)>,
    /// Each byte the host sends and the tick it arrives, in the order sent.
    received: VecDeque<(Tick, u8)>,
    /// Latched `INT_RAW` bits other than SOF.
    raw: u32,
    /// `INT_ENA`.
    ena: u32,
    /// `None` until `SERIAL_OUT_RECV_PKT` is cleared, so a byte at tick 0 still raises. After a
    /// clear at `now`, the next byte arrives strictly later.
    recv_cleared: Option<Tick>,
    /// SOF reads 1 once a frame has started after this tick.
    sof_cleared: Tick,
    /// `raw & ena != 0`.
    level: bool,
    /// Arrival of the first byte after `recv_cleared` while `SERIAL_OUT_RECV_PKT` is not latched.
    /// A read, a write, or `take_received` applies a due arrival. The poll applies it only while
    /// `ena` is nonzero.
    rises: Option<Tick>,
}

impl Default for UsbSerialJtag {
    fn default() -> Self {
        Self {
            fifo: Vec::new(),
            sent: VecDeque::new(),
            received: VecDeque::new(),
            raw: IN_EMPTY,
            ena: 0,
            recv_cleared: None,
            sof_cleared: Tick(0),
            level: false,
            rises: None,
        }
    }
}

impl UsbSerialJtag {
    pub(crate) fn read(&mut self, now: Tick, offset: u32) -> Option<u32> {
        self.latch(now);
        let arrived = self.received.front().filter(|&&(at, _)| at <= now);
        match offset {
            0x00 => Some(arrived.map_or(0, |&(_, byte)| u32::from(byte))),
            EP1_CONF if arrived.is_some() => {
                Some(SERIAL_IN_EP_DATA_FREE | SERIAL_OUT_EP_DATA_AVAIL)
            }
            EP1_CONF => Some(SERIAL_IN_EP_DATA_FREE),
            0x08 => Some(self.raw | self.sof(now)),
            0x0c => Some(self.raw & self.ena),
            0x10 => Some(self.ena),
            0x14 => Some(0),
            _ => None,
        }
    }

    /// Takes the received byte EP1 reads at `now`, after a load of it. Latches a due rise first,
    /// so taking the byte does not drop `SERIAL_OUT_RECV_PKT`.
    pub(crate) fn take_received(&mut self, now: Tick) {
        self.latch(now);
        if self.received.front().is_some_and(|&(at, _)| at <= now) {
            self.received.pop_front();
        }
        self.refresh();
    }

    /// Queues `bytes`, which the host sends to arrive at `at`, after every byte queued before.
    /// A rise that is already due stays pending. Applying it here would set the bit before `at`
    /// once a later byte is queued, and a read before the first arrival would see it.
    pub(crate) fn receive(&mut self, at: Tick, bytes: &[u8]) {
        self.received.extend(bytes.iter().map(|&byte| (at, byte)));
        self.arm_rise();
        self.refresh();
    }

    pub(crate) fn write(&mut self, now: Tick, offset: u32, value: u32) -> bool {
        self.latch(now);
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
            0x10 => {
                self.ena = value;
                self.arm_rise();
            }
            0x14 => {
                self.raw &= !value;
                if value & OUT_RECV != 0 {
                    self.recv_cleared = Some(now);
                }
                if value & SOF != 0 {
                    self.sof_cleared = now;
                }
                self.arm_rise();
            }
            _ => return false,
        }
        self.refresh();
        true
    }

    /// The source level at `now`. While nothing is enabled this is one test and latches nothing.
    /// A rise still ahead of `now` leaves the cached level, so the poll does not refresh it.
    #[inline]
    pub(crate) fn poll(&mut self, now: Tick) -> bool {
        if self.ena == 0 {
            return false;
        }
        if self.rises.is_some_and(|at| at <= now) {
            self.latch(now);
        }
        self.level
    }

    /// When `SERIAL_OUT_RECV_PKT` is enabled, the arrival still ahead of `now`.
    pub(crate) fn next_event(&self, now: Tick) -> Option<Tick> {
        if self.ena & OUT_RECV == 0 {
            return None;
        }
        self.rises.filter(|&at| at > now)
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

    fn flush(&mut self, now: Tick) {
        self.sent
            .extend(self.fifo.drain(..).map(|byte| (now, byte)));
        self.raw |= IN_EMPTY;
    }

    fn refresh(&mut self) {
        self.level = self.raw & self.ena != 0;
    }

    fn latch(&mut self, now: Tick) {
        if self.rises.is_some_and(|at| at <= now) {
            self.raw |= OUT_RECV;
            self.rises = None;
        }
        self.refresh();
    }

    fn sof(&self, now: Tick) -> u32 {
        if now.0 / FRAME > self.sof_cleared.0 / FRAME {
            SOF
        } else {
            0
        }
    }

    fn arm_rise(&mut self) {
        if self.raw & OUT_RECV != 0 {
            self.rises = None;
            return;
        }
        let cleared = self.recv_cleared;
        self.rises = self
            .received
            .iter()
            .find_map(|&(at, _)| cleared.is_none_or(|mark| at > mark).then_some(at));
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

    #[test]
    fn serial_in_empty_reads_1_from_reset_and_after_a_flush_and_its_clear_drops_it() {
        let mut usb = UsbSerialJtag::default();
        assert_eq!(usb.read(Tick(0), 0x08), Some(IN_EMPTY));
        usb.write(Tick(1), 0x14, IN_EMPTY);
        assert_eq!(usb.read(Tick(1), 0x08).unwrap() & IN_EMPTY, 0);
        usb.write(Tick(2), 0x00, u32::from(b'a'));
        usb.write(Tick(3), 0x04, WR_DONE);
        assert_eq!(usb.read(Tick(3), 0x08).unwrap() & IN_EMPTY, IN_EMPTY);
        usb.write(Tick(4), 0x14, IN_EMPTY);
        usb.write(Tick(5), 0x04, WR_DONE);
        assert_eq!(usb.read(Tick(5), 0x08).unwrap() & IN_EMPTY, IN_EMPTY);
        usb.write(Tick(6), 0x14, IN_EMPTY);
        for _ in 0..PACKET {
            usb.write(Tick(7), 0x00, 0x41);
        }
        assert_eq!(usb.read(Tick(7), 0x08).unwrap() & IN_EMPTY, IN_EMPTY);
    }

    #[test]
    fn a_byte_arriving_after_a_clear_sets_serial_out_recv_pkt() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(0), 0x10, OUT_RECV);
        usb.write(Tick(0), 0x14, OUT_RECV);
        usb.receive(Tick(10), b"a");
        assert_eq!(usb.read(Tick(9), 0x08).unwrap() & OUT_RECV, 0);
        assert_eq!(usb.read(Tick(10), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        let mut at_zero = UsbSerialJtag::default();
        at_zero.write(Tick(0), 0x10, OUT_RECV);
        at_zero.receive(Tick(0), b"z");
        assert_eq!(at_zero.read(Tick(0), 0x08).unwrap() & OUT_RECV, OUT_RECV);
    }

    #[test]
    fn a_clear_while_a_byte_is_still_queued_drops_the_bit() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(0), 0x10, OUT_RECV);
        usb.receive(Tick(10), b"a");
        assert_eq!(usb.read(Tick(10), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        usb.write(Tick(10), 0x14, OUT_RECV);
        assert_eq!(usb.read(Tick(10), 0x08).unwrap() & OUT_RECV, 0);
        assert_eq!(usb.read(Tick(10), 0x00), Some(u32::from(b'a')));
    }

    #[test]
    fn sof_sets_at_the_next_1_ms_boundary_after_its_clear() {
        let mut usb = UsbSerialJtag::default();
        assert_eq!(usb.read(Tick(0), 0x08).unwrap() & SOF, 0);
        assert_eq!(usb.read(Tick(479_999), 0x08).unwrap() & SOF, 0);
        assert_eq!(usb.read(Tick(480_000), 0x08).unwrap() & SOF, SOF);
        usb.write(Tick(100), 0x14, IN_EMPTY);
        assert_eq!(usb.read(Tick(480_000), 0x08).unwrap() & SOF, SOF);
        usb.write(Tick(480_000), 0x14, SOF);
        assert_eq!(usb.read(Tick(480_000), 0x08).unwrap() & SOF, 0);
        assert_eq!(usb.read(Tick(959_999), 0x08).unwrap() & SOF, 0);
        assert_eq!(usb.read(Tick(960_000), 0x08).unwrap() & SOF, SOF);
    }

    #[test]
    fn next_event_returns_the_arrival_only_while_serial_out_recv_pkt_is_enabled() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(50), b"a");
        assert_eq!(usb.next_event(Tick(0)), None);
        usb.write(Tick(0), 0x10, IN_EMPTY);
        assert_eq!(usb.next_event(Tick(0)), None);
        usb.write(Tick(0), 0x10, OUT_RECV | IN_EMPTY);
        assert_eq!(usb.next_event(Tick(0)), Some(Tick(50)));
        assert_eq!(usb.next_event(Tick(49)), Some(Tick(50)));
        assert_eq!(usb.next_event(Tick(50)), None);
        usb.write(Tick(0), 0x10, IN_EMPTY);
        assert_eq!(usb.next_event(Tick(0)), None);
    }

    #[test]
    fn level_equals_raw_and_ena_after_every_operation() {
        let mut usb = UsbSerialJtag::default();
        let check = |usb: &mut UsbSerialJtag, now: Tick| {
            let raw = usb.read(now, 0x08).unwrap();
            let status = usb.read(now, 0x0c).unwrap();
            let ena = usb.read(now, 0x10).unwrap();
            assert_eq!(status & SOF, 0);
            assert_eq!(status, (raw & !SOF) & ena);
            assert_eq!(status != 0, usb.poll(now));
        };
        check(&mut usb, Tick(0));
        usb.write(Tick(1), 0x10, IN_EMPTY | OUT_RECV | BUS_RESET);
        check(&mut usb, Tick(1));
        usb.receive(Tick(4), b"a");
        check(&mut usb, Tick(3));
        check(&mut usb, Tick(4));
        usb.take_received(Tick(4));
        check(&mut usb, Tick(4));
        usb.write(Tick(4), 0x00, u32::from(b'b'));
        usb.write(Tick(5), 0x04, WR_DONE);
        check(&mut usb, Tick(5));
        usb.write(Tick(5), 0x14, IN_EMPTY | OUT_RECV);
        check(&mut usb, Tick(5));
        check(&mut usb, Tick(480_000));
        usb.write(Tick(6), 0x10, 0);
        check(&mut usb, Tick(480_000));
    }

    #[test]
    fn a_poll_with_nothing_enabled_stays_low_and_int_raw_shows_the_byte() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(1), b"a");
        assert!(!usb.poll(Tick(1)));
        assert_eq!(usb.read(Tick(1), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        assert!(!usb.poll(Tick(1)));
        usb.write(Tick(1), 0x10, OUT_RECV);
        assert!(usb.poll(Tick(1)));
        assert_eq!(usb.read(Tick(1), 0x08).unwrap() & OUT_RECV, OUT_RECV);
    }

    #[test]
    fn a_byte_taken_then_enabled_sets_serial_out_recv_pkt_and_the_level() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(5), b"a");
        assert!(!usb.poll(Tick(5)));
        usb.take_received(Tick(6));
        usb.write(Tick(7), 0x10, OUT_RECV);
        assert_eq!(usb.read(Tick(7), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        assert!(usb.poll(Tick(7)));
    }

    #[test]
    fn a_clear_of_another_bit_keeps_serial_out_recv_pkt() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(5), b"a");
        assert!(!usb.poll(Tick(5)));
        usb.take_received(Tick(6));
        usb.write(Tick(7), 0x14, IN_EMPTY);
        assert_eq!(usb.read(Tick(7), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        usb.write(Tick(7), 0x10, OUT_RECV);
        assert!(usb.poll(Tick(7)));
    }

    #[test]
    fn taking_the_byte_before_the_latch_keeps_the_armed_rise() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(0), 0x10, OUT_RECV);
        usb.receive(Tick(5), b"a");
        assert_eq!(usb.read(Tick(5), 0x00), Some(u32::from(b'a')));
        usb.take_received(Tick(5));
        assert!(usb.poll(Tick(5)));
        assert_eq!(usb.read(Tick(5), 0x00), Some(0));
    }

    #[test]
    fn a_taken_byte_stays_in_int_raw_when_a_later_byte_is_queued() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(5), b"a");
        usb.take_received(Tick(6));
        usb.receive(Tick(9), b"b");
        assert_eq!(usb.read(Tick(7), 0x08).unwrap() & OUT_RECV, OUT_RECV);
    }

    #[test]
    fn a_later_queued_byte_does_not_set_int_raw_before_the_first_arrival() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(10), b"a");
        usb.receive(Tick(20), b"b");
        assert_eq!(usb.read(Tick(9), 0x08).unwrap() & OUT_RECV, 0);
        assert_eq!(usb.read(Tick(10), 0x08).unwrap() & OUT_RECV, OUT_RECV);
    }
}
