//! The USB Serial/JTAG unit's serial port, the console. The firmware writes bytes to the EP1
//! FIFO, and while attached (`USB_PAD_ENABLE` set and the D+ pull-up on) the unit sends them as
//! one packet when the firmware writes `WR_DONE` or the FIFO holds its 64th byte. The host takes
//! each packet at once, so `SERIAL_IN_EP_DATA_FREE` reads 1 while attached. While detached, a
//! committed packet stays pending in the FIFO and `SERIAL_IN_EP_DATA_FREE` reads 0 until re-attach.
//!
//! Each byte the host sends is readable from the tick it arrives, in the order sent, while
//! attached. `SERIAL_OUT_EP_DATA_AVAIL` reads 1 while one is, and a load of EP1 takes it. EP1
//! reads 0 while none is.
//!
//! `SERIAL_IN_EMPTY` latches at reset and at every flush. `IN_TOKEN_REC_IN_EP1` latches at every
//! flush and once a 1 ms USB frame has started after that bit was last cleared while attached.
//! `SERIAL_OUT_RECV_PKT` latches when a byte has arrived after that bit was last cleared while
//! attached. A read or a write of the unit, and `take_received`, apply a due rise. Queueing a
//! later byte does not. The poll applies a due rise only while `INT_ENA` is nonzero, so while
//! `INT_ENA` is 0 the poll is one test. `INT_RAW` also reads SOF once a 1 ms frame has started
//! after its clear while attached, and that read is the only one that divides. The level is
//! `raw & ena != 0`.
//!
//! `CONF0` keeps its value from reset (`DP_PULLUP | USB_PAD_ENABLE`, `0x4200`). `USB_PAD_ENABLE`,
//! `PAD_PULL_OVERRIDE` and `DP_PULLUP` control host attachment and whether the USB PHY claims
//! GPIO19 and GPIO20. Any other writable `CONF0` field set away from reset ends the run.

use std::collections::VecDeque;

use super::clock::Tick;

const PACKET: usize = 64;
/// `USB_SERIAL_JTAG_EP1_CONF_REG`, whose bits say whether EP1 takes or holds a byte.
pub(crate) const EP1_CONF: u32 = 0x04;
/// `USB_SERIAL_JTAG_CONF0_REG`, PHY and pad configuration.
pub(crate) const CONF0: u32 = 0x18;
const WR_DONE: u32 = 1 << 0;
const SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;
const SERIAL_OUT_EP_DATA_AVAIL: u32 = 1 << 2;

/// Ticks in one USB frame, 1 ms at 480 MHz.
const FRAME: u64 = 480_000;
const SOF: u32 = 1 << 1;
const OUT_RECV: u32 = 1 << 2;
const IN_EMPTY: u32 = 1 << 3;
const TOKEN_IN_EP1: u32 = 1 << 8;
const BUS_RESET: u32 = 1 << 9;
/// `INT_ENA` bits the model runs. Any other bit set in a write ends the run.
pub(crate) const ENA_ALLOWED: u32 = OUT_RECV | IN_EMPTY | TOKEN_IN_EP1 | BUS_RESET;
/// `ETS_USB_SERIAL_JTAG_INTR_SOURCE`.
pub(crate) const SOURCE: u32 = 96;

