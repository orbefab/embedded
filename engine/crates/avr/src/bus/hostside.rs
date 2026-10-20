//! Host peers that are not devices on the die.
//!
//! The host SPI master clocks a slave chip. The slave queue is the bytes that
//! host returns on `MISO` while the chip is master. The host I2C device is the
//! peer a TWI master addresses. This lives on [`super::Bus`], beside the device
//! list, because [`crate::core::Core::step`] only has `&mut Bus` and a master
//! transfer starts from an `SPDR` or `TWCR` store.

use std::collections::VecDeque;

use crate::Level;

fn bit_high(byte: u8, index: u8, lsb: bool) -> bool {
    let shift = if lsb { index } else { 7 - index };
    (byte >> shift) & 1 == 1
}

fn deposit(acc: u8, index: u8, lsb: bool, high: bool) -> u8 {
    if !high {
        return acc;
    }
    let shift = if lsb { index } else { 7 - index };
    acc | (1 << shift)
}

/// Up to three pad drives from one host-master edge.
#[derive(Clone, Copy)]
pub(crate) struct Drives {
    n: u8,
    pin: [u8; 3],
    level: [Level; 3],
}

impl Drives {
    fn none() -> Self {
        Self {
            n: 0,
            pin: [0; 3],
            level: [Level::Low; 3],
        }
    }

    fn push(&mut self, pin: u8, level: Level) {
        if usize::from(self.n) >= 3 {
            return;
        }
        let index = usize::from(self.n);
        self.pin[index] = pin;
        self.level[index] = level;
        self.n += 1;
    }

    pub(crate) fn iter(self) -> impl Iterator<Item = (u8, Level)> {
        let n = usize::from(self.n);
        (0..n).map(move |index| (self.pin[index], self.level[index]))
    }
}

/// Host master for a slave chip. Clocks `SS`/`SCK`/`MOSI` and samples `MISO`.
///
/// The chip's slave logic sees those pads. `SS` is driven low for the whole
/// call and high one cycle after the last SCK edge. A half-period is
/// `period/2`. §19.2 asks each half to be longer than two CPU cycles, so a
/// period of 8 is the shortest square wave that satisfies it.
#[derive(Debug, Clone)]
pub(crate) struct HostClock {
    active: bool,
    ss: u8,
    sck: u8,
    mosi: u8,
    miso: u8,
    half: u64,
    cpol_high: bool,
    cpha: bool,
    lsb: bool,
    bytes: Vec<u8>,
    index: usize,
    bit: u8,
    /// 0 = `SS` and the `CPHA` = 0 setup, 1 = leading, 2 = trailing, 3 = `SS` high.
    step: u8,
    next: u64,
    acc: u8,
    /// Each byte the host shifted in on `MISO`, stamped at that frame's last
    /// SCK edge.
    out: VecDeque<(u8, u64)>,
}

impl HostClock {
    pub(crate) fn idle() -> Self {
        Self {
            active: false,
            ss: 0,
            sck: 0,
            mosi: 0,
            miso: 0,
            half: 4,
            cpol_high: false,
            cpha: false,
            lsb: false,
            bytes: Vec::new(),
            index: 0,
            bit: 0,
            step: 0,
            next: u64::MAX,
            acc: 0,
            out: VecDeque::new(),
        }
    }

    pub(crate) fn next(&self) -> u64 {
        if self.active { self.next } else { u64::MAX }
    }

    pub(crate) fn due(&self, at: u64) -> bool {
        self.active && self.next == at
    }

