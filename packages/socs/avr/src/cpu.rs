//! Public facade. Constructors, the instruction loop, and the host queues.
//!
//! After each instruction the bus advances devices only when an event is due,
//! then the interrupt controller serves at most one request if the pending
//! mask is set. While the core is sleeping, [`Cpu::run_until`] jumps to the
//! next event. A budget is a cycle: there is no instruction-count run.

use std::fmt;

use crate::bus::Bus;
use crate::chip::{FLASH_WORDS, IRQ_CYCLES, RAMEND, USART0, VECTORS};
use crate::core::{Activity, Core};
use crate::fuses::FuseBytes;
use crate::image::Image;
use crate::irq;
use crate::periph::system::{BORF, EXTRF, PORF, WDRF};
use crate::periph::usart::Sent;
use crate::{IoWrite, Level, PinLevels, PinReport, SpmNote};

mod seam;

/// Parity bit on a host serial frame (DS40002061B §20.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

/// Character size, parity, and stop bits for the host's serial transmitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerialFrame {
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
}

impl SerialFrame {
    /// 8 data bits, no parity, one stop bit. `UCSRnC` resets to this size.
    pub const EIGHT_N_1: Self = Self {
        data_bits: 8,
        parity: Parity::None,
        stop_bits: 1,
    };
}

impl Default for SerialFrame {
    fn default() -> Self {
        Self::EIGHT_N_1
    }
}

/// Oscillator assumed until [`Cpu::set_clock_hz`]. Baud in hertz uses it.
pub const DEFAULT_CLOCK_HZ: u32 = 16_000_000;

/// Host byte stream. A new host stream is a variant here, not a method on [`Cpu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// USART0.
    ///
    /// A taken byte is stamped with the cycle its last stop bit ends
    /// (DS40002061B §20.6). [`Cpu::supply`] drives `RXD` with that frame.
    /// The bit time follows the USART's `UBRR`, `U2X`, and frame unless
    /// [`Cpu::set_host_serial`] set another. A byte supplied while `RXEN`
    /// is clear is still driven, and the receiver misses it.
    Usart0,
    /// MOSI from a master chip.
    ///
    /// [`Cpu::supply`] queues the bytes the host slave returns on `MISO`, one
    /// per frame, and `0xFF` when the queue is empty. [`Cpu::take`] yields
    /// each MOSI byte at the cycle `SPIF` is set. That slave shifts on every
    /// master frame and does not look at `SS`.
    Spi,
    /// MISO sampled by a host master.
    ///
    /// [`Cpu::queue_spi_master`] arms `SS`, `SCK`, and `MOSI` and returns.
    /// [`Cpu::take`] yields each MISO byte at that frame's last SCK edge.
    /// [`Cpu::supply`] does not accept this channel. MISO is taken, not supplied.
    SpiHostMaster,
}

/// A channel [`Cpu::supply`] accepts.
///
/// [`Channel::SpiHostMaster`] is absent: the host master has no inbound bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyChannel {
    Usart0,
    Spi,
}

/// Host-master clock for [`Cpu::queue_spi_master`].
///
/// `mode` is `CPOL` in bit 1 and `CPHA` in bit 0, the same packing as `SPCR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiClock {
    /// SCK period in CPU cycles. Each half is `period / 2`, and at least one.
    pub period: u32,
    /// `CPOL << 1 | CPHA`.
    pub mode: u8,
    pub lsb_first: bool,
}

/// The image is larger than this part's flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadError {
    pub bytes: usize,
    pub flash_bytes: usize,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "image is {} bytes and this part's flash is {} bytes",
            self.bytes, self.flash_bytes
        )
    }
}

impl std::error::Error for LoadError {}

/// One programmed ATmega328P.
#[derive(Debug)]
pub struct Cpu {
    core: Core,
    bus: Bus,
    clock_hz: u32,
    /// Host baud override for USART0. `None` follows the USART.
    host_serial: Option<(u32, SerialFrame)>,
    /// Supply in millivolts. Default 5000. §11.5.
    vcc_mv: i32,
    /// Host is holding the RESET pin low.
    reset_held: bool,
    /// VCC is below `VBOT-`.
    bod_held: bool,
    /// The host's own reading of `VCC` against the brown-out window, from
    /// [`Self::set_brown_out`]: `true` below it. `None` reads `vcc_mv`.
    bod_low: Option<bool>,
}

clone_fields!(Cpu {
    core,
    bus,
    clock_hz,
    host_serial,
    vcc_mv,
    reset_held,
    bod_held,
    bod_low
});