/// `USB_SERIAL_JTAG_CONF0_REG` fields (ESP32-S3 TRM, Register `USB_SERIAL_JTAG_CONF0_REG`).
const PHY_SEL: u32 = 1 << 0;
const EXCHG_PINS_OVERRIDE: u32 = 1 << 1;
const EXCHG_PINS: u32 = 1 << 2;
const VREFH: u32 = 0x3 << 3;
const VREFL: u32 = 0x3 << 5;
const VREF_OVERRIDE: u32 = 1 << 7;
const PAD_PULL_OVERRIDE: u32 = 1 << 8;
const DP_PULLUP: u32 = 1 << 9;
const DP_PULLDOWN: u32 = 1 << 10;
const DM_PULLUP: u32 = 1 << 11;
const DM_PULLDOWN: u32 = 1 << 12;
const PULLUP_VALUE: u32 = 1 << 13;
const USB_PAD_ENABLE: u32 = 1 << 14;
const PHY_TX_EDGE_SEL: u32 = 1 << 15;
const USB_JTAG_BRIDGE_EN: u32 = 1 << 16;
const CONF0_MASK: u32 = (1 << 17) - 1;
const CONF0_RESET: u32 = DP_PULLUP | USB_PAD_ENABLE;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsbSerialJtag {
    fifo: Vec<u8>,
    /// True when the firmware committed a TX packet (`WR_DONE` or 64th byte) while detached,
    /// waiting for the host to take it on re-attach.
    tx_pending: bool,
    /// Each sent byte and its packet's flush, in the order sent.
    sent: VecDeque<(Tick, u8)>,
    /// Each byte the host sends and the tick it arrives, in the order sent.
    received: VecDeque<(Tick, u8)>,
    /// Latched `INT_RAW` bits other than SOF.
    raw: u32,
    /// `INT_ENA`.
    ena: u32,
    /// `CONF0`.
    conf0: u32,
    /// `None` until `SERIAL_OUT_RECV_PKT` is cleared, so a byte at tick 0 still raises. After a
    /// clear at `now`, the next byte arrives strictly later.
    recv_cleared: Option<Tick>,
    /// SOF reads 1 once a frame has started after this tick while attached.
    sof_cleared: Tick,
    /// `IN_TOKEN_REC_IN_EP1` latches once a frame has started after this tick while attached.
    token_cleared: Tick,
    /// `raw & ena != 0`.
    level: bool,
    /// Arrival of the first byte after `recv_cleared` while `SERIAL_OUT_RECV_PKT` is not latched
    /// and the PHY is attached. A read, a write, or `take_received` applies a due arrival. The
    /// poll applies it only while `ena` is nonzero.
    rises: Option<Tick>,
    /// Next 1 ms frame boundary after `token_cleared` while `IN_TOKEN_REC_IN_EP1` is not latched
    /// and the PHY is attached.
    token_rises: Option<Tick>,
}

impl Default for UsbSerialJtag {
    fn default() -> Self {
        Self {
            fifo: Vec::new(),
            tx_pending: false,
            sent: VecDeque::new(),
            received: VecDeque::new(),
            raw: IN_EMPTY,
            ena: 0,
            conf0: CONF0_RESET,
            recv_cleared: None,
            sof_cleared: Tick(0),
            token_cleared: Tick(0),
            level: false,
            rises: None,
            token_rises: Some(Tick(FRAME)),
        }
    }
}

impl UsbSerialJtag {
    /// Whether `USB_PAD_ENABLE` claims GPIO19 and GPIO20 for the USB PHY.
    pub(crate) fn pad_enabled(&self) -> bool {
        self.conf0 & USB_PAD_ENABLE != 0
    }

    /// Whether the Full-Speed D+ pull-up is asserted: hardware default when `PAD_PULL_OVERRIDE`
    /// is clear, or `DP_PULLUP` when software overrides pulls.
    fn dp_pullup_on(&self) -> bool {
        self.conf0 & PAD_PULL_OVERRIDE == 0 || self.conf0 & DP_PULLUP != 0
    }

    /// Whether the USB host sees the device attached on D+/D-.
    fn attached(&self) -> bool {
        self.pad_enabled() && self.dp_pullup_on()
    }