    pub(crate) fn miso_pin(&self) -> u8 {
        self.miso
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn take_one(&mut self) -> Option<(u8, u64)> {
        self.out.pop_front()
    }

    pub(crate) fn arm(
        &mut self,
        pins: (u8, u8, u8, u8),
        bytes: &[u8],
        period: u64,
        mode: u8,
        lsb: bool,
        at: u64,
    ) {
        if self.active || bytes.is_empty() {
            return;
        }
        let half = (period / 2).max(1);
        self.active = true;
        self.ss = pins.0;
        self.mosi = pins.1;
        self.miso = pins.2;
        self.sck = pins.3;
        self.half = half;
        self.cpol_high = mode & 0x02 != 0;
        self.cpha = mode & 0x01 != 0;
        self.lsb = lsb;
        self.bytes = bytes.to_vec();
        self.index = 0;
        self.bit = 0;
        self.step = 0;
        self.next = at;
        self.acc = 0;
    }

    /// One edge. `miso_high` is the pad before this edge's drives.
    pub(crate) fn advance(&mut self, at: u64, miso_high: bool) -> Drives {
        let mut drives = Drives::none();
        if !self.due(at) {
            return drives;
        }
        match self.step {
            0 => {
                drives.push(self.ss, Level::Low);
                if !self.cpha {
                    drives.push(self.mosi, level_of(self.bytes[0], 0, self.lsb));
                }
                self.bit = 0;
                self.acc = 0;
                self.step = 1;
                self.next = at + self.half;
            }
            1 => {
                let byte = self.bytes[self.index];
                if self.cpha {
                    drives.push(self.mosi, level_of(byte, self.bit, self.lsb));
                } else {
                    self.acc = deposit(self.acc, self.bit, self.lsb, miso_high);
                }
                drives.push(self.sck, leading_level(self.cpol_high));
                self.step = 2;
                self.next = at + self.half;
            }
            2 => {
                drives.push(self.sck, idle_level(self.cpol_high));
                let byte = self.bytes[self.index];
                if self.cpha {
                    self.acc = deposit(self.acc, self.bit, self.lsb, miso_high);
                }
                self.bit += 1;
                if self.bit == 8 {
                    self.out.push_back((self.acc, at));
                    self.index += 1;
                    self.bit = 0;
                    self.acc = 0;
                    if self.index >= self.bytes.len() {
                        self.step = 3;
                        self.next = at + 1;
                    } else {
                        if !self.cpha {
                            drives.push(self.mosi, level_of(self.bytes[self.index], 0, self.lsb));
                        }
                        self.step = 1;
                        self.next = at + self.half;
                    }
                } else {
                    if !self.cpha {
                        drives.push(self.mosi, level_of(byte, self.bit, self.lsb));
                    }
                    self.step = 1;
                    self.next = at + self.half;
                }
            }
            _ => {
                drives.push(self.ss, Level::High);
                self.active = false;
                self.next = u64::MAX;
            }
        }
        drives
    }
}

fn level_of(byte: u8, index: u8, lsb: bool) -> Level {
    if bit_high(byte, index, lsb) {
        Level::High
    } else {
        Level::Low
    }
}

fn leading_level(cpol_high: bool) -> Level {
    if cpol_high { Level::Low } else { Level::High }
}

fn idle_level(cpol_high: bool) -> Level {
    if cpol_high { Level::High } else { Level::Low }
}

#[derive(Debug, Clone)]
pub(crate) struct HostSide {
    pub(crate) master: HostClock,
    /// One byte per master frame. An empty queue shifts in `0xFF`.
    slave_miso: VecDeque<u8>,
    i2c: Vec<I2cDev>,
    i2c_pending: Vec<I2cAttach>,
    logged_ff: bool,
}

impl HostSide {
    pub(crate) fn idle() -> Self {
        Self {
            master: HostClock::idle(),
            slave_miso: VecDeque::new(),
            i2c: Vec::new(),
            i2c_pending: Vec::new(),
            logged_ff: false,
        }
    }

    pub(crate) fn push_miso(&mut self, bytes: &[u8]) {
        self.slave_miso.extend(bytes);
    }

    pub(crate) fn slave_miso(&mut self) -> &mut VecDeque<u8> {
        &mut self.slave_miso
    }

    /// The host device sits outside the MCU, so a reset keeps it.
    /// The transaction in flight is aborted: write count and the repeat-read
    /// index clear, and unread queue bytes stay.
    pub(crate) fn take_i2c_across_reset(&mut self) -> I2cKept {
        for dev in &mut self.i2c {
            dev.write_n = 0;
            dev.read_i = 0;
        }
        I2cKept {
            devices: std::mem::take(&mut self.i2c),
            pending: std::mem::take(&mut self.i2c_pending),
            logged_ff: self.logged_ff,
        }
    }

