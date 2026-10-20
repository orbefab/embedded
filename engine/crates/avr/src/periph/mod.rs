//! One [`Device`] per peripheral instance. The bus matches the enum, so a new
//! peripheral is a variant, its module, and a chip-table row.
//!
//! `read`/`write` take the register id the owner table stored at construction.
//! `next_event` is the cycle of the next thing that device must do, or
//! `u64::MAX`. `advance` runs events up to a cycle. USART schedules bit
//! edges. The ADC schedules a conversion completion. The comparator schedules
//! the synchronized `ACO` update.
//!
//! A rising interrupt flag is a [`Notice`] the bus hands to the ADC. That is
//! the Table 24-6 auto-trigger. It is not a pad edge.

pub(crate) mod acmp;
pub(crate) mod adc;
pub(crate) mod eeprom;
mod extint;
mod gpio;
mod prescaler;
pub(crate) mod spi;
pub(crate) mod system;
pub(crate) mod timer;
pub(crate) mod twi;
pub(crate) mod usart;

#[cfg(test)]
pub(crate) mod probe;
#[cfg(test)]
mod step_check;

pub(crate) use acmp::{Acmp, AcmpCfg};
pub(crate) use adc::{Adc, AdcCfg};
pub(crate) use eeprom::{Eeprom, EepromCfg};
pub(crate) use extint::{
    ExtInt, ExtIntCfg, IntLine, PcGroup, REG_EICRA, REG_EICRB, REG_EIFR, REG_EIMSK, REG_MCUCR,
    REG_PCICR, REG_PCIFR, REG_PCMSK0, REG_PCMSK1, REG_PCMSK2,
};
pub(crate) use gpio::GpioPort;
pub(crate) use prescaler::{ClockView, Prescaler};
pub(crate) use spi::Spi;
pub(crate) use system::{System, SystemCfg};
pub(crate) use timer::Timer;
pub(crate) use twi::Twi;
pub(crate) use usart::Usart;

use std::collections::VecDeque;

use crate::chip::DeviceSpec;
use crate::mux::PinMux;
use crate::regs::OwnedReg;
use crate::{Level, PIN_QUEUE_CAP, PinOrigin, PinReport};

/// A store into I/O space (data addresses 0x0020 through 0x00FF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoWrite {
    pub addr: u16,
    pub value: u8,
}

/// How many diagnostic writes are kept. Further writes increment the dropped count.
pub const IO_LOG_CAP: usize = 64;

/// GPIO edges still inside the current instruction, before they are stamped.
pub(crate) const PENDING_PIN_CAP: usize = 8;

/// Clock and `PRR` freeze shared by USART, SPI, TWI, and the ADC.
///
/// Borrowed so the flags stay in the device's byte run. A stored struct pads
/// to 16 bytes and pushes `Device` past USART.
pub(crate) struct Gate<'a> {
    clock_on: &'a mut bool,
    prr_off: &'a mut bool,
    frozen_at: &'a mut u64,
}

impl Gate<'_> {
    /// `Some(delta)` when the module resumes after a freeze.
    pub(crate) fn set(self, io: Option<bool>, prr: Option<bool>, cycle: u64) -> Option<u64> {
        let was = *self.clock_on && !*self.prr_off;
        if let Some(on) = io {
            *self.clock_on = on;
        }
        if let Some(stopped) = prr {
            *self.prr_off = stopped;
        }
        let now_on = *self.clock_on && !*self.prr_off;
        if was && !now_on {
            *self.frozen_at = cycle;
            None
        } else if !was && now_on {
            let delta = cycle.saturating_sub(*self.frozen_at);
            (delta > 0).then_some(delta)
        } else {
            None
        }
    }
}

/// A stopped deadline stays stopped. A live one moves by `delta`.
pub(crate) fn shift_deadline(at: u64, delta: u64) -> u64 {
    if at == u64::MAX {
        at
    } else {
        at.wrapping_add(delta)
    }
}

