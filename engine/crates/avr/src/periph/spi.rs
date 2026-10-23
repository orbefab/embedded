//! SPI, DS40002061B §19.
//!
//! §19.2: writing `SPDR` starts the SPI clock generator, and the generator
//! stops after one byte, setting `SPIF`. It is not the USART's free-running
//! divider. Figure 19-1 selects `/2` through `/128`. Table 19-5 is that
//! period in CPU cycles (`SPI2X` and `SPR1:0`). The figures show a square
//! wave, and every Table 19-5 divisor is even, so each half is `N/2` cycles.
//!
//! The generator starts at the store's retire cycle. §19.2 does not give a
//! cycle offset for the first edge. The figures are a square wave and every
//! Table 19-5 divisor is even, so the model puts that edge half a period
//! later, puts the first MOSI bit on the pin at the write when `CPHA` is 0,
//! and sets `SPIF` at `start + 8N`. Figure 19-3 (`CPHA` = 0) has the first
//! bit on the wire before the sample edge, and the later bits change on the
//! trailing edge. Figure 19-4 (`CPHA` = 1) changes the first bit on the
//! leading edge and samples on the trailing edge. `SPIF` leaves SCK at `CPOL`.
//! §19.5.1 puts SCK at that idle level when `CPOL` or `SPE` is written,
//! at that store's retire cycle, not at the next `SPDR` write. The same
//! cycle releases SCK and MOSI when `SPE` clears and applies Table 19-1's
//! forced inputs.
//!
//! A slave `SPDR` write is not deferred to that retire. With `CPHA` 0 and
//! `SS` still low, Figure 19-3 already has the next bit on MISO, and the
//! write replaces bit 0 at the store's first cycle. `OUT` is one cycle and
//! `STS` is two. The bus delivers a host SCK edge at or before the retire
//! before `stamp()`, so an edge on the second cycle of an `STS`, or on the
//! retire cycle of either store, would sample the previous level if the bit
//! waited for the retire.
//!
//! Transmit is single-buffered (§19.2). The shifter ends a byte holding what
//! it shifted in, so a slave's next byte, with no new `SPDR` write, sends
//! the byte it just received. A write replaces that byte. Receive keeps the
//! last completed byte until `SPDR` is read; a second completion before that
//! read replaces it.
//! `SPIF` clears on the vector or by reading `SPSR` while it is set and then
//! accessing `SPDR`. `WCOL` clears by the same access sequence (§19.5.2).

use std::collections::VecDeque;

use crate::Level;
use crate::Port;
use crate::chip::SpiCfg;
use crate::mux::{DriveSource, PinMux};
use crate::periph::{Ctx, Gate, IoWrite, Touch};

use crate::regs::{SPCR as REG_SPCR, SPDR as REG_SPDR, SPSR as REG_SPSR};

const SPIE: u8 = 1 << 7;
const SPE: u8 = 1 << 6;
const DORD: u8 = 1 << 5;
const MSTR: u8 = 1 << 4;
const CPOL: u8 = 1 << 3;
const CPHA: u8 = 1 << 2;

const SPIF: u8 = 1 << 7;
const WCOL: u8 = 1 << 6;
const SPI2X: u8 = 1 << 0;

/// Table 19-5. SCK period in CPU cycles.
pub(crate) fn sck_period(spcr: u8, spsr: u8) -> u64 {
    match (spsr & SPI2X, spcr & 0x03) {
        (0, 0) => 4,
        (0, 1) => 16,
        (0, 2) => 64,
        (0, 3) => 128,
        (SPI2X, 0) => 2,
        (SPI2X, 1) => 8,
        (SPI2X, 2) => 32,
        _ => 64,
    }
}

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

#[derive(Clone, Copy, Debug)]
struct Wire {
    port: Port,
    bit: u8,
    id: u8,
}

