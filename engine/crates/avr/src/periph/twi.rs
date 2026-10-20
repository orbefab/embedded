//! TWI master, DS40002061B §22, transaction level.
//!
//! A byte phase is exactly `9·T` from the `TWINT` write to `TWINT` set
//! (§22.3.3, §22.3.4, §22.5.5). START, repeated START, and STOP take `T`.
//! This is a Bounded choice, ±T: Table 29-14 gives only minimums. `T` is
//! `16 + 2·TWBR·4^TWPS` (§22.5.2). The
//! write's cycle is the store's retire cycle, the same convention as an
//! `SPDR` store. The bus waits while `TWINT` is set (§22.5.5). SCL and SDA
//! edges are not modeled.
//!
//! Slave modes, arbitration loss, bus error, and general call are stored
//! nowhere: a master transfer does not produce those status codes.

use std::collections::VecDeque;

use crate::bus::hostside::HostSide;
use crate::chip::{OcPin, TwiCfg};
use crate::mux::PinMux;
use crate::periph::{Ctx, Gate, Touch};

use crate::regs::{
    TWAMR as REG_TWAMR, TWAR as REG_TWAR, TWBR as REG_TWBR, TWCR as REG_TWCR, TWDR as REG_TWDR,
    TWSR as REG_TWSR,
};

const TWINT: u8 = 1 << 7;
const TWEA: u8 = 1 << 6;
const TWSTA: u8 = 1 << 5;
const TWSTO: u8 = 1 << 4;
const TWWC: u8 = 1 << 3;
const TWEN: u8 = 1 << 2;
const TWIE: u8 = 1 << 0;

/// Control bits a program can store. `TWINT` and `TWWC` are separate.
const TWCR_STORE: u8 = TWEA | TWSTA | TWSTO | TWEN | TWIE;

/// One completed host phase. Stamped when `TWINT` sets, or when `TWSTO` clears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I2cNote {
    Start { repeated: bool },
    Address { addr: u8, read: bool, ack: bool },
    Data { byte: u8, write: bool, ack: bool },
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct I2cStamp {
    pub at: u64,
    pub note: I2cNote,
}

/// What the retiring `TWCR` write asked the bus to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Start {
        repeated: bool,
        /// Status was 0x08, 0x10, 0x40, or 0x50. No START row for these in
        /// Tables 22-2 and 22-3. Logged once.
        off_table: bool,
    },
    Address,
    DataTx,
    DataRx,
    Stop,
    StopThenStart,
    /// No row in Tables 22-2 and 22-3 for this write. Logged once.
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Start {
        repeated: bool,
    },
    Address {
        addr: u8,
        read: bool,
        ack: bool,
    },
    DataTx {
        byte: u8,
        ack: bool,
    },
    DataRx {
        byte: u8,
    },
    Stop,
    /// STOP at `due`, then a new START at `start_at`.
    StopThenStart {
        start_at: u64,
    },
}

/// SCL period in CPU cycles. `4^TWPS` is `1 << (2·TWPS)`.
pub(crate) fn scl_period(twbr: u8, twps: u8) -> u64 {
    let scale = 1u64 << (2 * u32::from(twps & 0x03));
    16 + 2 * u64::from(twbr) * scale
}

#[derive(Debug, Clone)]
pub(crate) struct Twi {
    twbr: u8,
    twps: u8,
    twar: u8,
    twdr: u8,
    /// `TWEA`, `TWSTA`, `TWSTO`, `TWEN`, `TWIE`. Not `TWINT` or `TWWC`.
    ctrl: u8,
    twint: bool,
    twwc: bool,
    /// Status code while `TWINT` is set. Reads as `0xF8` otherwise.
    status: u8,
    /// A START was sent and no STOP or `TWEN` = 0 has happened since.
    owns: bool,
    /// The last SLA+R/W was acknowledged. Data after a NACK goes nowhere.
    addressed: bool,
    last_addr: u8,
    due: u64,
    job: Option<Job>,
    armed: Option<Action>,
    clock_on: bool,
    prr_off: bool,
    frozen_at: u64,
    sda: OcPin,
    scl: OcPin,
    vec: u8,
    out: VecDeque<I2cStamp>,
    logged_speed: bool,
    logged_off: bool,
    logged_invalid: bool,
    logged_twar: bool,
    logged_twamr: bool,
    twamr: u8,
    /// CPU clock, so the 400 kHz check is `f_CK / T`.
    cpu_hz: u32,
}