    fn refuse_conf0(value: u32) -> Option<&'static str> {
        if value & PHY_SEL != 0 {
            return Some("phy_sel");
        }
        if value & (EXCHG_PINS_OVERRIDE | EXCHG_PINS) != 0 {
            return Some("exchg_pins");
        }
        if value & (VREF_OVERRIDE | VREFH | VREFL) != 0 {
            return Some("vref");
        }
        if value & (DP_PULLDOWN | DM_PULLUP | DM_PULLDOWN) != 0 {
            return Some("pad_pull");
        }
        if value & PULLUP_VALUE != 0 {
            return Some("pullup_value");
        }
        if value & PHY_TX_EDGE_SEL != 0 {
            return Some("phy_tx_edge");
        }
        if value & USB_JTAG_BRIDGE_EN != 0 {
            return Some("jtag_bridge");
        }
        None
    }

    pub(crate) fn read(&mut self, now: Tick, offset: u32) -> Option<u32> {
        self.latch(now);
        let arrived = self
            .attached()
            .then(|| self.received.front().filter(|&&(at, _)| at <= now))
            .flatten();
        match offset {
            0x00 => Some(arrived.map_or(0, |&(_, byte)| u32::from(byte))),
            EP1_CONF => {
                let free = if !self.tx_pending && self.fifo.len() < PACKET {
                    SERIAL_IN_EP_DATA_FREE
                } else {
                    0
                };
                let avail = if arrived.is_some() {
                    SERIAL_OUT_EP_DATA_AVAIL
                } else {
                    0
                };
                Some(free | avail)
            }
            0x08 => Some(self.raw | self.sof(now)),
            0x0c => Some(self.raw & self.ena),
            0x10 => Some(self.ena),
            0x14 => Some(0),
            CONF0 => Some(self.conf0),
            _ => None,
        }
    }

    /// Takes the received byte EP1 reads at `now`, after a load of it. Latches a due rise first,
    /// so taking the byte does not drop `SERIAL_OUT_RECV_PKT`.
    pub(crate) fn take_received(&mut self, now: Tick) {
        self.latch(now);
        if self.attached() && self.received.front().is_some_and(|&(at, _)| at <= now) {
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

    pub(crate) fn write(
        &mut self,
        now: Tick,
        offset: u32,
        value: u32,
    ) -> Result<bool, &'static str> {
        self.latch(now);
        match offset {
            0x00 => {
                if !self.tx_pending && self.fifo.len() < PACKET {
                    self.fifo.push(value as u8);
                    if self.fifo.len() == PACKET {
                        if self.attached() {
                            self.flush(now);
                        } else {
                            self.tx_pending = true;
                        }
                    }
                }
            }
            0x04 => {
                if value & WR_DONE != 0 {
                    if self.attached() {
                        self.flush(now);
                    } else {
                        self.tx_pending = true;
                    }
                }
            }
            0x10 => {
                self.ena = value;
                self.arm_rise();
                self.arm_token();
            }
            0x14 => {
                self.raw &= !value;
                if value & OUT_RECV != 0 {
                    self.recv_cleared = Some(now);
                }
                if value & SOF != 0 {
                    self.sof_cleared = now;
                }
                if value & TOKEN_IN_EP1 != 0 {
                    self.token_cleared = now;
                }
                self.arm_rise();
                self.arm_token();
            }
            CONF0 => {
                if let Some(setting) = Self::refuse_conf0(value) {
                    return Err(setting);
                }
                let was_attached = self.attached();
                self.conf0 = value & CONF0_MASK;
                let attached = self.attached();
                match (was_attached, attached) {
                    (true, false) => {
                        self.rises = None;
                        self.token_rises = None;
                    }
                    (false, true) => {
                        self.sof_cleared = now;
                        self.token_cleared = now;
                        if self.tx_pending {
                            self.flush(now);
                        }
                        self.arm_rise();
                        self.arm_token();
                        self.latch(now);
                    }
                    _ => {}
                }
            }
            _ => return Ok(false),
        }
        self.refresh();
        Ok(true)
    }

    /// The source level at `now`. While nothing is enabled this is one test and latches nothing.
    /// A rise still ahead of `now` leaves the cached level, so the poll does not refresh it.
    #[inline]
    pub(crate) fn poll(&mut self, now: Tick) -> bool {
        if self.ena == 0 {
            return false;
        }
        if self.rises.is_some_and(|at| at <= now) || self.token_rises.is_some_and(|at| at <= now) {
            self.latch(now);
        }
        self.level
    }

    /// The earliest enabled USB event still ahead of `now`: a host OUT packet while
    /// `SERIAL_OUT_RECV_PKT` is enabled, or the next 1 ms IN token while `IN_TOKEN_REC_IN_EP1` is
    /// enabled.
    pub(crate) fn next_event(&self, now: Tick) -> Option<Tick> {
        let recv = (self.ena & OUT_RECV != 0)
            .then(|| self.rises.filter(|&at| at > now))
            .flatten();
        let token = (self.ena & TOKEN_IN_EP1 != 0)
            .then(|| self.token_rises.filter(|&at| at > now))
            .flatten();
        match (recv, token) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
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
        self.tx_pending = false;
        self.raw |= IN_EMPTY | TOKEN_IN_EP1;
        self.token_rises = None;
    }

    fn refresh(&mut self) {
        self.level = self.raw & self.ena != 0;
    }

    fn latch(&mut self, now: Tick) {
        if self.rises.is_some_and(|at| at <= now) {
            self.raw |= OUT_RECV;
            self.rises = None;
        }
        if self.token_rises.is_some_and(|at| at <= now) {
            self.raw |= TOKEN_IN_EP1;
            self.token_rises = None;
        }
        self.refresh();
    }

    fn sof(&self, now: Tick) -> u32 {
        if self.attached() && now.0 / FRAME > self.sof_cleared.0 / FRAME {
            SOF
        } else {
            0
        }
    }

    fn arm_rise(&mut self) {
        if !self.attached() || self.raw & OUT_RECV != 0 {
            self.rises = None;
            return;
        }
        let cleared = self.recv_cleared;
        self.rises = self
            .received
            .iter()
            .find_map(|&(at, _)| cleared.is_none_or(|mark| at > mark).then_some(at));
    }

    fn arm_token(&mut self) {
        if !self.attached() || self.raw & TOKEN_IN_EP1 != 0 {
            self.token_rises = None;
            return;
        }
        self.token_rises = (self.token_cleared.0 / FRAME)
            .checked_add(1)
            .and_then(|frame| frame.checked_mul(FRAME))
            .map(Tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packet_goes_at_wr_done_or_its_64th_byte() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(1), 0x00, u32::from(b'h')).unwrap();
        usb.write(Tick(2), 0x00, u32::from(b'i')).unwrap();
        assert_eq!(usb.take_sent(Tick(100)), None, "held in the FIFO");
        usb.write(Tick(3), 0x04, WR_DONE).unwrap();
        assert_eq!(usb.take_sent(Tick(2)), None);
        assert_eq!(usb.take_sent(Tick(3)), Some((Tick(3), b'h')));
        assert_eq!(usb.take_sent(Tick(3)), Some((Tick(3), b'i')));
        for k in 0..PACKET as u64 {
            usb.write(Tick(10 + k), 0x00, 0x41).unwrap();
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
        usb.write(Tick(1), 0x14, IN_EMPTY).unwrap();
        assert_eq!(usb.read(Tick(1), 0x08).unwrap() & IN_EMPTY, 0);
        usb.write(Tick(2), 0x00, u32::from(b'a')).unwrap();
        usb.write(Tick(3), 0x04, WR_DONE).unwrap();
        assert_eq!(usb.read(Tick(3), 0x08).unwrap() & IN_EMPTY, IN_EMPTY);
        usb.write(Tick(4), 0x14, IN_EMPTY).unwrap();
        usb.write(Tick(5), 0x04, WR_DONE).unwrap();
        assert_eq!(usb.read(Tick(5), 0x08).unwrap() & IN_EMPTY, IN_EMPTY);
        usb.write(Tick(6), 0x14, IN_EMPTY).unwrap();
        for _ in 0..PACKET {
            usb.write(Tick(7), 0x00, 0x41).unwrap();
        }
        assert_eq!(usb.read(Tick(7), 0x08).unwrap() & IN_EMPTY, IN_EMPTY);
    }

    #[test]
    fn a_byte_arriving_after_a_clear_sets_serial_out_recv_pkt() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(0), 0x10, OUT_RECV).unwrap();
        usb.write(Tick(0), 0x14, OUT_RECV).unwrap();
        usb.receive(Tick(10), b"a");
        assert_eq!(usb.read(Tick(9), 0x08).unwrap() & OUT_RECV, 0);
        assert_eq!(usb.read(Tick(10), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        let mut at_zero = UsbSerialJtag::default();
        at_zero.write(Tick(0), 0x10, OUT_RECV).unwrap();
        at_zero.receive(Tick(0), b"z");
        assert_eq!(at_zero.read(Tick(0), 0x08).unwrap() & OUT_RECV, OUT_RECV);
    }

    #[test]
    fn a_clear_while_a_byte_is_still_queued_drops_the_bit() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(0), 0x10, OUT_RECV).unwrap();
        usb.receive(Tick(10), b"a");
        assert_eq!(usb.read(Tick(10), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        usb.write(Tick(10), 0x14, OUT_RECV).unwrap();
        assert_eq!(usb.read(Tick(10), 0x08).unwrap() & OUT_RECV, 0);
        assert_eq!(usb.read(Tick(10), 0x00), Some(u32::from(b'a')));
    }

    #[test]
    fn sof_sets_at_the_next_1_ms_boundary_after_its_clear() {
        let mut usb = UsbSerialJtag::default();
        assert_eq!(usb.read(Tick(0), 0x08).unwrap() & SOF, 0);
        assert_eq!(usb.read(Tick(479_999), 0x08).unwrap() & SOF, 0);
        assert_eq!(usb.read(Tick(480_000), 0x08).unwrap() & SOF, SOF);
        usb.write(Tick(100), 0x14, IN_EMPTY).unwrap();
        assert_eq!(usb.read(Tick(480_000), 0x08).unwrap() & SOF, SOF);
        usb.write(Tick(480_000), 0x14, SOF).unwrap();
        assert_eq!(usb.read(Tick(480_000), 0x08).unwrap() & SOF, 0);
        assert_eq!(usb.read(Tick(959_999), 0x08).unwrap() & SOF, 0);
        assert_eq!(usb.read(Tick(960_000), 0x08).unwrap() & SOF, SOF);
    }

    #[test]
    fn in_token_rec_in_ep1_sets_at_1_ms_frames_and_on_flush() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(0), 0x10, TOKEN_IN_EP1).unwrap();
        assert_eq!(usb.read(Tick(0), 0x08).unwrap() & TOKEN_IN_EP1, 0);
        assert!(!usb.poll(Tick(479_999)));
        assert_eq!(usb.next_event(Tick(0)), Some(Tick(FRAME)));
        assert!(usb.poll(Tick(FRAME)));
        assert_eq!(
            usb.read(Tick(FRAME), 0x0c).unwrap() & TOKEN_IN_EP1,
            TOKEN_IN_EP1
        );
        assert_eq!(usb.next_event(Tick(FRAME)), None);

        // Clearing moves the next IN token to the next 1 ms USB frame.
        usb.write(Tick(FRAME), 0x14, TOKEN_IN_EP1).unwrap();
        assert!(!usb.poll(Tick(FRAME)));
        assert_eq!(usb.next_event(Tick(FRAME)), Some(Tick(2 * FRAME)));
        assert!(!usb.poll(Tick(2 * FRAME - 1)));
        assert!(usb.poll(Tick(2 * FRAME)));

        // A flush also latches IN_TOKEN_REC_IN_EP1 immediately.
        usb.write(Tick(2 * FRAME), 0x14, TOKEN_IN_EP1).unwrap();
        usb.write(Tick(2 * FRAME + 10), 0x00, u32::from(b'x'))
            .unwrap();
        usb.write(Tick(2 * FRAME + 20), 0x04, WR_DONE).unwrap();
        assert!(usb.poll(Tick(2 * FRAME + 20)));
        assert_eq!(
            usb.read(Tick(2 * FRAME + 20), 0x08).unwrap() & TOKEN_IN_EP1,
            TOKEN_IN_EP1
        );
        usb.write(Tick(2 * FRAME + 25), 0x14, TOKEN_IN_EP1).unwrap();
        assert_eq!(usb.next_event(Tick(2 * FRAME + 25)), Some(Tick(3 * FRAME)));
    }

    #[test]
    fn next_event_returns_the_earliest_enabled_usb_event() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(50), b"a");
        assert_eq!(usb.next_event(Tick(0)), None);
        usb.write(Tick(0), 0x10, IN_EMPTY).unwrap();
        assert_eq!(usb.next_event(Tick(0)), None);
        usb.write(Tick(0), 0x10, OUT_RECV | IN_EMPTY).unwrap();
        assert_eq!(usb.next_event(Tick(0)), Some(Tick(50)));
        assert_eq!(usb.next_event(Tick(49)), Some(Tick(50)));
        assert_eq!(usb.next_event(Tick(50)), None);
        usb.write(Tick(0), 0x10, TOKEN_IN_EP1).unwrap();
        assert_eq!(usb.next_event(Tick(0)), Some(Tick(FRAME)));
        usb.write(Tick(0), 0x10, OUT_RECV | TOKEN_IN_EP1).unwrap();
        assert_eq!(usb.next_event(Tick(0)), Some(Tick(50)));
        usb.write(Tick(0), 0x10, IN_EMPTY).unwrap();
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
        usb.write(
            Tick(1),
            0x10,
            IN_EMPTY | OUT_RECV | TOKEN_IN_EP1 | BUS_RESET,
        )
        .unwrap();
        check(&mut usb, Tick(1));
        usb.receive(Tick(4), b"a");
        check(&mut usb, Tick(3));
        check(&mut usb, Tick(4));
        usb.take_received(Tick(4));
        check(&mut usb, Tick(4));
        usb.write(Tick(4), 0x00, u32::from(b'b')).unwrap();
        usb.write(Tick(5), 0x04, WR_DONE).unwrap();
        check(&mut usb, Tick(5));
        usb.write(Tick(5), 0x14, IN_EMPTY | OUT_RECV | TOKEN_IN_EP1)
            .unwrap();
        check(&mut usb, Tick(5));
        check(&mut usb, Tick(480_000));
        usb.write(Tick(6), 0x10, 0).unwrap();
        check(&mut usb, Tick(480_000));
    }

    #[test]
    fn a_poll_with_nothing_enabled_stays_low_and_int_raw_shows_the_byte() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(1), b"a");
        assert!(!usb.poll(Tick(1)));
        assert_eq!(usb.read(Tick(1), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        assert!(!usb.poll(Tick(1)));
        usb.write(Tick(1), 0x10, OUT_RECV).unwrap();
        assert!(usb.poll(Tick(1)));
        assert_eq!(usb.read(Tick(1), 0x08).unwrap() & OUT_RECV, OUT_RECV);
    }

    #[test]
    fn a_byte_taken_then_enabled_sets_serial_out_recv_pkt_and_the_level() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(5), b"a");
        assert!(!usb.poll(Tick(5)));
        usb.take_received(Tick(6));
        usb.write(Tick(7), 0x10, OUT_RECV).unwrap();
        assert_eq!(usb.read(Tick(7), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        assert!(usb.poll(Tick(7)));
    }

    #[test]
    fn a_clear_of_another_bit_keeps_serial_out_recv_pkt() {
        let mut usb = UsbSerialJtag::default();
        usb.receive(Tick(5), b"a");
        assert!(!usb.poll(Tick(5)));
        usb.take_received(Tick(6));
        usb.write(Tick(7), 0x14, IN_EMPTY).unwrap();
        assert_eq!(usb.read(Tick(7), 0x08).unwrap() & OUT_RECV, OUT_RECV);
        usb.write(Tick(7), 0x10, OUT_RECV).unwrap();
        assert!(usb.poll(Tick(7)));
    }

    #[test]
    fn taking_the_byte_before_the_latch_keeps_the_armed_rise() {
        let mut usb = UsbSerialJtag::default();
        usb.write(Tick(0), 0x10, OUT_RECV).unwrap();
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

    #[test]
    fn conf0_resets_keeps_allowed_writes_and_refuses_unmodeled_fields() {
        let mut usb = UsbSerialJtag::default();
        assert_eq!(usb.read(Tick(0), CONF0), Some(CONF0_RESET));
        assert!(usb.pad_enabled());
        assert!(usb.attached());

        // gpio_ll_func_sel(20) + gpio_ll_pullup_dis(20) sequence.
        usb.write(Tick(1), CONF0, DP_PULLUP).unwrap();
        assert_eq!(usb.read(Tick(1), CONF0), Some(DP_PULLUP));
        assert!(!usb.pad_enabled());
        usb.write(Tick(2), CONF0, PAD_PULL_OVERRIDE | DP_PULLUP)
            .unwrap();
        usb.write(Tick(3), CONF0, PAD_PULL_OVERRIDE).unwrap();
        assert_eq!(usb.read(Tick(3), CONF0), Some(PAD_PULL_OVERRIDE));

        // Reserved upper bits read back 0.
        usb.write(Tick(4), CONF0, CONF0_RESET | 0xfffe_0000)
            .unwrap();
        assert_eq!(usb.read(Tick(4), CONF0), Some(CONF0_RESET));

        // Every unmodeled field is refused without changing CONF0.
        let refused = [
            (PHY_SEL, "phy_sel"),
            (EXCHG_PINS_OVERRIDE, "exchg_pins"),
            (EXCHG_PINS, "exchg_pins"),
            (1 << 3, "vref"),
            (1 << 5, "vref"),
            (VREF_OVERRIDE, "vref"),
            (DP_PULLDOWN, "pad_pull"),
            (DM_PULLUP, "pad_pull"),
            (DM_PULLDOWN, "pad_pull"),
            (PULLUP_VALUE, "pullup_value"),
            (PHY_TX_EDGE_SEL, "phy_tx_edge"),
            (USB_JTAG_BRIDGE_EN, "jtag_bridge"),
        ];
        for (bit, setting) in refused {
            assert_eq!(
                usb.write(Tick(5), CONF0, CONF0_RESET | bit),
                Err(setting),
                "{bit:#x}"
            );
            assert_eq!(usb.read(Tick(5), CONF0), Some(CONF0_RESET));
        }
    }

    #[test]
    fn detaching_via_conf0_halts_host_traffic_until_reattach() {
        for detach in [DP_PULLUP, USB_PAD_ENABLE | PAD_PULL_OVERRIDE] {
            let mut usb = UsbSerialJtag::default();
            usb.write(Tick(0), 0x14, IN_EMPTY).unwrap();
            usb.write(Tick(0), 0x10, OUT_RECV | IN_EMPTY | TOKEN_IN_EP1)
                .unwrap();
            usb.receive(Tick(100), b"k");

            // Detach before the byte or 1 ms frame arrives.
            usb.write(Tick(10), CONF0, detach).unwrap();
            assert!(!usb.attached());
            assert_eq!(usb.next_event(Tick(10)), None);
            assert!(!usb.poll(Tick(FRAME * 2)));
            assert_eq!(usb.read(Tick(FRAME * 2), 0x08).unwrap() & SOF, 0);
            assert_eq!(usb.read(Tick(FRAME * 2), 0x08).unwrap() & TOKEN_IN_EP1, 0);
            assert_eq!(usb.read(Tick(FRAME * 2), 0x08).unwrap() & OUT_RECV, 0);
            assert_eq!(
                usb.read(Tick(FRAME * 2), EP1_CONF),
                Some(SERIAL_IN_EP_DATA_FREE)
            );
            assert_eq!(usb.read(Tick(FRAME * 2), 0x00), Some(0));

            // Writing a TX packet while detached holds it pending until re-attach.
            usb.write(Tick(FRAME * 2 + 1), 0x00, u32::from(b'q'))
                .unwrap();
            usb.write(Tick(FRAME * 2 + 2), 0x04, WR_DONE).unwrap();
            assert_eq!(usb.read(Tick(FRAME * 2 + 2), EP1_CONF), Some(0));
            assert_eq!(usb.take_sent(Tick(u64::MAX)), None);
            assert_eq!(usb.read(Tick(FRAME * 2 + 2), 0x08).unwrap() & IN_EMPTY, 0);

            // Re-attach delivers the pending TX packet, queued RX byte and resumes frames.
            let reattach = Tick(FRAME * 2 + 10);
            usb.write(reattach, CONF0, CONF0_RESET).unwrap();
            assert!(usb.attached());
            assert_eq!(usb.take_sent(reattach), Some((reattach, b'q')));
            assert_eq!(
                usb.read(reattach, EP1_CONF),
                Some(SERIAL_IN_EP_DATA_FREE | SERIAL_OUT_EP_DATA_AVAIL)
            );
            assert_eq!(usb.read(reattach, 0x00), Some(u32::from(b'k')));
            let raw = usb.read(reattach, 0x08).unwrap();
            assert_eq!(
                raw & (IN_EMPTY | OUT_RECV | TOKEN_IN_EP1),
                IN_EMPTY | OUT_RECV | TOKEN_IN_EP1
            );
            usb.write(reattach, 0x14, IN_EMPTY | OUT_RECV | TOKEN_IN_EP1 | SOF)
                .unwrap();
            assert_eq!(usb.next_event(reattach), Some(Tick(FRAME * 3)));
            assert_eq!(usb.read(Tick(FRAME * 3), 0x08).unwrap() & SOF, SOF);
        }
    }
}