/// When a store happens: the cycle it starts on and the edge that retires it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StoreAt {
    pub cycle: u64,
    pub end: u64,
}

/// What a register write changed, so the bus recomputes only then.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Touch {
    pub schedule: bool,
    pub irq: bool,
    pub hold: bool,
    /// A USART byte is waiting for this instruction's cycle.
    pub stamp: bool,
    /// CPU clocks to stall before the next instruction (§8.4.1).
    pub halt: u8,
    /// A 4-cycle enable window was armed. The bus commits it at retire.
    pub windows: bool,
    /// `CLKPR` changed the division.
    pub div: bool,
    /// `PRR` changed.
    pub prr: bool,
}

/// Device-to-device note. The bus drains these after a write or a fired event.
///
/// `rose`/`fell` bits are Table 24-6 trigger codes: bit 1 `ACI`, 2 `INTF0`,
/// 3 `OCF0A`, 4 `TOV0`, 5 `OCF1B`, 6 `TOV1`, 7 `ICF1`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Notice {
    pub rose: u8,
    pub fell: u8,
    /// `ADMUX` / `ADEN` / `ACME` changed. The comparator re-reads its inputs.
    pub mux: bool,
    pub acme: bool,
    pub aden: bool,
    pub channel: u8,
    /// `ADATE` or `ADTS` changed. `synthesize` is set only for an `ADTS` change.
    pub retarget: bool,
    /// §24.9.4. An `ADTS` change from a clear source onto a set one is an edge.
    pub synthesize: bool,
    pub old_adts: u8,
    /// Comparator output changed after its synchronizer.
    pub aco: bool,
    pub aco_high: bool,
    /// `ACIC` was written.
    pub acic: bool,
    pub acic_on: bool,
    /// A comparator control bit changed. Re-compare the inputs.
    pub refresh: bool,
}

impl Notice {
    pub(crate) fn is_empty(self) -> bool {
        self.rose == 0
            && self.fell == 0
            && !self.mux
            && !self.retarget
            && !self.aco
            && !self.acic
            && !self.refresh
    }

    pub(crate) fn absorb(&mut self, other: Self) {
        self.rose |= other.rose;
        self.fell |= other.fell;
        if other.mux {
            self.mux = true;
            self.acme = other.acme;
            self.aden = other.aden;
            self.channel = other.channel;
        }
        if other.retarget {
            self.retarget = true;
            self.synthesize |= other.synthesize;
            self.old_adts = other.old_adts;
        }
        if other.aco {
            self.aco = true;
            self.aco_high = other.aco_high;
        }
        if other.acic {
            self.acic = true;
            self.acic_on = other.acic_on;
        }
        self.refresh |= other.refresh;
    }
}

/// Queues a device can push while it handles a read or a write.
pub(crate) struct Ctx<'a> {
    pins: &'a mut VecDeque<PinReport>,
    pending: &'a mut Vec<PinReport>,
    overflow: &'a mut u64,
    io_writes: &'a mut Vec<IoWrite>,
    io_drops: &'a mut u64,
}

impl Ctx<'_> {
    pub(crate) fn new<'a>(
        pins: &'a mut VecDeque<PinReport>,
        pending: &'a mut Vec<PinReport>,
        overflow: &'a mut u64,
        io_writes: &'a mut Vec<IoWrite>,
        io_drops: &'a mut u64,
    ) -> Ctx<'a> {
        Ctx {
            pins,
            pending,
            overflow,
            io_writes,
            io_drops,
        }
    }

    pub(crate) fn log(&mut self, addr: u16, value: u8) {
        if self.io_writes.len() >= IO_LOG_CAP {
            *self.io_drops = self.io_drops.wrapping_add(1);
            return;
        }
        self.io_writes.push(IoWrite { addr, value });
    }

    /// `cycle` is `None` for a store still inside the current instruction.
    pub(crate) fn emit_pin(
        &mut self,
        pin: u32,
        level: Level,
        cycle: Option<u64>,
        origin: PinOrigin,
    ) {
        let Some(cycle) = cycle else {
            if self.pending.len() < PENDING_PIN_CAP {
                self.pending.push(PinReport {
                    pin,
                    level,
                    cycle: 0,
                    origin,
                });
            } else {
                *self.overflow = self.overflow.wrapping_add(1);
            }
            return;
        };
        push_pin(
            self.pins,
            self.overflow,
            PinReport {
                pin,
                level,
                cycle,
                origin,
            },
        );
    }
}