/// Why [`Cpu::reset_as`] ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetKind {
    /// Supply rose through the power-on threshold. Clears the other `MCUSR` flags.
    PowerOn,
    /// The host pulled `RESET`.
    External,
    /// The watchdog reached its period in reset mode.
    Watchdog,
    /// `VCC` fell through `VBOT-`.
    BrownOut,
}

enum ResetWhen {
    /// Host `reset()`: the timeline restarts at the first instruction.
    Restart,
    /// An in-run reset: `cycles()` continues through the start-up time-out.
    Delay,
    /// Stay stopped until the pin rises or `VCC` recovers.
    Hold,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            core: Core::new(RAMEND, FLASH_WORDS - 1),
            bus: Bus::new(),
            clock_hz: DEFAULT_CLOCK_HZ,
            host_serial: None,
            vcc_mv: 5_000,
            reset_held: false,
            bod_held: false,
            bod_low: None,
        }
    }

    /// Power-on reset that restarts the host timeline.
    ///
    /// Flash and EEPROM are kept. SRAM is cleared: the datasheet leaves the
    /// power-on contents undefined, and this model fills it with zero. Cycle 0
    /// is the first instruction. The fuse start-up time-out has already
    /// elapsed, so existing programs are not shifted by it. [`Self::reset_as`]
    /// is the in-run reset, and that one does insert the time-out.
    /// `SP` is RAMEND. The host clock, fuses, `VCC`, and host serial setting
    /// are kept. A host pin drive is not.
    pub(crate) fn reset(&mut self) {
        self.perform_reset(ResetKind::PowerOn, ResetWhen::Restart);
    }

    /// Reset while the cycle counter keeps running. The start-up time-out from
    /// the fuses elapses before the first instruction (§11.1). SRAM is kept
    /// except on [`ResetKind::PowerOn`].
    pub(crate) fn reset_as(&mut self, kind: ResetKind) {
        self.perform_reset(kind, ResetWhen::Delay);
    }

    /// Hold the chip in reset. Ports return to their reset state immediately
    /// (§11.1). [`Self::release_reset`] starts the time-out.
    pub fn pull_reset(&mut self) {
        if self.reset_held {
            return;
        }
        self.reset_held = true;
        self.perform_reset(ResetKind::External, ResetWhen::Hold);
    }

    /// Release `RESET`. If `VCC` is also above the brown-out threshold, the
    /// start-up time-out runs and then the reset vector is fetched.
    pub fn release_reset(&mut self) {
        if !self.reset_held {
            return;
        }
        self.reset_held = false;
        if !self.bod_held {
            self.begin_reset_delay();
        }
    }

    /// Pull and release `RESET` at the current cycle. The pulse is long enough
    /// to reset (Table 29-11 `tRST` is 2.5 µs; this call does not model a
    /// shorter spike).
    pub(crate) fn pulse_reset(&mut self) {
        self.reset_held = false;
        self.perform_reset(ResetKind::External, ResetWhen::Delay);
    }

    /// Replace the fuse bytes. `CLKPR`'s initial value is applied immediately
    /// when no instruction has run yet, and on the next reset after that.
    /// A running program's current `CLKPS` is left as it is unless `cycles` is 0.
    pub fn set_fuses(&mut self, low: u8, high: u8, extended: u8) {
        self.bus.set_fuses(FuseBytes {
            low,
            high,
            extended,
        });
    }

    pub fn fuses(&self) -> FuseBytes {
        self.bus.fuses().bytes
    }

    /// Word address `BOOTRST` selects. The model still fetches the image from 0:
    /// no boot loader is present.
    pub fn boot_word(&self) -> u16 {
        let fuses = self.bus.fuses();
        if fuses.bootrst { fuses.boot_word } else { 0 }
    }

    /// Supply voltage. The default is 5.0 V. Below the fuse `BODLEVEL` the
    /// chip holds in reset (§11.5). Thresholds use Table 29-12's typical
    /// `VBOT` and Table 29-11's 50 mV hysteresis.
    pub fn set_vcc(&mut self, volts: f32) {
        self.vcc_mv = (f64::from(volts) * 1000.0).round() as i32;
        self.bod_low = None;
        self.consider_bod();
    }

    pub fn vcc(&self) -> f32 {
        self.vcc_mv as f32 / 1000.0
    }

    pub fn is_held_in_reset(&self) -> bool {
        self.core.is_held()
    }

    /// EEPROM image.
    pub fn eeprom(&self) -> Vec<u8> {
        self.bus.eeprom_bytes()
    }

    /// Replace EEPROM. A short slice leaves the remaining bytes at `0xFF`.
    pub fn set_eeprom(&mut self, bytes: &[u8]) {
        self.bus.set_eeprom(bytes);
    }

    /// Watchdog oscillator, in hertz. The default is the typical 128 kHz.
    pub fn set_wdt_hz(&mut self, hz: u32) {
        self.bus.set_wdt_hz(hz);
    }

    /// System clock used to turn [`Cpu::set_host_serial`]'s baud into cycles.
    /// The part itself has no frequency. The default is 16 MHz.
    pub fn set_clock_hz(&mut self, hz: u32) {
        self.clock_hz = hz.max(1);
        self.bus.set_osc_hz(self.clock_hz);
    }

    /// CPU clock: the oscillator divided by `CLKPR` (§9.11). `cycles()` counts
    /// these clocks, not oscillator clocks.
    pub fn cpu_hz(&self) -> u32 {
        self.bus.cpu_hz()
    }

    /// Oscillator cycles since power-on on this timeline. A `CLKPR` division
    /// of N makes one CPU cycle N oscillator cycles.
    pub fn oscillator_cycles(&self) -> u64 {
        self.bus.oscillator_cycles(self.cycles())
    }

    pub fn clock_div(&self) -> u32 {
        self.bus.clock_div()
    }

    /// Host transmitter on `channel`'s `RXD` pin.
    ///
    /// `baud_hz` becomes `cpu_hz / baud_hz` CPU cycles per bit, rounded down,
    /// captured at this call. A later `CLKPR` write does not retune a host
    /// baud already stored. A later [`Cpu::supply`] uses `frame` instead of
    /// the USART's registers. The setting applies to the next host frame, not
    /// one already on the wire. `baud_hz` of 0 is ignored.
    pub fn set_host_serial(&mut self, channel: Channel, baud_hz: u32, frame: SerialFrame) {
        if channel != Channel::Usart0 || baud_hz == 0 {
            return;
        }
        self.host_serial = Some((baud_hz, frame));
        self.apply_host_serial();
    }

    fn apply_host_serial(&mut self) {
        let Some((baud, frame)) = self.host_serial else {
            return;
        };
        let bit_cycles = u64::from(self.cpu_hz()) / u64::from(baud);
        if bit_cycles == 0 {
            return;
        }
        let parity = match frame.parity {
            Parity::None => 0,
            Parity::Even => 1,
            Parity::Odd => 2,
        };
        self.bus
            .set_host_serial(USART0, bit_cycles, frame.data_bits, parity, frame.stop_bits);
    }
}