    pub(crate) fn restore_i2c_across_reset(&mut self, kept: I2cKept) {
        self.i2c = kept.devices;
        self.i2c_pending = kept.pending;
        self.logged_ff = kept.logged_ff;
    }

    /// Queue an attach, replace, or detach. It takes effect at the next address phase.
    pub(crate) fn queue_i2c(&mut self, update: I2cAttach) {
        self.i2c_pending.push(update);
    }

    /// SLA+R/W. Pending attaches apply here, then the addressed device ACKs or not.
    pub(crate) fn i2c_address(&mut self, addr: u8) -> bool {
        self.apply_i2c();
        let Some(dev) = self.i2c.iter_mut().find(|dev| dev.addr == addr) else {
            return false;
        };
        dev.write_n = 0;
        // `read_i` is read only while `repeat` is set, after this ACK.
        dev.read_i = 0;
        true
    }

    /// Data byte of the current write. `false` when nothing is addressed or this index NACKs.
    pub(crate) fn i2c_write_data(&mut self, addr: u8) -> bool {
        let Some(dev) = self.i2c.iter_mut().find(|dev| dev.addr == addr) else {
            return false;
        };
        let nack = dev.nack_at == Some(dev.write_n);
        dev.write_n = dev.write_n.saturating_add(1);
        !nack
    }

    /// Next byte of a read. Past the end is `0xFF`, logged once.
    pub(crate) fn i2c_read_data(&mut self, addr: u8, ctx: &mut crate::periph::Ctx<'_>) -> u8 {
        let got = self.i2c.iter_mut().find(|dev| dev.addr == addr).map(|dev| {
            if dev.repeat {
                if dev.read_i >= dev.pattern.len() {
                    return None;
                }
                let byte = dev.pattern[dev.read_i];
                dev.read_i += 1;
                return Some(byte);
            }
            dev.reads.pop_front()
        });
        match got {
            Some(Some(byte)) => byte,
            _ => self.i2c_ff(ctx),
        }
    }

    /// A read with no device on the address. The wire is high.
    pub(crate) fn i2c_idle_read(&mut self, ctx: &mut crate::periph::Ctx<'_>) -> u8 {
        self.i2c_ff(ctx)
    }

    fn i2c_ff(&mut self, ctx: &mut crate::periph::Ctx<'_>) -> u8 {
        if !self.logged_ff {
            self.logged_ff = true;
            ctx.log(0x00BB, 0xFF);
        }
        0xFF
    }

    fn apply_i2c(&mut self) {
        let pending = std::mem::take(&mut self.i2c_pending);
        for update in pending {
            if !update.present {
                self.i2c.retain(|dev| dev.addr != update.addr);
                continue;
            }
            let dev = I2cDev {
                addr: update.addr,
                reads: update.reads.iter().copied().collect(),
                pattern: update.reads,
                repeat: update.repeat,
                nack_at: update.nack_data_at,
                write_n: 0,
                read_i: 0,
            };
            if let Some(slot) = self.i2c.iter_mut().find(|dev| dev.addr == update.addr) {
                *slot = dev;
            } else {
                self.i2c.push(dev);
            }
        }
    }
}

/// Devices and pending attaches carried across an MCU reset.
pub(crate) struct I2cKept {
    devices: Vec<I2cDev>,
    pending: Vec<I2cAttach>,
    logged_ff: bool,
}

/// One host I2C device, applied at the next address phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct I2cAttach {
    pub addr: u8,
    pub present: bool,
    pub reads: Vec<u8>,
    pub repeat: bool,
    pub nack_data_at: Option<u8>,
}

#[derive(Debug, Clone)]
struct I2cDev {
    addr: u8,
    reads: VecDeque<u8>,
    pattern: Vec<u8>,
    repeat: bool,
    nack_at: Option<u8>,
    write_n: u8,
    read_i: usize,
}