/// Keep the newest reports. A host that falls behind still sees the current
/// level; the edges it missed are the oldest ones, counted in the overflow.
pub(crate) fn push_pin(pins: &mut VecDeque<PinReport>, overflow: &mut u64, report: PinReport) {
    if pins.len() >= PIN_QUEUE_CAP {
        pins.pop_front();
        *overflow = overflow.wrapping_add(1);
    }
    pins.push_back(report);
}

/// Static dispatch over the peripherals on this part.
///
/// `ExtInt`'s edge queue is allocated once in [`ExtInt::new`] so this enum
/// stays the size of the timer and the USART. A tick does not follow that
/// pointer unless an edge is actually waiting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub(crate) enum Device {
    Port(GpioPort),
    ExtInt(ExtInt),
    Timer(Timer),
    Prescaler(Prescaler),
    Usart(Usart),
    Spi(Spi),
    Twi(Twi),
    Adc(Adc),
    Acmp(Acmp),
    Eeprom(Eeprom),
    System(System),
}

impl Device {
    pub(crate) fn from_spec(spec: &DeviceSpec, timer_index: u8) -> Self {
        match spec {
            DeviceSpec::Port(cfg) => Device::Port(GpioPort::new(cfg.port)),
            DeviceSpec::ExtInt(cfg) => Device::ExtInt(ExtInt::new(*cfg)),
            DeviceSpec::Timer(cfg) => Device::Timer(Timer::new(*cfg, timer_index)),
            DeviceSpec::Prescaler(_) => Device::Prescaler(Prescaler::new()),
            DeviceSpec::Usart(cfg) => Device::Usart(Usart::new(cfg)),
            DeviceSpec::Spi(cfg) => Device::Spi(Spi::new(cfg)),
            DeviceSpec::Twi(cfg) => Device::Twi(Twi::new(cfg)),
            DeviceSpec::Adc(cfg) => Device::Adc(Adc::new(cfg)),
            DeviceSpec::Acmp(cfg) => Device::Acmp(Acmp::new(cfg)),
            DeviceSpec::Eeprom(cfg) => Device::Eeprom(Eeprom::new(*cfg)),
            DeviceSpec::System(cfg) => Device::System(System::new(*cfg)),
        }
    }