impl Twi {
    pub(crate) fn new(cfg: &TwiCfg) -> Self {
        Self {
            twbr: 0,
            twps: 0,
            twar: 0xFE,
            twdr: 0xFF,
            ctrl: 0,
            twint: false,
            twwc: false,
            status: 0xF8,
            owns: false,
            addressed: false,
            last_addr: 0,
            due: u64::MAX,
            job: None,
            armed: None,
            clock_on: true,
            prr_off: false,
            frozen_at: 0,
            sda: cfg.sda,
            scl: cfg.scl,
            vec: cfg.vec,
            out: VecDeque::new(),
            logged_speed: false,
            logged_off: false,
            logged_invalid: false,
            logged_twar: false,
            logged_twamr: false,
            twamr: 0,
            cpu_hz: 16_000_000,
        }
    }

    pub(crate) fn set_cpu_hz(&mut self, hz: u32) {
        self.cpu_hz = hz.max(1);
    }

    fn running(&self) -> bool {
        self.clock_on && !self.prr_off
    }

    fn period(&self) -> u64 {
        scl_period(self.twbr, self.twps)
    }

    fn enabled(&self) -> bool {
        self.ctrl & TWEN != 0
    }

    pub(crate) fn set_clock_gate(&mut self, io: Option<bool>, prr: Option<bool>, cycle: u64) {
        let delta = Gate {
            clock_on: &mut self.clock_on,
            prr_off: &mut self.prr_off,
            frozen_at: &mut self.frozen_at,
        }
        .set(io, prr, cycle);
        if let Some(delta) = delta
            && self.due != u64::MAX
        {
            self.due = self.due.wrapping_add(delta);
        }
    }

    pub(crate) fn take(&mut self) -> Option<I2cStamp> {
        self.out.pop_front()
    }

    pub(crate) fn peek(&self, reg: u8) -> u8 {
        match reg {
            REG_TWBR => self.twbr,
            REG_TWSR => self.twsr(),
            REG_TWAR => self.twar,
            REG_TWDR => self.twdr,
            REG_TWCR => self.twcr_read(),
            REG_TWAMR => self.twamr & !1,
            _ => 0,
        }
    }

    fn twsr(&self) -> u8 {
        let prescale = self.twps & 0x03;
        if self.twint {
            (self.status & 0xF8) | prescale
        } else {
            0xF8 | prescale
        }
    }

