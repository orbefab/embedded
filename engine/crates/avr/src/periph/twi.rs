//! TWI master, DS40002061B §22, at bit level on `SCL` and `SDA`.
//!
//! The TWI pulls each line low through the pin's low stage and releases it otherwise
//! (Table 14-7). It reads both lines from pad edges through `AIO`, with no `PINx`
//! synchronizer. The part suppresses a spike under 50 ns. The model has no filter,
//! so it passes any pulse the circuit resolves (§22.5.1).
//!
//! The datasheet gives the period and the minimums of Table 29-14, not where the edges sit
//! inside a bit. This bit table is a choice that meets every timing row of Table 29-14
//! from 100 kHz to 400 kHz. `T` is `16 + 2·TWBR·4^TWPS` (§22.5.2), `S = ⌊0.45·T⌋`, `L = T - S`
//! and `D = ⌊T/16⌋`. A length after the TWI releases a line counts from the cycle it reads that
//! line high, so a device that holds SCL low stretches the bit (§22.4). A length after the TWI
//! pulls a line low counts from that cycle, and another master's earlier SCL fall pulls SCL
//! low at once.
//!
//! - START on a free bus: SDA low at `w + L`, SCL low `S` later.
//! - Each bit after the SCL fall `f`: SDA at `f + D`, SCL released at `f + L`, SDA sampled
//!   when SCL reads high, SCL low `S` after that.
//! - STOP: SDA low at `w + D`, SCL released at `w + L`, SDA released `S` after SCL reads high.
//! - Repeated START: SDA released at `w + D`, SCL released at `w + L`, SDA low `L` after SCL
//!   reads high, SCL low `S` after that.
//!
//! `w` is the `TWCR` write's retire cycle, the same convention as an `SPDR` store. SCL stays low
//! while `TWINT` is set (§22.5.5). A master read releases SDA for the eight data bits and drives
//! the ninth from `TWEA` (§22.7.2). A 1 the master sends on an address bit, a write data bit, or
//! that ninth bit, read back as 0, loses arbitration, 0x38. An SDA change while SCL is high
//! inside a byte is a bus error, 0x00 (§22.4, §22.8). A START asked for while another master
//! holds the bus waits for its STOP.
//!
//! Slave modes, general call and `TWAMR` matching are not modelled, so this TWI never answers
//! its own address.

use std::collections::VecDeque;

use crate::Level;
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

/// One phase this master saw on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I2cNote {
    Start {
        repeated: bool,
    },
    Address {
        addr: u8,
        read: bool,
        ack: bool,
    },
    Data {
        byte: u8,
        write: bool,
        ack: bool,
    },
    Stop,
    /// A transmitted 1 read back as 0, status 0x38.
    ArbitrationLost,
    /// A START or STOP inside an address or data byte, status 0x00.
    BusError,
}

/// `start` is the phase's first bus change, the first SCL or SDA edge the TWI read after the
/// cycle the previous phase ended while it owned the bus or had a phase under way, or `at` when
/// there was none. `at` is the cycle the phase
/// ended: `TWINT` set, or `TWSTO` cleared. Both are oscillator cycles from power-on, as a
/// USART's [`crate::Sent`] is, so a reset and a clock prescaler leave them in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct I2cStamp {
    pub start: u64,
    pub at: u64,
    pub note: I2cNote,
}

/// What the retiring `TWCR` write asked the bus to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Start {
        repeated: bool,
    },
    Address,
    DataTx,
    DataRx,
    Stop,
    StopThenStart,
    /// No row in Tables 22-2 or 22-3. Logged once, and this master releases its lines.
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    Scl,
    Sda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Address,
    Write,
    Read,
}

/// The byte on the bus: what it sends, and what it has read so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Byte {
    kind: Kind,
    out: u8,
    got: u8,
    /// 0 to 8. Bit 8 is the acknowledge.
    bit: u8,
    ack: bool,
}