fn merged_status(kind: ResetKind, old: u8) -> u8 {
    match kind {
        // §11.9.1. Power-on clears the other three flags.
        ResetKind::PowerOn => PORF,
        ResetKind::External => old | EXTRF,
        ResetKind::BrownOut => old | BORF,
        ResetKind::Watchdog => old | WDRF,
    }
}

impl Cpu {
    pub fn load_words(&mut self, words: &[u16]) {
        self.core.load_words(words);
    }

    /// Flash capacity of this part, in bytes.
    pub fn flash_bytes(&self) -> usize {
        usize::from(FLASH_WORDS) * 2
    }

    /// Load a parsed image. An image longer than [`Self::flash_bytes`] is an
    /// error. Bytes are not truncated.
    pub fn load_image(&mut self, image: &Image) -> Result<(), LoadError> {
        let flash_bytes = self.flash_bytes();
        if image.bytes().len() > flash_bytes {
            return Err(LoadError {
                bytes: image.bytes().len(),
                flash_bytes,
            });
        }
        self.load_flash(image.bytes());
        Ok(())
    }

    /// Little-endian program image. A trailing odd byte is padded with zero.
    pub fn load_flash(&mut self, bytes: &[u8]) {
        let words = bytes
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair.get(1).copied().unwrap_or(0)]))
            .collect();
        self.core.set_flash(words);
    }

    pub fn reg(&self, index: u8) -> u8 {
        self.core.reg(index)
    }

    pub fn set_reg(&mut self, index: u8, value: u8) {
        self.core.set_reg(index, value);
    }

    pub fn sreg(&self) -> u8 {
        self.core.sreg()
    }

    pub fn set_sreg(&mut self, value: u8) {
        self.core.set_sreg(value);
    }

    pub fn pc(&self) -> u16 {
        self.core.pc()
    }

    pub fn cycles(&self) -> u64 {
        self.core.cycles()
    }

    pub fn io_writes(&self) -> &[IoWrite] {
        self.bus.io_writes()
    }

    /// Diagnostic writes dropped after [`crate::IO_LOG_CAP`].
    pub fn io_write_drops(&self) -> u64 {
        self.bus.io_write_drops()
    }

    pub fn spm_notes(&self) -> &[SpmNote] {
        self.core.spm_notes()
    }

    /// How many `WDR` instructions have executed. This is not a reset count.
    pub fn wdr_executions(&self) -> u32 {
        self.core.wdr_executions()
    }

    pub fn is_sleeping(&self) -> bool {
        self.core.is_sleeping()
    }

    pub fn is_stopped(&self) -> bool {
        self.core.is_stopped()
    }

    pub fn unknown(&self) -> Option<u16> {
        self.core.unknown()
    }

    /// Plant a data-space byte without logging it.
    pub fn poke(&mut self, addr: u16, value: u8) {
        self.bus.poke(&mut self.core, addr, value);
    }

    pub fn peek(&self, addr: u16) -> u8 {
        self.bus.peek(&self.core, addr)
    }

    pub fn flash_word(&self, addr: u16) -> u16 {
        self.core.flash_word(addr)
    }

    /// Every timer clock calls `step` instead of projecting `TCNT`.
    #[cfg(test)]
    pub(crate) fn arm_stepped_timers(&mut self) {
        self.bus.arm_stepped_timers();
    }

    /// `(TCNT, flag byte)` for each timer on this part.
    #[cfg(test)]
    pub(crate) fn timer_observe(&self) -> Vec<(u16, u8)> {
        self.bus.timer_observe()
    }

    /// Run until `limit` cycles have been completed, or the core stops.
    ///
    /// While sleeping with nothing pending, time jumps to the next device
    /// event, or to `limit` when that event is sooner.
    pub fn run_until(&mut self, limit: u64) {
        self.run(limit, false);
    }

    fn run(&mut self, limit: u64, until_news: bool) {
        let div = self.clock_div();
        while self.cycles() < limit && !self.core.is_stopped() {
            if self.core.can_fetch() {
                if !self.run_free(limit, until_news, div) {
                    self.step();
                }
            } else if self.core.is_held() {
                self.advance_held(limit);
                break;
            } else {
                let before = self.cycles();
                self.coast(limit);
                if !self.core.can_fetch() && self.cycles() == before {
                    break;
                }
            }
            if until_news && self.has_news(div) {
                break;
            }
        }
    }

    /// Run `n` cycles, or until the core stops.
    ///
    /// A start-up time-out with nothing else scheduled still advances.
    pub fn run_cycles(&mut self, n: u64) {
        self.run_until(self.cycles().saturating_add(n));
    }

    /// Advance a sleeping or starting core to `limit`, the next event, or the
    /// end of the start-up delay, whichever is sooner.
    fn coast(&mut self, limit: u64) {
        if matches!(self.core.activity(), Activity::Sleep) && self.bus.sleep_wake_pending() {
            self.start_wake();
        }
        let startup_at = match self.core.activity() {
            Activity::Startup { until, .. } => until,
            _ => u64::MAX,
        };
        let counters_held = self.counters_held();
        let next = if counters_held {
            u64::MAX
        } else {
            self.bus.next_event()
        };
        let dest = limit.min(startup_at).min(next);
        if dest > self.cycles() {
            let spent = dest.wrapping_sub(self.cycles());
            self.core.advance_cycles(spent);
            if counters_held {
                self.bus.set_cycle(self.cycles());
            } else {
                self.bus.tick(spent);
            }
        }
        if self.bus.take_wdt_reset() {
            self.perform_reset(ResetKind::Watchdog, ResetWhen::Delay);
            return;
        }
        if let Activity::Startup { until, from_reset } = self.core.activity()
            && self.cycles() >= until
        {
            self.finish_startup(from_reset);
            return;
        }
        if matches!(self.core.activity(), Activity::Sleep) && self.bus.sleep_wake_pending() {
            self.start_wake();
        }
    }

    /// §11.1. Internal reset stays held through the time-out. Only the global cycle counter
    /// runs; free-running counters restart at release.
    fn counters_held(&self) -> bool {
        self.core.is_held()
            || matches!(
                self.core.activity(),
                Activity::Startup {
                    from_reset: true,
                    ..
                }
            )
    }

    fn advance_held(&mut self, limit: u64) {
        if limit > self.cycles() {
            let spent = limit.wrapping_sub(self.cycles());
            self.core.advance_cycles(spent);
            self.bus.set_cycle(self.cycles());
        }
    }

    fn start_wake(&mut self) {
        let delay = self.bus.wake_delay(self.cpu_hz());
        if delay == 0 {
            self.complete_wake();
            return;
        }
        let until = self.cycles().saturating_add(delay);
        self.core.set_activity(Activity::Startup {
            until,
            from_reset: false,
        });
    }

    fn finish_startup(&mut self, from_reset: bool) {
        if from_reset {
            self.bus.release_epoch(self.cycles());
            self.core.set_activity(Activity::Fetch);
            self.bus.arm_watchdog();
            return;
        }
        self.complete_wake();
    }

    fn complete_wake(&mut self) {
        let irq = self.bus.pending_irq() != 0;
        self.bus.end_sleep();
        self.core.set_activity(Activity::Fetch);
        self.consider_bod();
        if self.core.is_held() || !irq {
            return;
        }
        if self.service_interrupt(true) {
            self.finish_step();
        }
    }

    /// External, watchdog, and power-on complete a program when `VCC` is at
    /// or above `VBOT−`. Brown-out does not. With BOD fused off there is no
    /// threshold, so a reset completes the byte.
    fn eeprom_write_survives(&self, kind: ResetKind) -> bool {
        if kind == ResetKind::BrownOut {
            return false;
        }
        self.bus
            .bod_window()
            .is_none_or(|window| self.below_bod(window) != Some(true))
    }

    fn perform_reset(&mut self, kind: ResetKind, when: ResetWhen) {
        let now = self.cycles();
        // §8.4.2. The write finishes when VCC is still sufficient. A brown-out
        // means it was not, and the datasheet leaves that byte undefined.
        // This model keeps the old value in that case.
        if self.eeprom_write_survives(kind) {
            self.bus.commit_eeprom_pending();
        }
        let eeprom = self.bus.eeprom_bytes();
        let sram = if kind == ResetKind::PowerOn {
            None
        } else {
            Some(self.bus.sram_snapshot())
        };
        let old_flags = self.bus.mcusr();
        let fuses = self.bus.fuses().bytes;
        let vcc = self.vcc_mv;
        let bod_low = self.bod_low;
        let reset_held = self.reset_held;
        let bod_held = self.bod_held;
        let wdrs = self.core.wdr_executions();
        let epochs = self.bus.epochs();
        let program = self.core.program();
        let clock = self.clock_hz;
        let host = self.host_serial;
        let asks = self.bus.asks_for_samples();
        let pins = self.bus.pin_levels();
        let pads = self.bus.host_levels();
        let sent = self.bus.take_sent_across_reset();
        let i2c = self.bus.take_i2c_across_reset();
        *self = Self::new();
        self.clock_hz = clock;
        self.bus.set_osc_hz(clock);
        self.vcc_mv = vcc;
        self.bod_low = bod_low;
        self.reset_held = reset_held;
        self.bod_held = bod_held;
        self.host_serial = host;
        if asks {
            self.bus.ask_for_samples();
        }
        self.bus.set_fuses(fuses);
        self.bus.set_eeprom(&eeprom);
        if let Some(bytes) = sram.as_deref() {
            self.bus.restore_sram(bytes);
        }
        self.bus.set_mcusr(merged_status(kind, old_flags));
        self.apply_host_serial();
        self.core.set_program(program);
        match when {
            ResetWhen::Restart => {}
            ResetWhen::Delay => {
                self.core.set_cycles(now);
                self.bus.set_cycle(now);
                self.core.set_wdr_executions(wdrs);
                self.bus.restore_epochs(epochs, now);
                self.bus.restore_sent_across_reset(sent);
                self.bus.restore_i2c_across_reset(i2c);
                self.begin_reset_delay();
            }
            ResetWhen::Hold => {
                self.core.set_cycles(now);
                self.bus.set_cycle(now);
                self.core.set_wdr_executions(wdrs);
                self.bus.restore_epochs(epochs, now);
                self.bus.restore_sent_across_reset(sent);
                self.bus.restore_i2c_across_reset(i2c);
                self.core.set_activity(Activity::Held);
            }
        }
        // A timeline restart has no previous host. An in-run reset does: what the host
        // drives stays on the pads, and ports are back at their reset level on this
        // cycle (§11.1).
        if !matches!(when, ResetWhen::Restart) {
            for (pin, level) in pads.into_iter().enumerate() {
                if level != Level::Released {
                    self.bus.drive_pin(pin as u32, level);
                }
            }
            self.bus.note_reset_pins(pins, now);
        }
    }

    fn consider_bod(&mut self) {
        if self.core.is_sleeping() && self.bus.bod_off_in_sleep() {
            return;
        }
        let Some(window) = self.bus.bod_window() else {
            return;
        };
        match (self.bod_held, self.below_bod(window)) {
            (false, Some(true)) => {
                self.bod_held = true;
                self.perform_reset(ResetKind::BrownOut, ResetWhen::Hold);
            }
            (true, Some(false)) => {
                self.bod_held = false;
                if !self.reset_held {
                    self.begin_reset_delay();
                }
            }
            _ => {}
        }
    }

    /// `VCC` against `VBOT-` and `VBOT+`: `Some(true)` below the window, `Some(false)` above
    /// it, `None` inside it.
    fn below_bod(&self, (low, high): (u16, u16)) -> Option<bool> {
        self.bod_low.or_else(|| match self.vcc_mv {
            mv if mv < i32::from(low) => Some(true),
            mv if mv > i32::from(high) => Some(false),
            _ => None,
        })
    }

    fn begin_reset_delay(&mut self) {
        let extra = self.bus.reset_delay(self.cpu_hz());
        let until = self.cycles().saturating_add(extra);
        self.core.set_activity(Activity::Startup {
            until,
            from_reset: true,
        });
    }

    /// Runs one instruction, or enters the interrupt pending at the boundary the core sits on.
    ///
    /// A flag a host [`Self::poke`] raised between two steps is pending at that boundary, so the
    /// entry comes first, as in a run (§7.7.1). After `SEI` or `RETI` one instruction still runs
    /// before it (§7.7).
    #[inline(always)]
    pub fn step(&mut self) {
        if self.core.is_stopped() {
            return;
        }
        if !self.core.can_fetch() {
            self.step_halted();
            return;
        }
        if self.bus.pending_irq() != 0 && self.take_boundary_interrupt() {
            return;
        }
        let before = self.core.cycles();
        self.core.step(&mut self.bus);
        let spent = self.core.cycles().wrapping_sub(before);
        self.retire(spent);
    }

    /// Runs free ops, steady loads and idle stores from here while the bus is quiet and no
    /// interrupt can be taken. Returns `true` when it retired an op that reached `limit` or the
    /// next event, and `false` before any other op, which the caller steps.
    #[inline(always)]
    fn run_free(&mut self, limit: u64, until_news: bool, div: u32) -> bool {
        let irq_open = self.bus.pending_irq() != 0 && self.core.interrupts_enabled();
        if irq_open || !self.bus.quiet() || (until_news && self.has_news(div)) {
            return false;
        }
        let start = self.core.cycles();
        let room = self.bus.next_event().saturating_sub(self.bus.cycle());
        let bound = limit.min(start.saturating_add(room));
        let last = self.core.run_free(&self.bus, bound);
        let passed = self.core.cycles().wrapping_sub(start);
        match last {
            None => {
                self.bus.pass(passed);
                false
            }
            Some(spent) => {
                self.bus.pass(passed.wrapping_sub(spent));
                self.retire(spent);
                true
            }
        }
    }

    /// The bus side of an instruction that took `spent` cycles: the devices, then a `SLEEP`, a
    /// watchdog reset, or an interrupt.
    #[inline(always)]
    fn retire(&mut self, spent: u64) {
        // `SLEEP` has already set `Core::sleeping`. A watchdog reset is
        // reported only when `tick` took its slow path and fired the event.
        let watchdog = self.bus.tick(spent);
        if watchdog || self.core.is_sleeping() || self.core.is_stopped() {
            self.finish_step();
            return;
        }
        if self.bus.pending_irq() == 0 {
            return;
        }
        if self.service_interrupt(false) {
            self.finish_step();
        }
    }

    /// Sleep, reset start-up, or a held reset. Kept off the fetch path.
    #[inline(never)]
    fn step_halted(&mut self) {
        if self.core.is_held() {
            return;
        }
        let target = match self.core.activity() {
            Activity::Startup { until, .. } => until,
            _ => self.cycles().saturating_add(1),
        };
        self.coast(target);
    }

    /// A `SLEEP`, a watchdog reset, or a core that stopped inside the instruction.
    #[inline(never)]
    fn finish_step(&mut self) {
        if self.bus.take_sleep_arm() && !self.bus.begin_sleep() {
            self.core.set_activity(Activity::Fetch);
        }
        if self.bus.take_wdt_reset() {
            self.perform_reset(ResetKind::Watchdog, ResetWhen::Delay);
            return;
        }
        if self.core.is_stopped() {
            return;
        }
        if self.core.is_sleeping() {
            if self.bus.sleep_wake_pending() {
                self.start_wake();
            }
            return;
        }
        if self.bus.pending_irq() == 0 {
            return;
        }
        if self.service_interrupt(false) {
            self.finish_step();
        }
    }

    /// Enters the interrupt pending at this boundary, by vector. Whether the core advanced.
    fn take_boundary_interrupt(&mut self) -> bool {
        let before = self.cycles();
        if self.service_interrupt(false) {
            self.finish_step();
        }
        self.cycles() != before
    }

    /// `true` when the entry cycles fired a watchdog reset.
    #[inline]
    fn service_interrupt(&mut self, from_sleep: bool) -> bool {
        if self.core.irq_delay_pending() || !self.core.interrupts_enabled() {
            return false;
        }
        let lines = self.bus.pending_irq();
        if lines == 0 {
            return false;
        }
        let Some((number, word)) = irq::select(lines, VECTORS.len()) else {
            return false;
        };
        let cycles = irq::entry_cycles(IRQ_CYCLES, from_sleep);
        self.bus.ack_irq(number);
        self.core.enter_interrupt(&mut self.bus, word, cycles);
        self.bus.tick(cycles)
    }

    pub fn take_pin(&mut self) -> Option<PinReport> {
        self.bus.take_pin()
    }

    /// Pin reports dropped because the queue was full.
    pub fn pin_overflow(&self) -> u64 {
        self.bus.pin_overflow()
    }

    /// Level of every pin. [`Level::Released`] is floating. A pull-up reads high.
    pub fn pin_levels(&self) -> PinLevels {
        self.bus.pin_levels()
    }

    /// Drive `pin` ([`crate::Port::pin_id`]) from the host at the current cycle.
    ///
    /// [`Level::Released`] is high-Z. A released input with its pull-up on reads
    /// high. A released input with no pull-up reads low; that level is undefined
    /// on the die (DS40002061B Table 14-1).
    pub fn drive_pin(&mut self, pin: u32, level: Level) {
        self.bus.drive_pin(pin, level);
    }

    /// Analog voltage on `pin` ([`crate::Port::pin_id`]) at the current cycle.
    ///
    /// The same pad feeds the digital input. At `VCC` = 5.0 V, Table 30-1 gives
    /// `VIL` max = 1.5 V and `VIH` min = 3.0 V. Between those, the previous
    /// level holds. A later [`Self::drive_pin`] replaces this voltage, and a
    /// later call here replaces a digital drive.
    pub fn set_voltage(&mut self, pin: u32, volts: f32) {
        self.bus
            .set_pin_voltage(pin, crate::analog::microvolts(volts));
    }

    /// Voltage on an ADC channel. Channels with a pad (ADC0–ADC5 on the 328P)
    /// are the same wire as [`Self::set_voltage`]. ADC6 and ADC7 have no pad.
    pub fn set_channel_voltage(&mut self, channel: u8, volts: f32) {
        self.bus
            .set_channel_voltage(channel, crate::analog::microvolts(volts));
    }

    /// `AVCC`, default 5.0 V. `REFS` = `01` uses this (§24.9.1, Table 24-3).
    pub fn set_avcc(&mut self, volts: f32) {
        self.bus.set_avcc(crate::analog::microvolts(volts));
    }

    /// `AREF`, default 5.0 V. An unset `AREF` is this default, not a floating
    /// node. `REFS` = `00` uses it.
    pub fn set_aref(&mut self, volts: f32) {
        self.bus.set_aref(crate::analog::microvolts(volts));
    }

    /// Temperature in °C for the §24.8 sensor. The mapping is Table 24-2's
    /// typical column, not a calibrated offset. The default is 25 °C.
    pub fn set_temperature_c(&mut self, celsius: f32) {
        self.bus
            .set_temperature_milli(crate::analog::millidegrees(celsius));
    }

    /// §24.6. Called after ADC noise reduction has halted the CPU.
    /// Starts one conversion when `ADEN` is set, the ADC is idle, and
    /// auto-trigger is off. This call does not enter sleep.
    pub(crate) fn adc_noise_reduction(&mut self) {
        self.bus.adc_noise_reduction();
    }

    /// While `running` is false, edge detectors that need the I/O clock do not
    /// sample (DS40002061B §13, §16.6.1, §17.3). Sleep modes that stop `clkI/O`
    /// call this.
    pub(crate) fn set_io_clock_running(&mut self, running: bool) {
        self.bus.set_io_clock_running(running);
    }

    /// Queue `bytes` on `channel`.
    ///
    /// On a USART channel the bytes are a waveform on `RXD`, starting at the
    /// current cycle. See [`Channel::Usart0`].
    pub fn supply(&mut self, channel: SupplyChannel, bytes: &[u8]) {
        match channel {
            SupplyChannel::Usart0 => self.bus.supply_usart(USART0, bytes),
            SupplyChannel::Spi => self.bus.supply_spi(bytes),
        }
    }

    /// Arm a host master at cycle `at`. Returns immediately.
    ///
    /// `true` when the transfer is armed. `false` when `bytes` is empty or a
    /// transfer is already active. A `false` return changes nothing. `at` in the past starts at the current cycle.
    ///
    /// The host drives `SS`, `SCK`, and `MOSI` through the same pad events as
    /// [`Cpu::drive_pin`]. `SS` goes low at `at` and high one cycle after the
    /// last SCK edge. Each MISO byte is taken from [`Channel::SpiHostMaster`]
    /// at that frame's last SCK edge.
    ///
    /// `clock.period` is the SCK period in CPU cycles. §19.2 asks each half
    /// to be longer than two CPU cycles, so 8 is the shortest square wave
    /// that satisfies it. The model still samples a faster clock.
    #[must_use = "false leaves the current transfer unchanged"]
    pub fn queue_spi_master(&mut self, at: u64, bytes: &[u8], clock: SpiClock) -> bool {
        let at = at.max(self.cycles());
        self.bus
            .arm_host_spi(at, bytes, clock.period, clock.mode, clock.lsb_first)
    }

    /// The next byte the model delivered on `channel`, and its cycle.
    ///
    /// A USART byte's cycle is the end of its last stop bit. [`Channel::Spi`] is a master
    /// chip's MOSI byte at `SPIF`. [`Channel::SpiHostMaster`] is the MISO byte
    /// a host master sampled, at that frame's last SCK edge. `None` when the
    /// queue is empty.
    pub fn take(&mut self, channel: Channel) -> Option<(u8, u64)> {
        match channel {
            Channel::Usart0 => self.bus.usart_mut(USART0)?.take_at(),
            Channel::Spi => self.bus.take_spi(),
            Channel::SpiHostMaster => self.bus.take_spi_host_master(),
        }
    }

    /// The next asynchronous frame a USART sent whose last stop bit ended at or before
    /// oscillator cycle `through`. [`Self::take`] on [`Channel::Usart0`] pops the same queue.
    /// This one drops each Master SPI mode byte ahead of the frame, and stops at the first entry
    /// that ended after `through`. An in-run reset keeps the frames that ended at or before its
    /// cycle and drops the one on the wire. A power-on reset keeps none.
    pub fn take_sent(&mut self, through: u64) -> Option<Sent> {
        self.bus.take_sent(through)
    }

    /// The next TWI phase the master finished, at any cycle.
    pub fn take_i2c(&mut self) -> Option<crate::I2cStamp> {
        self.bus.take_i2c(u64::MAX)
    }

    /// The next TWI phase that ended at or before oscillator cycle `through`. An in-run reset
    /// keeps the phases that ended and drops the one on the bus. A power-on reset keeps none.
    pub fn take_i2c_through(&mut self, through: u64) -> Option<crate::I2cStamp> {
        self.bus.take_i2c(through)
    }
}