impl Wire {
    fn new(port: Port, bit: u8) -> Self {
        Self {
            port,
            bit,
            id: port.pin_id(bit) as u8,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Spi {
    spcr: u8,
    /// `SPIF` and `WCOL`. `SPI2X` lives in [`Self::spi2x`].
    flags: u8,
    spi2x: u8,
    /// Bits of `SPIF`/`WCOL` that the next `SPDR` access clears.
    armed: u8,
    rx: u8,
    shift_out: u8,
    rx_acc: u8,
    miso_cur: u8,
    bit: u8,
    /// Master transfer in progress.
    shifting: bool,
    /// 1 = next event is the leading edge, 2 = trailing.
    phase: u8,
    next: u64,
    half: u64,
    /// Mode latched when the generator starts, or when slave `SS` falls.
    lat_cpha: bool,
    lat_cpol: bool,
    lat_lsb: bool,
    ss_at: u64,
    sck_at: u64,
    /// Slave has `SS` low.
    selected: bool,
    /// Slave bit clock has started and the byte is not finished.
    slave_busy: bool,
    /// `SPCR` pin effects wait until the store retires.
    defer_pins: bool,
    ss_edge: Option<Level>,
    sck_edge: Option<Level>,
    pending_tx: Option<u8>,
    mosi_q: VecDeque<(u8, u64)>,
    ss: Wire,
    mosi: Wire,
    miso: Wire,
    sck: Wire,
    vec: u8,
    clock_on: bool,
    prr_off: bool,
    frozen_at: u64,
    /// Last level driven on `MOSI` / `SCK` / slave `MISO`. `None` is no override.
    mosi_lv: Option<bool>,
    sck_lv: Option<bool>,
    miso_lv: Option<bool>,
}

impl Spi {
    pub(crate) fn new(cfg: &SpiCfg) -> Self {
        Self {
            spcr: 0,
            flags: 0,
            spi2x: 0,
            armed: 0,
            rx: 0,
            shift_out: 0,
            rx_acc: 0,
            miso_cur: 0xFF,
            bit: 0,
            shifting: false,
            phase: 0,
            next: u64::MAX,
            half: 2,
            lat_cpha: false,
            lat_cpol: false,
            lat_lsb: false,
            ss_at: 0,
            sck_at: 0,
            selected: false,
            slave_busy: false,
            defer_pins: false,
            ss_edge: None,
            sck_edge: None,
            pending_tx: None,
            mosi_q: VecDeque::new(),
            ss: Wire::new(cfg.ss.port, cfg.ss.bit),
            mosi: Wire::new(cfg.mosi.port, cfg.mosi.bit),
            miso: Wire::new(cfg.miso.port, cfg.miso.bit),
            sck: Wire::new(cfg.sck.port, cfg.sck.bit),
            vec: cfg.vec,
            clock_on: true,
            prr_off: false,
            frozen_at: 0,
            mosi_lv: None,
            sck_lv: None,
            miso_lv: None,
        }
    }

    pub(crate) fn pins(&self) -> (u8, u8, u8, u8) {
        (self.ss.id, self.mosi.id, self.miso.id, self.sck.id)
    }

    fn running(&self) -> bool {
        self.clock_on && !self.prr_off
    }

    fn enabled(&self) -> bool {
        self.spcr & SPE != 0
    }

    fn master(&self) -> bool {
        self.spcr & MSTR != 0
    }

    fn cpol_high(&self) -> bool {
        self.spcr & CPOL != 0
    }

    fn cpha(&self) -> bool {
        self.spcr & CPHA != 0
    }

    fn lsb(&self) -> bool {
        self.spcr & DORD != 0
    }

    fn ss_is_output(&self, mux: &PinMux) -> bool {
        let (ddr, _) = mux.ddr_port(self.ss.port);
        ddr & (1 << self.ss.bit) != 0
    }

    pub(crate) fn set_clock_gate(&mut self, io: Option<bool>, prr: Option<bool>, cycle: u64) {
        let delta = Gate {
            clock_on: &mut self.clock_on,
            prr_off: &mut self.prr_off,
            frozen_at: &mut self.frozen_at,
        }
        .set(io, prr, cycle);
        if let Some(delta) = delta
            && self.next != u64::MAX
        {
            self.next = self.next.wrapping_add(delta);
        }
    }

    pub(crate) fn take_mosi(&mut self) -> Option<(u8, u64)> {
        self.mosi_q.pop_front()
    }

    pub(crate) fn read(&mut self, reg: u8) -> (u8, bool) {
        if !self.running() {
            return (self.peek(reg), false);
        }
        match reg {
            REG_SPSR => {
                let value = self.peek(reg);
                if value & SPIF != 0 {
                    self.armed |= SPIF;
                }
                if value & WCOL != 0 {
                    self.armed |= WCOL;
                }
                (value, false)
            }
            REG_SPDR => {
                let value = self.rx;
                self.consume_spdr();
                (value, true)
            }
            _ => (self.peek(reg), false),
        }
    }

    /// `SPIF` and `WCOL` change at the byte's event. Not `SPSR` with a flag it would arm, or
    /// `SPDR`, whose read completes the clear.
    pub(crate) fn steady(&self, reg: u8) -> bool {
        match reg {
            REG_SPCR => true,
            REG_SPSR => !self.running() || self.flags & (SPIF | WCOL) & !self.armed == 0,
            _ => false,
        }
    }

    pub(crate) fn peek(&self, reg: u8) -> u8 {
        match reg {
            REG_SPCR => self.spcr,
            REG_SPSR => self.flags | self.spi2x,
            REG_SPDR => self.rx,
            _ => 0,
        }
    }

    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        _cycle: u64,
    ) -> Touch {
        if !self.running() {
            return Touch::default();
        }
        let mut touch = Touch {
            irq: true,
            schedule: true,
            ..Touch::default()
        };
        match reg {
            REG_SPCR => {
                self.spcr = value;
                if self.shifting && (!self.enabled() || !self.master()) {
                    self.shifting = false;
                    self.phase = 0;
                    self.next = u64::MAX;
                }
                // Pins follow the retire cycle, the same stamp `flush` gives
                // a GPIO store and `touch.stamp` gives an `SPDR` store.
                self.defer_pins = true;
                touch.stamp = true;
            }
            REG_SPSR => {
                self.spi2x = value & SPI2X;
            }
            REG_SPDR => {
                self.consume_spdr();
                if self.shifting || self.slave_busy {
                    self.flags |= WCOL;
                } else if self.enabled() && self.master() {
                    self.pending_tx = Some(value);
                    touch.stamp = true;
                } else {
                    self.shift_out = value;
                    // §19.2 / Figure 19-3. `CPHA` 0 already has the next bit
                    // on MISO, set up at the previous trailing edge. A write
                    // while `SS` stays low replaces that bit at this store,
                    // so the next leading edge samples it. `CPHA` 1 presents
                    // the bit on the leading edge and needs nothing here.
                    if self.enabled() && self.selected && !self.lat_cpha {
                        self.present_slave_bit(0, _cycle, mux, ctx);
                    }
                }
            }
            _ => {}
        }
        touch
    }

    pub(crate) fn poke(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        cycle: u64,
        slave_miso: &mut VecDeque<u8>,
    ) -> Touch {
        if !self.running() {
            return Touch::default();
        }
        let mut present = false;
        match reg {
            REG_SPCR => self.spcr = value,
            REG_SPSR => self.spi2x = value & SPI2X,
            REG_SPDR if self.shifting || self.slave_busy => {
                self.flags |= WCOL;
            }
            REG_SPDR => {
                self.shift_out = value;
                if self.enabled() && self.master() {
                    self.pending_tx = Some(value);
                } else if self.enabled() && self.selected && !self.lat_cpha {
                    present = true;
                }
            }
            _ => {}
        }
        let mut pins = VecDeque::new();
        let mut pending = Vec::new();
        let mut overflow = 0u64;
        let mut logs: Vec<IoWrite> = Vec::new();
        let mut drops = 0u64;
        let mut ctx = Ctx::new(
            &mut pins,
            &mut pending,
            &mut overflow,
            &mut logs,
            &mut drops,
        );
        if present {
            self.present_slave_bit(0, cycle, mux, &mut ctx);
        }
        self.consider_ss(mux, &mut ctx, cycle);
        self.refresh(mux, &mut ctx, cycle);
        if self.pending_tx.is_some() {
            self.stamp(cycle, mux, &mut ctx, slave_miso);
        }
        Touch {
            irq: true,
            schedule: true,
            ..Touch::default()
        }
    }

    /// The store has retired. §19.2 starts the generator here, and an
    /// `SPCR` write drives its pins here.
    pub(crate) fn stamp(
        &mut self,
        cycle: u64,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        slave_miso: &mut VecDeque<u8>,
    ) {
        if self.defer_pins {
            self.defer_pins = false;
            self.consider_ss(mux, ctx, cycle);
            self.refresh(mux, ctx, cycle);
        }
        let Some(byte) = self.pending_tx.take() else {
            return;
        };
        if !self.running() || !self.enabled() || !self.master() || self.shifting {
            self.shift_out = byte;
            return;
        }
        let miso = slave_miso.pop_front().unwrap_or(0xFF);
        self.begin_master(byte, miso, cycle, mux, ctx);
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        if self.flags & SPIF != 0 && self.spcr & SPIE != 0 {
            1u64 << self.vec
        } else {
            0
        }
    }

    pub(crate) fn ack(&mut self, vector: u8) {
        if vector == self.vec {
            self.flags &= !SPIF;
            self.armed &= !SPIF;
        }
    }

    pub(crate) fn next_event(&self) -> u64 {
        if self.running() { self.next } else { u64::MAX }
    }

    pub(crate) fn fire(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        if self.next != at || !self.shifting {
            return;
        }
        let leading = self.phase == 1;
        if leading {
            if self.lat_cpha {
                self.present_master_bit(self.bit, at, mux, ctx);
            } else if self.sample_pad(mux) {
                self.rx_acc = deposit(self.rx_acc, self.bit, self.lat_lsb, true);
            }
            self.drive_sck(!self.lat_cpol, at, mux, ctx);
            self.phase = 2;
            self.next = at + self.half;
        } else if self.lat_cpha {
            if self.sample_pad(mux) {
                self.rx_acc = deposit(self.rx_acc, self.bit, self.lat_lsb, true);
            }
            self.drive_sck(self.lat_cpol, at, mux, ctx);
            self.advance_master_bit(at);
        } else {
            self.bit += 1;
            if self.bit < 8 {
                self.present_master_bit(self.bit, at, mux, ctx);
            }
            self.drive_sck(self.lat_cpol, at, mux, ctx);
            if self.bit == 8 {
                self.finish_master(at);
            } else {
                self.phase = 1;
                self.next = at + self.half;
            }
        }
    }

    pub(crate) fn on_pad(&mut self, pin: u8, level: Level, at: u64) -> Touch {
        if !self.running() || !self.enabled() {
            return Touch::default();
        }
        if pin == self.ss.id {
            self.ss_edge = Some(level);
            self.ss_at = at;
            return Touch {
                schedule: true,
                irq: true,
                ..Touch::default()
            };
        }
        if pin == self.sck.id && !self.master() {
            self.sck_edge = Some(level);
            self.sck_at = at;
            return Touch {
                schedule: true,
                irq: true,
                ..Touch::default()
            };
        }
        Touch::default()
    }

    pub(crate) fn after_pad(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, _at: u64) -> Touch {
        let ss = self.ss_edge.take();
        let sck = self.sck_edge.take();
        if ss.is_none() && sck.is_none() {
            return Touch::default();
        }
        if let Some(level) = ss {
            self.on_ss(level, mux, ctx, self.ss_at);
        }
        if let Some(level) = sck {
            self.on_sck(level, mux, ctx, self.sck_at);
        }
        Touch {
            irq: true,
            schedule: true,
            ..Touch::default()
        }
    }

    fn consume_spdr(&mut self) {
        if self.armed & SPIF != 0 {
            self.flags &= !SPIF;
        }
        if self.armed & WCOL != 0 {
            self.flags &= !WCOL;
        }
        self.armed = 0;
    }

    fn begin_master(&mut self, byte: u8, miso: u8, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let period = sck_period(self.spcr, self.spi2x);
        self.half = period / 2;
        self.shift_out = byte;
        self.rx_acc = 0;
        self.bit = 0;
        self.miso_cur = miso;
        self.shifting = true;
        self.phase = 1;
        self.latch_mode();
        if !self.lat_cpha {
            self.present_master_bit(0, at, mux, ctx);
        }
        self.drive_sck(self.lat_cpol, at, mux, ctx);
        self.next = at + self.half;
    }

    fn latch_mode(&mut self) {
        self.lat_cpha = self.cpha();
        self.lat_cpol = self.cpol_high();
        self.lat_lsb = self.lsb();
    }

    fn advance_master_bit(&mut self, at: u64) {
        self.bit += 1;
        if self.bit == 8 {
            self.finish_master(at);
            return;
        }
        self.phase = 1;
        self.next = at + self.half;
    }

    fn present_master_bit(&mut self, index: u8, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let high = bit_high(self.shift_out, index, self.lat_lsb);
        self.mosi_lv = Some(high);
        self.drive_wire(self.mosi, high, at, mux, ctx);
        let miso_high = bit_high(self.miso_cur, index, self.lat_lsb);
        let level = if miso_high { Level::High } else { Level::Low };
        mux.drive(u32::from(self.miso.id), level, ctx, at);
    }

    fn finish_master(&mut self, at: u64) {
        self.shifting = false;
        self.phase = 0;
        self.next = u64::MAX;
        self.rx = self.rx_acc;
        self.flags |= SPIF;
        self.mosi_q.push_back((self.shift_out, at));
    }

    fn sample_pad(&self, mux: &PinMux) -> bool {
        mux.pad_high(self.miso.port, self.miso.bit)
    }

    fn consider_ss(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        if self.enabled()
            && self.master()
            && !self.ss_is_output(mux)
            && self.ss_level(mux) == Level::Low
        {
            self.become_slave(mux, ctx, at);
        }
    }

    fn ss_level(&self, mux: &PinMux) -> Level {
        mux.pad_level(self.ss.port, self.ss.bit)
    }

    fn on_ss(&mut self, level: Level, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        if self.master() {
            if level == Level::Low && !self.ss_is_output(mux) {
                self.become_slave(mux, ctx, at);
            }
            return;
        }
        if !self.enabled() {
            return;
        }
        if level == Level::Low {
            self.selected = true;
            self.slave_busy = false;
            self.bit = 0;
            self.rx_acc = 0;
            self.latch_mode();
            self.refresh(mux, ctx, at);
            if !self.lat_cpha {
                self.present_slave_bit(0, at, mux, ctx);
            }
        } else if level == Level::High {
            self.selected = false;
            self.slave_busy = false;
            self.bit = 0;
            self.rx_acc = 0;
            self.miso_lv = None;
            self.refresh(mux, ctx, at);
        }
    }

    fn become_slave(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        self.spcr &= !MSTR;
        self.flags |= SPIF;
        self.shifting = false;
        self.phase = 0;
        self.next = u64::MAX;
        self.selected = true;
        self.slave_busy = false;
        self.bit = 0;
        self.rx_acc = 0;
        self.mosi_lv = None;
        self.sck_lv = None;
        self.latch_mode();
        self.refresh(mux, ctx, at);
        if !self.lat_cpha {
            self.present_slave_bit(0, at, mux, ctx);
        }
    }

    fn on_sck(&mut self, level: Level, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        if self.master() || !self.selected || !self.enabled() {
            return;
        }
        let high = level == Level::High;
        let leading = high != self.lat_cpol;
        let sample = self.lat_cpha != leading;
        self.slave_busy = true;
        if sample {
            let mosi_high = mux.pad_high(self.mosi.port, self.mosi.bit);
            self.rx_acc = deposit(self.rx_acc, self.bit, self.lat_lsb, mosi_high);
            if !leading {
                self.finish_slave_bit(at, mux, ctx);
            }
        } else if leading {
            self.present_slave_bit(self.bit, at, mux, ctx);
        } else {
            self.finish_slave_bit(at, mux, ctx);
        }
    }

    fn finish_slave_bit(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        self.bit += 1;
        if self.bit == 8 {
            // §19.2: transmit is single-buffered. The shifter ends the byte
            // holding what it shifted in, and that is what the next byte
            // sends unless `SPDR` is written again.
            self.rx = self.rx_acc;
            self.shift_out = self.rx_acc;
            self.flags |= SPIF;
            self.bit = 0;
            self.rx_acc = 0;
            self.slave_busy = false;
            if self.selected && !self.lat_cpha {
                self.present_slave_bit(0, at, mux, ctx);
            }
        } else if !self.lat_cpha {
            self.present_slave_bit(self.bit, at, mux, ctx);
        }
    }

    fn present_slave_bit(&mut self, index: u8, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let high = bit_high(self.shift_out, index, self.lat_lsb);
        self.miso_lv = Some(high);
        self.drive_wire(self.miso, high, at, mux, ctx);
    }

    fn drive_sck(&mut self, high: bool, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        self.sck_lv = Some(high);
        self.drive_wire(self.sck, high, at, mux, ctx);
    }

    fn drive_wire(&mut self, wire: Wire, high: bool, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let level = if high { Level::High } else { Level::Low };
        mux.set_override(
            DriveSource::Spi { pin: wire.id },
            wire.port,
            wire.bit,
            Some(level),
            ctx,
            Some(at),
            at,
        );
    }

    fn refresh(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        let spe = self.enabled();
        let master = self.master();
        let miso_force = spe && (master || !self.selected);
        let slave_in = spe && !master;
        self.force(mux, ctx, self.miso, miso_force, at);
        self.force(mux, ctx, self.mosi, slave_in, at);
        self.force(mux, ctx, self.sck, slave_in, at);
        self.force(mux, ctx, self.ss, slave_in, at);
        if spe && master {
            // §19.5.1 CPOL. "When this bit is written to one, SCK is high when
            // idle." Enabling the master, or changing CPOL while no byte is
            // shifting, drives that level at the store's retire cycle.
            if !self.shifting {
                self.sck_lv = Some(self.cpol_high());
            }
            if let Some(high) = self.sck_lv {
                self.drive_wire(self.sck, high, at, mux, ctx);
            }
            if let Some(high) = self.mosi_lv {
                self.drive_wire(self.mosi, high, at, mux, ctx);
            }
        } else {
            self.clear_override(mux, ctx, self.sck, at);
            self.clear_override(mux, ctx, self.mosi, at);
            self.sck_lv = None;
            self.mosi_lv = None;
        }
        if spe && !master && self.selected {
            if let Some(high) = self.miso_lv {
                self.drive_wire(self.miso, high, at, mux, ctx);
            }
        } else if !master {
            self.clear_override(mux, ctx, self.miso, at);
            self.miso_lv = None;
        }
    }

    fn force(&self, mux: &mut PinMux, ctx: &mut Ctx<'_>, wire: Wire, force: bool, at: u64) {
        mux.set_force_input(wire.port, wire.bit, force, ctx, Some(at), at);
    }

    fn clear_override(&self, mux: &mut PinMux, ctx: &mut Ctx<'_>, wire: Wire, at: u64) {
        mux.set_override(
            DriveSource::Spi { pin: wire.id },
            wire.port,
            wire.bit,
            None,
            ctx,
            Some(at),
            at,
        );
    }
}