/// The next bus action of a phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// A START waits for another master's STOP.
    Busy,
    /// SDA falls for a START.
    StartSda { repeated: bool },
    /// SCL falls `S` after SDA, and the START ends.
    StartScl { repeated: bool },
    /// A repeated START releases SDA at `w + D`.
    RestartSda,
    /// A repeated START releases SCL at `w + L`.
    RestartScl,
    /// SDA takes the bit at `f + D`.
    Data(Byte),
    /// SCL is released at `f + L`.
    Release(Byte),
    /// SCL reads high: the bit is sampled.
    Sample(Byte),
    /// SCL falls `S` after it read high.
    Fall(Byte),
    /// SDA falls at `w + D` for a STOP.
    StopSda { then_start: bool },
    /// SCL is released at `w + L`.
    StopScl { then_start: bool },
    /// SDA is released `S` after SCL reads high.
    StopRise { then_start: bool },
    /// SDA reads high, and the STOP ends.
    StopEnd { then_start: bool },
}

/// `step` runs `delay` cycles after `line` reads `high`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sense {
    line: Line,
    high: bool,
    delay: u64,
}

/// SDA and SCL edges of one pad batch, in the order they took effect.
const EDGE_CAP: usize = 8;

/// SCL period in CPU cycles. `4^TWPS` is `1 << (2·TWPS)`.
pub(crate) fn scl_period(twbr: u8, twps: u8) -> u64 {
    let scale = 1u64 << (2 * u32::from(twps & 0x03));
    16 + 2 * u64::from(twbr) * scale
}