#[cfg(test)]
impl Cpu {
    pub(crate) fn raise_irq_at(&mut self, vector: u8, at_cycle: u64) {
        self.bus.raise_irq_at(vector, at_cycle);
    }

    pub(crate) fn clear_irq(&mut self, vector: u8) {
        self.bus.clear_irq(vector);
    }

    pub(crate) fn irq_acked(&self, vector: u8) -> bool {
        self.bus.irq_acked(vector)
    }

    pub(crate) fn peripheral_cycles(&self) -> u64 {
        self.bus.peripheral_cycles()
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Cpu, ResetKind};
    use crate::Level;

    #[test]
    fn a_clone_and_a_reset_share_the_flash() {
        let mut cpu = Cpu::new();
        cpu.load_flash(&[0; 2048]);
        let copy = cpu.clone();
        assert!(cpu.core.shares_image(&copy.core));
        cpu.reset_as(ResetKind::Watchdog);
        assert!(cpu.core.shares_image(&copy.core));
    }

    /// §11.1. No device event fires while reset holds the core, through the hold and the
    /// start-up time-out after it. A pin that moves every cycle keeps exactly two samples
    /// waiting, the synchronizer's two stages, and a sample not yet due stays.
    #[test]
    fn a_pin_that_moves_while_reset_is_held_keeps_two_samples_waiting() {
        let mut cpu = Cpu::new();
        cpu.pull_reset();
        for held in [true, false] {
            if !held {
                cpu.release_reset();
            }
            for n in 0..80 {
                cpu.run_until(cpu.cycles() + 1);
                cpu.drive_pin(18, if n % 2 == 0 { Level::High } else { Level::Low });
            }
            assert!(cpu.counters_held(), "reset holds the core, held {held}");
            let waiting = cpu.bus.extint_waiting();
            assert_eq!(waiting, 2, "samples wait, held {held}");
        }
    }
}