    /// The boolean is set when the read changed an interrupt flag.
    pub(crate) fn read(
        &mut self,
        reg: u8,
        mux: &PinMux,
        cycle: u64,
        clock: &ClockView,
    ) -> (u8, bool) {
        match self {
            Device::Port(port) => (port.read(reg, mux, cycle), false),
            Device::ExtInt(ext) => (ext.read(reg), false),
            Device::Timer(timer) => (timer.read(reg, cycle, clock), false),
            Device::Prescaler(pre) => (pre.peek(), false),
            Device::Usart(usart) => usart.read(reg),
            Device::Spi(spi) => spi.read(reg),
            Device::Twi(twi) => (twi.peek(reg), false),
            Device::Adc(adc) => adc.read(reg),
            Device::Acmp(acmp) => (acmp.read(reg), false),
            Device::Eeprom(eeprom) => eeprom.read(reg),
            Device::System(sys) => sys.read(reg),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        ctx: &mut Ctx<'_>,
        mux: &mut PinMux,
        at: StoreAt,
        clock: &ClockView,
        addr: u16,
    ) -> Touch {
        let cycle = at.cycle;
        match self {
            Device::Port(port) => {
                port.write(reg, value, mux, ctx, cycle);
                Touch::default()
            }
            Device::ExtInt(ext) => ext.write(reg, value, mux, ctx, cycle),
            Device::Timer(timer) => timer.write(reg, value, ctx, mux, at, clock, addr),
            Device::Usart(usart) => usart.write(reg, value, mux, ctx, cycle),
            Device::Spi(spi) => spi.write(reg, value, mux, ctx, cycle),
            Device::Twi(twi) => twi.write(reg, value, mux, ctx, cycle),
            Device::Adc(adc) => adc.write(reg, value, mux, ctx, cycle),
            Device::Acmp(acmp) => acmp.write(reg, value, mux, ctx, cycle),
            Device::Eeprom(eeprom) => eeprom.write(reg, value, cycle),
            Device::System(sys) => sys.write(reg, value, cycle),
            Device::Prescaler(pre) => {
                pre.defer(value, at.end);
                Touch {
                    schedule: true,
                    ..Touch::default()
                }
            }
        }
    }

    pub(crate) fn peek(&self, reg: u8, mux: &PinMux, cycle: u64, clock: &ClockView) -> u8 {
        match self {
            Device::Port(port) => port.peek(reg, mux, cycle),
            Device::ExtInt(ext) => ext.peek(reg),
            Device::Timer(timer) => timer.peek(reg, cycle, clock),
            Device::Prescaler(pre) => pre.peek(),
            Device::Usart(usart) => usart.peek(reg),
            Device::Spi(spi) => spi.peek(reg),
            Device::Twi(twi) => twi.peek(reg),
            Device::Adc(adc) => adc.peek(reg),
            Device::Acmp(acmp) => acmp.peek(reg),
            Device::Eeprom(eeprom) => eeprom.peek(reg),
            Device::System(sys) => sys.peek(reg),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn poke(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        cycle: u64,
        clock: &ClockView,
        ctx: &mut Ctx<'_>,
        host: &mut crate::bus::hostside::HostSide,
    ) -> Touch {
        match self {
            Device::Port(port) => {
                port.poke(reg, value, mux, ctx, cycle);
                Touch::default()
            }
            Device::ExtInt(ext) => ext.poke(reg, value, mux, ctx, cycle),
            Device::Timer(timer) => timer.poke(reg, value, cycle, clock),
            Device::Usart(usart) => {
                usart.poke(reg, value, cycle);
                Touch {
                    irq: true,
                    ..Touch::default()
                }
            }
            // The scratch stays here. The real context would report SS and MOSI,
            // and no caller expects a poke to do that.
            Device::Spi(spi) => spi.poke(reg, value, mux, cycle, host.slave_miso()),
            Device::Twi(twi) => twi.poke(reg, value, mux, ctx, cycle, host),
            Device::Adc(adc) => adc.write(reg, value, mux, ctx, cycle),
            Device::Acmp(acmp) => acmp.write(reg, value, mux, ctx, cycle),
            Device::Eeprom(eeprom) => eeprom.poke(reg, value, cycle),
            Device::System(sys) => sys.poke(reg, value, cycle),
            // The prescaler store is applied before this match.
            _ => Touch {
                schedule: true,
                ..Touch::default()
            },
        }
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        match self {
            Device::Timer(timer) => timer.irq_bits(),
            Device::ExtInt(ext) => ext.irq_bits(),
            Device::Usart(usart) => usart.irq_bits(),
            Device::Spi(spi) => spi.irq_bits(),
            Device::Twi(twi) => twi.irq_bits(),
            Device::Adc(adc) => adc.irq_bits(),
            Device::Acmp(acmp) => acmp.irq_bits(),
            Device::Eeprom(eeprom) => eeprom.irq_bits(),
            Device::System(sys) => sys.irq_bits(),
            _ => 0,
        }
    }

    pub(crate) fn ack(&mut self, vector: u8) {
        match self {
            Device::Timer(timer) => timer.ack(vector),
            Device::ExtInt(ext) => ext.ack(vector),
            Device::Usart(usart) => usart.ack(vector),
            Device::Spi(spi) => spi.ack(vector),
            Device::Twi(twi) => twi.ack(vector),
            Device::Adc(adc) => adc.ack(vector),
            Device::Acmp(acmp) => acmp.ack(vector),
            Device::System(sys) => sys.ack(vector),
            _ => {}
        }
    }

    pub(crate) fn next_event(&self) -> u64 {
        match self {
            Device::Timer(timer) => timer.next_event(),
            Device::ExtInt(ext) => ext.next_event(),
            Device::Usart(usart) => usart.next_event(),
            Device::Spi(spi) => spi.next_event(),
            Device::Twi(twi) => twi.next_event(),
            Device::Adc(adc) => adc.next_event(),
            Device::Acmp(acmp) => acmp.next_event(),
            Device::Eeprom(eeprom) => eeprom.next_event(),
            Device::System(sys) => sys.next_event(),
            Device::Prescaler(pre) => pre.next_event(),
            _ => u64::MAX,
        }
    }

    pub(crate) fn materialize(&mut self, cycle: u64, clock: &ClockView) {
        if let Device::Timer(timer) = self {
            timer.materialize(cycle, clock);
        }
    }

    pub(crate) fn release_hold(&mut self, end: u64, clock: &ClockView) {
        if let Device::Timer(timer) = self {
            timer.release_hold(end, clock);
        }
    }

    /// Run the event at `at`. Notices are drained by the bus afterwards.
    pub(crate) fn fire(&mut self, at: u64, clock: &ClockView, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        match self {
            Device::Timer(timer) => timer.fire(at, clock, mux, ctx),
            Device::ExtInt(ext) => ext.fire(at),
            Device::Usart(usart) => usart.fire(at, mux, ctx),
            Device::Spi(spi) => spi.fire(at, mux, ctx),
            Device::Twi(twi) => twi.fire(at),
            Device::Adc(adc) => adc.fire(at),
            Device::Acmp(acmp) => acmp.fire(at),
            Device::Eeprom(eeprom) => eeprom.fire(at),
            Device::System(sys) => sys.fire(at),
            _ => {}
        }
    }

    pub(crate) fn reschedule(&mut self, clock: &ClockView) {
        match self {
            Device::Timer(timer) => timer.reschedule(clock),
            Device::ExtInt(ext) => ext.reschedule(),
            _ => {}
        }
    }

    /// A pad edge at `at`, before the synchronizer. The device applies its own delay.
    pub(crate) fn on_pad(&mut self, pin: u8, level: Level, at: u64) -> Touch {
        match self {
            Device::ExtInt(ext) => ext.on_pad(pin, level, at),
            Device::Timer(timer) => timer.on_pad(pin, level, at),
            Device::Usart(usart) => usart.on_pad(pin, level, at),
            Device::Spi(spi) => spi.on_pad(pin, level, at),
            _ => Touch::default(),
        }
    }

    /// Apply a device's pin drive after every `on_pad` in this batch.
    pub(crate) fn after_pad(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) -> Touch {
        match self {
            Device::Spi(spi) => spi.after_pad(mux, ctx, at),
            _ => Touch::default(),
        }
    }

    /// One clock gate. `io` is `clkI/O` running, `adc` is `clkADC` running, and
    /// `prr` is the module's power-reduction bit (true means stopped). `None`
    /// leaves that input unchanged. The ADC clock is `adc` when it is `Some`,
    /// otherwise `io`.
    pub(crate) fn gate(
        &mut self,
        io: Option<bool>,
        adc: Option<bool>,
        prr: Option<bool>,
        cycle: u64,
        clock: &crate::periph::prescaler::ClockView,
    ) {
        match self {
            Device::ExtInt(ext) => {
                if let Some(running) = io {
                    ext.set_io_clock_running(running);
                }
            }
            Device::Timer(timer) => timer.apply_gate(io, prr, cycle, clock),
            Device::Usart(usart) => usart.set_clock_gate(io, prr, cycle),
            Device::Spi(spi) => spi.set_clock_gate(io, prr, cycle),
            Device::Twi(twi) => twi.set_clock_gate(io, prr, cycle),
            Device::Adc(adc_dev) => adc_dev.set_clock_gate(adc.or(io), prr, cycle),
            Device::Prescaler(pre) => {
                if let Some(running) = io {
                    pre.set_io_clock(running, cycle);
                }
            }
            _ => {}
        }
    }

    /// §11.1. Free-running counters read phase 0 at internal-reset release.
    pub(crate) fn release_epoch(&mut self, cycle: u64) {
        match self {
            Device::Prescaler(pre) => pre.release_epoch(cycle),
            Device::Usart(usart) => usart.release_epoch(cycle),
            Device::Adc(adc) => adc.release_epoch(cycle),
            _ => {}
        }
    }

    pub(crate) fn commit_windows(&mut self, retire: u64) {
        match self {
            Device::Eeprom(eeprom) => eeprom.commit_windows(retire),
            Device::System(sys) => sys.commit_windows(retire),
            Device::ExtInt(ext) => ext.commit_windows(retire),
            _ => {}
        }
    }

    pub(crate) fn take_notice(&mut self) -> Notice {
        match self {
            Device::Timer(timer) => timer.take_notice(),
            Device::ExtInt(ext) => ext.take_notice(),
            Device::Adc(adc) => adc.take_notice(),
            Device::Acmp(acmp) => acmp.take_notice(),
            _ => Notice::default(),
        }
    }

    pub(crate) fn stamp(
        &mut self,
        cycle: u64,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        host: &mut crate::bus::hostside::HostSide,
    ) {
        match self {
            Device::Usart(usart) => usart.stamp(cycle, mux, ctx),
            Device::Spi(spi) => spi.stamp(cycle, mux, ctx, host.slave_miso()),
            Device::Twi(twi) => twi.stamp(cycle, mux, ctx, host),
            _ => {}
        }
    }
}

/// Register rows of one chip-table entry. The bus copies `strobe` and `unmodeled`.
pub(crate) fn rows_of(spec: &DeviceSpec, out: &mut Vec<OwnedReg>) {
    out.clear();
    match spec {
        DeviceSpec::Port(cfg) => out.extend_from_slice(&cfg.regs),
        DeviceSpec::ExtInt(cfg) => {
            out.extend_from_slice(&cfg.regs.rows[..usize::from(cfg.regs.len)]);
        }
        DeviceSpec::Timer(cfg) => {
            out.extend_from_slice(&cfg.regs.rows[..usize::from(cfg.regs.len)]);
        }
        DeviceSpec::Prescaler(cfg) => out.push(cfg.reg),
        DeviceSpec::Usart(cfg) => out.extend_from_slice(&cfg.regs),
        DeviceSpec::Spi(cfg) => out.extend_from_slice(&cfg.regs),
        DeviceSpec::Twi(cfg) => out.extend_from_slice(&cfg.regs),
        DeviceSpec::Adc(cfg) => {
            out.extend_from_slice(&cfg.regs.rows[..usize::from(cfg.regs.len)]);
        }
        DeviceSpec::Eeprom(cfg) => {
            out.extend_from_slice(&cfg.regs.rows[..usize::from(cfg.regs.len)]);
        }
        DeviceSpec::System(cfg) => {
            out.extend_from_slice(&cfg.regs.rows[..usize::from(cfg.regs.len)]);
        }
        DeviceSpec::Acmp(cfg) => {
            out.extend_from_slice(&cfg.regs.rows[..usize::from(cfg.regs.len)]);
        }
    }
}

#[cfg(test)]
mod size {
    use super::*;

    /// USART is the largest variant at 288 bytes. ExtInt's edge queue is boxed.
    #[test]
    fn device_stays_within_bound() {
        let size = std::mem::size_of::<Device>();
        assert!(size <= 288, "Device is {size} bytes");
    }
}