/// `(S, L, D)` of the bit table for period `t`.
pub(crate) fn lengths(t: u64) -> (u64, u64, u64) {
    let short = t * 45 / 100;
    (short, t - short, t / 16)
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
    /// A START of this master and no STOP, loss or `TWEN` = 0 since.
    owns: bool,
    /// A START on the bus and no STOP since, from any master.
    busy: bool,
    step: Option<Step>,
    /// The cycle `step` runs, `u64::MAX` while it waits on `sense` or nothing.
    due: u64,
    sense: Option<Sense>,
    armed: Option<Action>,
    /// What the TWI pulls low.
    sda_low: bool,
    scl_low: bool,
    /// The levels it last read.
    sda_high: bool,
    scl_high: bool,
    /// The first edge read after the cycle the last phase ended.
    first: Option<u64>,
    ended: Option<u64>,
    edges: [(Line, bool, u64); EDGE_CAP],
    edge_len: u8,
    clock_on: bool,
    prr_off: bool,
    frozen_at: u64,
    sda: OcPin,
    scl: OcPin,
    vec: u8,
    out: VecDeque<I2cStamp>,
    logged_speed: bool,
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
            busy: false,
            step: None,
            due: u64::MAX,
            sense: None,
            armed: None,
            sda_low: false,
            scl_low: false,
            sda_high: false,
            scl_high: false,
            first: None,
            ended: None,
            edges: [(Line::Scl, false, 0); EDGE_CAP],
            edge_len: 0,
            clock_on: true,
            prr_off: false,
            frozen_at: 0,
            sda: cfg.sda,
            scl: cfg.scl,
            vec: cfg.vec,
            out: VecDeque::new(),
            logged_speed: false,
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
        if self.running() {
            self.resume(cycle);
        }
    }

    /// A level read or a STOP seen while frozen starts its step from the resume.
    fn resume(&mut self, cycle: u64) {
        if self.step == Some(Step::Busy) && !self.busy {
            let (_, long, _) = lengths(self.period());
            self.at(Step::StartSda { repeated: false }, cycle + long);
        }
        if let Some(sense) = self.sense
            && self.reads(sense.line) == sense.high
        {
            self.sense = None;
            self.due = cycle + sense.delay;
        }
    }

    /// The next phase that ended at or before oscillator cycle `through`.
    pub(crate) fn take(&mut self, through: u64) -> Option<I2cStamp> {
        self.out.pop_front_if(|stamp| stamp.at <= through)
    }

    /// The phases that ended, which an in-run reset keeps.
    pub(crate) fn take_all(&mut self) -> VecDeque<I2cStamp> {
        std::mem::take(&mut self.out)
    }

    pub(crate) fn restore(&mut self, kept: VecDeque<I2cStamp>) {
        self.out = kept;
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

    /// `TWINT`, the status and a received `TWDR` change at an event or a pad edge.
    pub(crate) fn steady(&self, reg: u8) -> bool {
        matches!(
            reg,
            REG_TWBR | REG_TWSR | REG_TWAR | REG_TWDR | REG_TWCR | REG_TWAMR
        )
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
    ) -> Touch {
        if !self.running() {
            return Touch::default();
        }
        let touch = self.store(reg, value, mux, ctx, cycle, true);
        if self.armed.is_some() {
            self.launch(cycle, mux, ctx);
        }
        touch
    }

    /// The store has retired. The phase is measured from `cycle`.
    pub(crate) fn stamp(&mut self, cycle: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        if self.armed.is_some() {
            self.launch(cycle, mux, ctx);
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
        let was_enabled = self.enabled();
        let status = if self.twint { self.status } else { 0xF8 };
        let owns = self.owns;
        self.ctrl = value & TWCR_STORE;
        if !self.enabled() {
            if was_enabled {
                self.abort();
            }
            self.apply_pads(mux, ctx, cycle);
            self.armed = None;
            return;
        }
        if !was_enabled {
            self.sda_high = mux.pad_high(self.sda.port, self.sda.bit);
            self.scl_high = mux.pad_high(self.scl.port, self.scl.bit);
            self.busy = false;
            self.first = None;
        }
        self.apply_pads(mux, ctx, cycle);
        if self.step.is_some() {
            // The in-flight phase finishes on its own. A `TWEN` clear returned above (§22.9.2).
            self.armed = None;
            return;
        }
        let stop = self.ctrl & TWSTO != 0;
        // Idle `TWSTO` sends nothing and reads 0 after the write (§22.9.2).
        if !owns && stop {
            self.ctrl &= !TWSTO;
        }
        // Writing 0 leaves `TWINT`. Nothing starts while it stays set (§22.3.2).
        if !start_bit {
            return;
        }
        // Bit 7 clears `TWINT` and the other bits are already stored, so the
        // interrupt request is the result of this write (§22.9.2).
        self.twint = false;
        let start = self.ctrl & TWSTA != 0;
        // Tables 22-2 and 22-3, 0x38 with neither bit: the bus is released. Table 22-6, 0x00
        // with `TWSTO`: only the hardware recovers, and no STOP goes on the bus.
        if !owns && !start && (status == 0x38 || (status == 0x00 && stop)) {
            return;
        }
        self.armed = Some(self.choose(status, owns));
    }

    fn choose(&mut self, status: u8, owns: bool) -> Action {
        let sta = self.ctrl & TWSTA != 0;
        let sto = self.ctrl & TWSTO != 0;
        if sta && !sto {
            // 0x08, 0x10, 0x40 and 0x50 have no START row. They take the same path as
            // every other missing row.
            if owns && matches!(status, 0x08 | 0x10 | 0x40 | 0x50) {
                return Action::Invalid;
            }
            return Action::Start { repeated: owns };
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

    fn launch(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let Some(action) = self.armed.take() else {
            return;
        };
        if !self.running() || !self.enabled() {
            self.abort();
            return;
        }
        if !matches!(action, Action::Invalid) {
            self.note_speed(ctx);
        }
        let (_, long, data) = lengths(self.period());
        let byte = |kind, out| Byte {
            kind,
            out,
            got: 0,
            bit: 0,
            ack: false,
        };
        match action {
            Action::Start { repeated } => {
                if repeated {
                    self.at(Step::RestartSda, at + data);
                } else if self.busy {
                    self.step = Some(Step::Busy);
                    self.due = u64::MAX;
                } else {
                    self.at(Step::StartSda { repeated: false }, at + long);
                }
            }
            Action::Address => self.at(Step::Data(byte(Kind::Address, self.twdr)), at + data),
            Action::DataTx => self.at(Step::Data(byte(Kind::Write, self.twdr)), at + data),
            Action::DataRx => self.at(Step::Data(byte(Kind::Read, 0xFF)), at + data),
            Action::Stop => self.at(Step::StopSda { then_start: false }, at + data),
            Action::StopThenStart => self.at(Step::StopSda { then_start: true }, at + data),
            Action::Invalid => {
                let value = self.ctrl;
                log_once(ctx, &mut self.logged_invalid, 0x00BC, value);
                // One path for every missing row. This master releases its lines.
                // `busy` clears only when it owned the bus. Otherwise the SDA edge
                // while SCL is high keeps it.
                let owned = self.owns;
                self.abort();
                if owned {
                    self.busy = false;
                }
                self.drive(Some(false), Some(false), mux, ctx, at);
            }
        }
    }

    fn at(&mut self, step: Step, due: u64) {
        self.step = Some(step);
        self.sense = None;
        self.due = due;
    }

    /// `step` runs `delay` cycles after `line` reads `high`.
    fn when(&mut self, step: Step, line: Line, high: bool, delay: u64) {
        self.step = Some(step);
        self.due = u64::MAX;
        self.sense = Some(Sense { line, high, delay });
    }

    fn abort(&mut self) {
        self.step = None;
        self.sense = None;
        self.due = u64::MAX;
        self.armed = None;
        self.twint = false;
        self.ctrl &= !TWSTO;
        self.owns = false;
        // The next phase must not keep an edge recorded after the last one ended.
        self.first = None;
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

    pub(crate) fn fire(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        if !self.running() || self.due != at {
            return;
        }
        self.due = u64::MAX;
        if let Some(step) = self.step {
            self.act(step, at, mux, ctx);
        }
    }

    pub(crate) fn on_pad(&mut self, pin: u8, level: Level, at: u64) -> Touch {
        if !self.enabled() {
            return Touch::default();
        }
        let line = if u32::from(pin) == self.scl.port.pin_id(self.scl.bit) {
            Line::Scl
        } else if u32::from(pin) == self.sda.port.pin_id(self.sda.bit) {
            Line::Sda
        } else {
            return Touch::default();
        };
        let index = usize::from(self.edge_len);
        debug_assert!(
            index < EDGE_CAP,
            "more bus edges in one batch than the TWI keeps"
        );
        if index < EDGE_CAP {
            self.edges[index] = (line, level == Level::High, at);
            self.edge_len += 1;
        }
        Touch {
            schedule: true,
            irq: true,
            ..Touch::default()
        }
    }

    pub(crate) fn after_pad(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>) -> Touch {
        let n = usize::from(self.edge_len);
        if n == 0 {
            return Touch::default();
        }
        self.edge_len = 0;
        for k in 0..n {
            let (line, high, at) = self.edges[k];
            self.on_edge(line, high, at, mux, ctx);
        }
        Touch {
            schedule: true,
            irq: true,
            ..Touch::default()
        }
    }

    fn on_edge(&mut self, line: Line, high: bool, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let was = self.reads(line);
        match line {
            Line::Scl => self.scl_high = high,
            Line::Sda => self.sda_high = high,
        }
        if was == high {
            return;
        }
        // An edge on the cycle a phase ended belongs to that phase, as the edge ending it does.
        // Another master's traffic while this one waits or has lost belongs to no phase of its.
        let mine = self.owns || !matches!(self.step, None | Some(Step::Busy));
        if mine && self.ended.is_none_or(|end| at > end) {
            self.first.get_or_insert(ctx.oscillator_cycles(at));
        }
        if line == Line::Sda && self.scl_high {
            self.busy = !high;
        }
        if !self.running() {
            return;
        }
        match (self.step, line, high) {
            (Some(Step::Fall(_) | Step::Sample(_)), Line::Sda, _) if self.scl_high => {
                self.bus_error(at, mux, ctx);
                return;
            }
            (Some(Step::Busy), Line::Sda, true) if self.scl_high => {
                let (_, long, _) = lengths(self.period());
                self.at(Step::StartSda { repeated: false }, at + long);
                return;
            }
            // Another master's START: this one joins it (§22.4).
            (Some(step @ Step::StartSda { repeated: false }), Line::Sda, false)
                if self.scl_high =>
            {
                self.act(step, at, mux, ctx);
                return;
            }
            // Another master pulled SCL low first: this one pulls it low too (§22.4).
            (Some(step @ (Step::Fall(_) | Step::StartScl { .. })), Line::Scl, false) => {
                self.act(step, at, mux, ctx);
                return;
            }
            _ => {}
        }
        if let (Some(step), Some(sense)) = (self.step, self.sense)
            && sense.line == line
            && sense.high == high
        {
            self.sense = None;
            if sense.delay == 0 {
                self.act(step, at, mux, ctx);
            } else {
                self.due = at + sense.delay;
            }
        }
    }

    fn reads(&self, line: Line) -> bool {
        match line {
            Line::Scl => self.scl_high,
            Line::Sda => self.sda_high,
        }
    }

    /// Runs `step` at `at`, and sets the next one.
    fn act(&mut self, step: Step, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let (short, long, data) = lengths(self.period());
        match step {
            Step::Busy => {}
            Step::StartSda { repeated } => {
                self.drive(Some(true), None, mux, ctx, at);
                self.at(Step::StartScl { repeated }, at + short);
            }
            Step::StartScl { repeated } => {
                self.drive(None, Some(true), mux, ctx, at);
                self.owns = true;
                self.finish(if repeated { 0x10 } else { 0x08 });
                self.push(at, I2cNote::Start { repeated }, ctx);
            }
            Step::RestartSda => {
                self.drive(Some(false), None, mux, ctx, at);
                self.at(Step::RestartScl, at + (long - data));
            }
            Step::RestartScl => {
                self.drive(None, Some(false), mux, ctx, at);
                self.when(Step::StartSda { repeated: true }, Line::Scl, true, long);
            }
            Step::Data(byte) => {
                let release = match (byte.kind, byte.bit) {
                    (Kind::Read, 8) => self.ctrl & TWEA == 0,
                    (Kind::Read, _) | (_, 8) => true,
                    (_, bit) => byte.out & (0x80 >> bit) != 0,
                };
                self.drive(Some(!release), None, mux, ctx, at);
                self.at(Step::Release(byte), at + (long - data));
            }
            Step::Release(byte) => {
                self.drive(None, Some(false), mux, ctx, at);
                self.when(Step::Sample(byte), Line::Scl, true, 0);
            }
            Step::Sample(mut byte) => {
                let high = self.sda_high;
                let sent = match byte.kind {
                    Kind::Read => byte.bit == 8,
                    Kind::Address | Kind::Write => byte.bit < 8,
                };
                let sent_one = sent && !self.sda_low;
                if sent_one && !high {
                    if byte.kind == Kind::Read && byte.bit == 8 {
                        self.twdr = byte.got;
                    }
                    self.drive(Some(false), Some(false), mux, ctx, at);
                    self.owns = false;
                    self.finish(0x38);
                    self.push(at, I2cNote::ArbitrationLost, ctx);
                    return;
                }
                match (byte.kind, byte.bit) {
                    (Kind::Read, 8) => byte.ack = self.sda_low,
                    (_, 8) => byte.ack = !high,
                    (_, bit) if high => byte.got |= 0x80 >> bit,
                    _ => {}
                }
                self.at(Step::Fall(byte), at + short);
            }
            Step::Fall(byte) => {
                self.drive(None, Some(true), mux, ctx, at);
                if byte.bit < 8 {
                    self.at(
                        Step::Data(Byte {
                            bit: byte.bit + 1,
                            ..byte
                        }),
                        at + data,
                    );
                } else {
                    self.end_byte(byte, at, ctx);
                }
            }
            Step::StopSda { then_start } => {
                self.drive(Some(true), None, mux, ctx, at);
                self.at(Step::StopScl { then_start }, at + (long - data));
            }
            Step::StopScl { then_start } => {
                self.drive(None, Some(false), mux, ctx, at);
                self.when(Step::StopRise { then_start }, Line::Scl, true, short);
            }
            Step::StopRise { then_start } => {
                self.drive(Some(false), None, mux, ctx, at);
                self.when(Step::StopEnd { then_start }, Line::Sda, true, 0);
            }
            Step::StopEnd { then_start } => {
                self.ctrl &= !TWSTO;
                self.owns = false;
                self.twint = false;
                self.push(at, I2cNote::Stop, ctx);
                if then_start {
                    self.at(Step::StartSda { repeated: false }, at + long);
                } else {
                    self.step = None;
                }
            }
        }
    }

    fn end_byte(&mut self, byte: Byte, at: u64, ctx: &Ctx<'_>) {
        let ack = byte.ack;
        let read = byte.out & 1 == 1;
        let (status, note) = match byte.kind {
            Kind::Address => (
                match (read, ack) {
                    (false, true) => 0x18,
                    (false, false) => 0x20,
                    (true, true) => 0x40,
                    (true, false) => 0x48,
                },
                I2cNote::Address {
                    addr: byte.out >> 1,
                    read,
                    ack,
                },
            ),
            Kind::Write => (
                if ack { 0x28 } else { 0x30 },
                I2cNote::Data {
                    byte: byte.out,
                    write: true,
                    ack,
                },
            ),
            Kind::Read => {
                self.twdr = byte.got;
                (
                    if ack { 0x50 } else { 0x58 },
                    I2cNote::Data {
                        byte: byte.got,
                        write: false,
                        ack,
                    },
                )
            }
        };
        self.finish(status);
        self.push(at, note, ctx);
    }

    /// A START or STOP inside a byte. Both lines are released (§22.8, Table 22-6).
    fn bus_error(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        self.drive(Some(false), Some(false), mux, ctx, at);
        self.owns = false;
        self.finish(0x00);
        self.push(at, I2cNote::BusError, ctx);
    }

    fn finish(&mut self, status: u8) {
        self.status = status;
        self.twint = true;
        self.step = None;
        self.sense = None;
        self.due = u64::MAX;
    }

    fn push(&mut self, at: u64, note: I2cNote, ctx: &Ctx<'_>) {
        let end = ctx.oscillator_cycles(at);
        let start = self.first.take().unwrap_or(end);
        self.ended = Some(at);
        self.out.push_back(I2cStamp {
            start,
            at: end,
            note,
        });
    }

    /// `Some(true)` pulls the line low, `Some(false)` releases it, `None` leaves it.
    fn drive(
        &mut self,
        sda: Option<bool>,
        scl: Option<bool>,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        at: u64,
    ) {
        if let Some(low) = sda {
            self.sda_low = low;
        }
        if let Some(low) = scl {
            self.scl_low = low;
        }
        self.apply_pads(mux, ctx, at);
    }

    fn apply_pads(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        let (sda, scl) = ((1u8 << self.sda.bit), (1u8 << self.scl.bit));
        let (mask, low) = if self.enabled() {
            let low = if self.sda_low { sda } else { 0 } | if self.scl_low { scl } else { 0 };
            (sda | scl, low)
        } else {
            self.sda_low = false;
            self.scl_low = false;
            (0, 0)
        };
        mux.set_twi_override(self.sda.port, mask, low, ctx, Some(at), at);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// DS40002061B Table 29-14 at TWBR 72 and 12 on a 16 MHz clock, from the plan's table.
    #[test]
    fn the_bit_table_at_100_and_400_khz() {
        assert_eq!(lengths(scl_period(72, 0)), (72, 88, 10));
        assert_eq!(lengths(scl_period(12, 0)), (18, 22, 2));
    }
}