    fn twcr_read(&self) -> u8 {
        let mut value = self.ctrl & TWCR_STORE;
        if self.twint {
            value |= TWINT;
        }
        if self.twwc {
            value |= TWWC;
        }
        value
    }

    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        cycle: u64,
    ) -> Touch {
        if !self.running() {
            return Touch::default();
        }
        self.store(reg, value, mux, ctx, cycle, false)
    }

    pub(crate) fn poke(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        cycle: u64,
        host: &mut HostSide,
    ) -> Touch {
        if !self.running() {
            return Touch::default();
        }
        let touch = self.store(reg, value, mux, ctx, cycle, true);
        if self.armed.is_some() {
            self.launch(cycle, mux, ctx, host);
        }
        touch
    }

    /// The store has retired. The phase is measured from `cycle`.
    pub(crate) fn stamp(
        &mut self,
        cycle: u64,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        host: &mut HostSide,
    ) {
        if self.armed.is_some() {
            self.launch(cycle, mux, ctx, host);
        }
    }

    fn store(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        cycle: u64,
        immediate: bool,
    ) -> Touch {
        let mut touch = Touch {
            irq: true,
            schedule: true,
            ..Touch::default()
        };
        match reg {
            REG_TWBR => self.twbr = value,
            REG_TWSR => self.twps = value & 0x03,
            REG_TWAR => {
                self.twar = value;
                log_once(ctx, &mut self.logged_twar, 0x00BA, value);
            }
            REG_TWDR => {
                if self.twint {
                    self.twdr = value;
                    self.twwc = false;
                } else {
                    self.twwc = true;
                }
            }
            REG_TWCR => {
                self.write_twcr(value, mux, ctx, cycle);
                if !immediate {
                    touch.stamp = self.armed.is_some();
                }
            }
            REG_TWAMR => {
                self.twamr = value & !1;
                log_once(ctx, &mut self.logged_twamr, 0x00BD, value);
            }
            _ => {}
        }
        touch
    }

    fn write_twcr(&mut self, value: u8, mux: &mut PinMux, ctx: &mut Ctx<'_>, cycle: u64) {
        let start_bit = value & TWINT != 0;
        let was_on = self.enabled() || self.job.is_some() || self.owns || self.twint;
        let status = self.status;
        let owns = self.owns;
        self.ctrl = value & TWCR_STORE;
        self.apply_pads(mux, ctx, cycle);
        if !self.enabled() {
            if was_on {
                self.abort();
            }
            self.armed = None;
            return;
        }
        if self.job.is_some() {
            // The in-flight phase finishes on its own. A `TWEN` clear returned above (§22.9.2).
            self.armed = None;
            return;
        }
        // Idle `TWSTO` sends nothing and reads 0 after the write (§22.9.2).
        if !owns && self.ctrl & TWSTO != 0 {
            self.ctrl &= !TWSTO;
        }
        // Writing 0 leaves `TWINT`. Nothing starts while it stays set (§22.3.2).
        if !start_bit {
            return;
        }
        // Bit 7 clears `TWINT` and the other bits are already stored, so the
        // interrupt request is the result of this write (§22.9.2).
        self.twint = false;
        self.armed = Some(self.choose(status, owns));
    }

    fn choose(&mut self, status: u8, owns: bool) -> Action {
        let sta = self.ctrl & TWSTA != 0;
        let sto = self.ctrl & TWSTO != 0;
        if sta && !sto {
            let off_table = matches!(status, 0x08 | 0x10 | 0x40 | 0x50);
            return Action::Start {
                repeated: owns,
                off_table: owns && off_table,
            };
        }
        if !owns {
            return Action::Invalid;
        }
        if !sta && !sto {
            return match status {
                0x08 | 0x10 => Action::Address,
                0x18 | 0x20 | 0x28 | 0x30 => Action::DataTx,
                0x40 | 0x50 => Action::DataRx,
                _ => Action::Invalid,
            };
        }
        if !sta && sto {
            return match status {
                0x18 | 0x20 | 0x28 | 0x30 | 0x48 | 0x58 => Action::Stop,
                _ => Action::Invalid,
            };
        }
        match status {
            0x18 | 0x20 | 0x28 | 0x30 | 0x48 | 0x58 => Action::StopThenStart,
            _ => Action::Invalid,
        }
    }

    fn launch(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>, host: &mut HostSide) {
        let Some(action) = self.armed.take() else {
            return;
        };
        if !self.running() || !self.enabled() {
            self.abort();
            return;
        }
        let period = self.period();
        if !matches!(action, Action::Invalid) {
            self.note_speed(ctx);
        }
        match action {
            Action::Start {
                repeated,
                off_table,
            } => {
                if off_table {
                    let value = self.ctrl | TWINT;
                    log_once(ctx, &mut self.logged_off, 0x00BC, value);
                }
                self.job = Some(Job::Start { repeated });
                self.due = at.saturating_add(period);
            }
            Action::Address => {
                let sla = self.twdr;
                let addr = sla >> 1;
                let read = sla & 1 == 1;
                let ack = host.i2c_address(addr);
                self.addressed = ack;
                self.last_addr = addr;
                self.job = Some(Job::Address { addr, read, ack });
                self.due = at.saturating_add(9 * period);
            }
            Action::DataTx => {
                let byte = self.twdr;
                let ack = self.addressed && host.i2c_write_data(self.last_addr);
                self.job = Some(Job::DataTx { byte, ack });
                self.due = at.saturating_add(9 * period);
            }
            Action::DataRx => {
                let byte = if self.addressed {
                    host.i2c_read_data(self.last_addr, ctx)
                } else {
                    host.i2c_idle_read(ctx)
                };
                self.job = Some(Job::DataRx { byte });
                self.due = at.saturating_add(9 * period);
            }
            Action::Stop => {
                self.job = Some(Job::Stop);
                self.due = at.saturating_add(period);
            }
            Action::StopThenStart => {
                let start_at = at.saturating_add(2 * period);
                self.job = Some(Job::StopThenStart { start_at });
                self.due = at.saturating_add(period);
            }
            Action::Invalid => {
                let value = self.ctrl;
                log_once(ctx, &mut self.logged_invalid, 0x00BC, value);
                self.abort();
                self.apply_pads(mux, ctx, at);
            }
        }
    }

    fn abort(&mut self) {
        self.job = None;
        self.due = u64::MAX;
        self.armed = None;
        self.twint = false;
        self.ctrl &= !TWSTO;
        self.owns = false;
        self.addressed = false;
    }

    /// `running` masks the request while `PRTWI` is set or clkI/O is stopped.
    /// Vector 25's wake class is the TWI address match, so a pending master
    /// `TWINT` would otherwise wake power-down. SPI leaves its request up
    /// while frozen because that vector is other I/O, which power-down ignores.
    pub(crate) fn irq_bits(&self) -> u64 {
        if self.twint && self.ctrl & TWIE != 0 && self.running() {
            1u64 << self.vec
        } else {
            0
        }
    }

    /// Level request. Entry does not clear `TWINT` (§22.9.2).
    pub(crate) fn ack(&mut self, _vector: u8) {}

    pub(crate) fn next_event(&self) -> u64 {
        if self.running() { self.due } else { u64::MAX }
    }

    pub(crate) fn fire(&mut self, at: u64) {
        if !self.running() || self.due != at {
            return;
        }
        let Some(job) = self.job.take() else {
            self.due = u64::MAX;
            return;
        };
        match job {
            Job::Start { repeated } => {
                self.owns = true;
                self.finish_wait(if repeated { 0x10 } else { 0x08 });
                self.push(at, I2cNote::Start { repeated });
            }
            Job::Address { addr, read, ack } => {
                let status = match (read, ack) {
                    (false, true) => 0x18,
                    (false, false) => 0x20,
                    (true, true) => 0x40,
                    (true, false) => 0x48,
                };
                self.finish_wait(status);
                self.push(at, I2cNote::Address { addr, read, ack });
            }
            Job::DataTx { byte, ack } => {
                self.finish_wait(if ack { 0x28 } else { 0x30 });
                self.push(
                    at,
                    I2cNote::Data {
                        byte,
                        write: true,
                        ack,
                    },
                );
            }
            Job::DataRx { byte } => {
                let ack = self.ctrl & TWEA != 0;
                self.twdr = byte;
                self.finish_wait(if ack { 0x50 } else { 0x58 });
                self.push(
                    at,
                    I2cNote::Data {
                        byte,
                        write: false,
                        ack,
                    },
                );
            }
            Job::Stop => {
                self.ctrl &= !TWSTO;
                self.owns = false;
                self.addressed = false;
                self.twint = false;
                self.due = u64::MAX;
                self.push(at, I2cNote::Stop);
            }
            Job::StopThenStart { start_at } => {
                self.ctrl &= !TWSTO;
                self.owns = false;
                self.addressed = false;
                self.twint = false;
                self.push(at, I2cNote::Stop);
                self.job = Some(Job::Start { repeated: false });
                self.due = start_at;
            }
        }
    }

    fn finish_wait(&mut self, status: u8) {
        self.status = status;
        self.twint = true;
        self.due = u64::MAX;
        self.job = None;
    }

    fn push(&mut self, at: u64, note: I2cNote) {
        self.out.push_back(I2cStamp { at, note });
    }

    fn apply_pads(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        let mask = if self.enabled() {
            (1u8 << self.sda.bit) | (1u8 << self.scl.bit)
        } else {
            0
        };
        mux.set_twi_override(self.sda.port, mask, ctx, Some(at), at);
    }

    /// Out of spec when `f_SCL = f_CK / T` is above 400 kHz (Table 29-14).
    /// Checked on the period the phase actually uses, once.
    fn note_speed(&mut self, ctx: &mut Ctx<'_>) {
        let period = self.period();
        if period > 0 && u64::from(self.cpu_hz) > 400_000 * period {
            log_once(ctx, &mut self.logged_speed, 0x00B8, self.twbr);
        }
    }
}

fn log_once(ctx: &mut Ctx<'_>, flag: &mut bool, addr: u16, value: u8) {
    if *flag {
        return;
    }
    *flag = true;
    ctx.log(addr, value);
}
