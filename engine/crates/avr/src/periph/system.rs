//! Sleep, the watchdog, the clock prescaler, and power reduction.
//!
//! DS40002061B §9.11 (`CLKPR`), §10 (`SMCR`, `PRR`, `BODS`), and §11
//! (`MCUSR`, `WDTCSR`).
//!
//! `cycles()` are CPU cycles. `CLKPR` divides the oscillator, so a watchdog
//! tick (a fixed 128 kHz) is fewer CPU cycles when the division grows. Timer
//! and USART events stay the same number of CPU cycles apart, because `clkI/O`
//! and `clkCPU` are divided together.

use crate::fuses::{self, DelayKind, FuseBytes, Fuses};
use crate::periph::Touch;
use crate::periph::eeprom::scale;

pub(crate) const REG_SMCR: u8 = 0;
pub(crate) const REG_MCUSR: u8 = 1;
pub(crate) const REG_WDTCSR: u8 = 2;
pub(crate) const REG_CLKPR: u8 = 3;
pub(crate) const REG_PRR: u8 = 4;
pub(crate) const REG_OSCCAL: u8 = 5;

pub(crate) const PORF: u8 = 1 << 0;
pub(crate) const EXTRF: u8 = 1 << 1;
pub(crate) const BORF: u8 = 1 << 2;
pub(crate) const WDRF: u8 = 1 << 3;

const SE: u8 = 1 << 0;
const SM_SHIFT: u8 = 1;
const SM_MASK: u8 = 0x07 << SM_SHIFT;

const WDIF: u8 = 1 << 7;
const WDIE: u8 = 1 << 6;
const WDP3: u8 = 1 << 5;
const WDCE: u8 = 1 << 4;
const WDE: u8 = 1 << 3;
const WDP2: u8 = 1 << 2;
const WDP1: u8 = 1 << 1;
const WDP0: u8 = 1 << 0;

const CLKPCE: u8 = 1 << 7;
const CLKPS_MASK: u8 = 0x0F;

pub(crate) const PRTWI: u8 = 1 << 7;
pub(crate) const PRTIM2: u8 = 1 << 6;
pub(crate) const PRTIM0: u8 = 1 << 5;
pub(crate) const PRTIM1: u8 = 1 << 3;
pub(crate) const PRSPI: u8 = 1 << 2;
pub(crate) const PRUSART0: u8 = 1 << 1;
pub(crate) const PRADC: u8 = 1 << 0;
const PRR_MASK: u8 = PRTWI | PRTIM2 | PRTIM0 | PRTIM1 | PRSPI | PRUSART0 | PRADC;

/// Typical watchdog oscillator, §11.8. The host can replace it.
pub(crate) const WDT_HZ_TYPICAL: u32 = 128_000;

/// Table 10-2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SleepMode {
    Idle,
    AdcNoiseReduction,
    PowerDown,
    PowerSave,
    Standby,
    ExtendedStandby,
    /// SM 100 and 101. The instruction does not sleep.
    Reserved,
}

impl SleepMode {
    pub(crate) fn from_sm(sm: u8) -> Self {
        match sm & 0x07 {
            0b000 => Self::Idle,
            0b001 => Self::AdcNoiseReduction,
            0b010 => Self::PowerDown,
            0b011 => Self::PowerSave,
            0b110 => Self::Standby,
            0b111 => Self::ExtendedStandby,
            _ => Self::Reserved,
        }
    }

    /// Table 10-1. `clkI/O` runs in Idle only.
    pub(crate) fn io_clock(self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Table 10-1. `clkADC` runs in Idle and ADC noise reduction.
    pub(crate) fn adc_clock(self) -> bool {
        matches!(self, Self::Idle | Self::AdcNoiseReduction)
    }

    pub(crate) fn delay(self) -> DelayKind {
        match self {
            Self::Idle | Self::AdcNoiseReduction | Self::Reserved => DelayKind::None,
            Self::PowerDown | Self::PowerSave => DelayKind::PowerDown,
            Self::Standby | Self::ExtendedStandby => DelayKind::Standby,
        }
    }

    /// Table 10-1, BOD disable column. Idle and noise reduction are not listed.
    pub(crate) fn allows_bod_off(self) -> bool {
        matches!(
            self,
            Self::PowerDown | Self::PowerSave | Self::Standby | Self::ExtendedStandby
        )
    }

    /// Index into the die's Table 10-1 mask. Reserved is the last slot.
    pub(crate) fn mask_index(self) -> usize {
        match self {
            Self::Idle => 0,
            Self::AdcNoiseReduction => 1,
            Self::PowerDown => 2,
            Self::PowerSave => 3,
            Self::Standby => 4,
            Self::ExtendedStandby => 5,
            Self::Reserved => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemCfg {
    /// Table 12-6. `WDT` is 7 on the 328P.
    pub wdt_vec: u8,
    pub regs: crate::regs::RegBlock,
}

#[derive(Debug, Clone)]
pub(crate) struct System {
    wdt_vec: u8,
    smcr: u8,
    mcusr: u8,
    wdtcsr: u8,
    clkpr: u8,
    prr: u8,
    /// Stored. EEPROM programming stays on Table 8-2's typical (§9.12.1).
    osccal: u8,
    fuses: Fuses,
    wdt_hz: u32,
    /// CPU cycles per oscillator cycle. 1 until `CLKPR` says otherwise.
    div: u32,
    /// `clock_hz / div`. Watchdog periods are counted in these cycles.
    cpu_hz: u32,
    /// Watchdog deadline. `u64::MAX` when the timer is stopped.
    wdt_at: u64,
    /// Set when the current period should reset rather than interrupt.
    wdt_resets: bool,
    wdce_until: u64,
    arm_wdce: bool,
    /// The period is measured from the retire cycle of the store or `WDR`.
    rearm: bool,
    clkpce_until: u64,
    arm_clkpce: bool,
    /// Latched by the `BODS` sequence for the sleep that is about to start.
    bod_off: bool,
    wdt_reset: bool,
}

impl System {
    pub(crate) fn new(cfg: SystemCfg) -> Self {
        let fuses = Fuses::decode(FuseBytes::BARE);
        let mut sys = Self {
            wdt_vec: cfg.wdt_vec,
            smcr: 0,
            mcusr: PORF,
            wdtcsr: 0,
            clkpr: 0,
            prr: 0,
            osccal: 0,
            fuses,
            wdt_hz: WDT_HZ_TYPICAL,
            div: 1,
            cpu_hz: 16_000_000,
            wdt_at: u64::MAX,
            wdt_resets: false,
            wdce_until: 0,
            arm_wdce: false,
            rearm: false,
            clkpce_until: 0,
            arm_clkpce: false,
            bod_off: false,
            wdt_reset: false,
        };
        sys.apply_fuses(fuses, true);
        sys
    }

    /// `reload_clock` applies the fuse `CLKPS`. A mid-run override leaves the
    /// program's current prescaler; the next reset loads the fuse value.
    pub(crate) fn apply_fuses(&mut self, fuses: Fuses, reload_clock: bool) {
        self.fuses = fuses;
        if reload_clock {
            let clkps = fuses.initial_clkps();
            self.clkpr = clkps;
            self.div = fuses.clock_div();
        }
        if fuses.wdton {
            self.wdtcsr = (self.wdtcsr & !(WDIE | WDCE)) | WDE;
            self.wdt_resets = true;
        }
    }

    pub(crate) fn fuses(&self) -> Fuses {
        self.fuses
    }

    pub(crate) fn div(&self) -> u32 {
        self.div
    }

    pub(crate) fn take_wdt_reset(&mut self) -> bool {
        let reset = self.wdt_reset;
        self.wdt_reset = false;
        reset
    }

    pub(crate) fn set_cpu_hz(&mut self, hz: u32) {
        self.cpu_hz = hz.max(1);
    }

    pub(crate) fn set_wdt_hz(&mut self, hz: u32, now: u64) {
        let old = self.wdt_hz.max(1);
        let new = hz.max(1);
        if self.wdt_at != u64::MAX && self.wdt_at > now && old != new {
            // Remaining watchdog cycles keep their count. A faster oscillator
            // spends fewer CPU cycles on them.
            let left_cpu = self.wdt_at - now;
            let left = left_cpu.saturating_mul(u64::from(old)) / u64::from(new);
            self.wdt_at = now.saturating_add(left.max(1));
        }
        self.wdt_hz = new;
    }

    pub(crate) fn mcusr(&self) -> u8 {
        self.mcusr
    }

    pub(crate) fn set_mcusr(&mut self, value: u8) {
        self.mcusr = value & (PORF | EXTRF | BORF | WDRF);
        self.force_wde();
    }

    pub(crate) fn prr(&self) -> u8 {
        self.prr
    }

    pub(crate) fn se(&self) -> bool {
        self.smcr & SE != 0
    }

    pub(crate) fn mode(&self) -> SleepMode {
        SleepMode::from_sm(self.smcr >> SM_SHIFT)
    }

    pub(crate) fn bod_disabled_for_sleep(&self) -> bool {
        self.bod_off && self.fuses.bod_typ_mv.is_some() && self.mode().allows_bod_off()
    }

    pub(crate) fn latch_bod_off(&mut self) {
        if self.fuses.bod_typ_mv.is_some() && self.mode().allows_bod_off() {
            self.bod_off = true;
        }
    }

    pub(crate) fn end_sleep(&mut self) {
        self.bod_off = false;
    }

    /// Arm the watchdog from the current mode. Called when a reset delay ends.
    pub(crate) fn arm_wdt(&mut self, now: u64) {
        self.schedule(now);
    }

    pub(crate) fn wdr(&mut self, _now: u64) {
        if self.running() {
            self.rearm = true;
            self.wdt_at = u64::MAX;
        }
    }

    pub(crate) fn read(&mut self, reg: u8) -> (u8, bool) {
        (self.peek(reg), false)
    }

    pub(crate) fn peek(&self, reg: u8) -> u8 {
        match reg {
            REG_SMCR => self.smcr,
            REG_MCUSR => self.mcusr,
            REG_WDTCSR => self.wdtcsr,
            REG_CLKPR => self.clkpr,
            REG_PRR => self.prr,
            REG_OSCCAL => self.osccal,
            _ => 0,
        }
    }

    pub(crate) fn write(&mut self, reg: u8, value: u8, now: u64) -> Touch {
        let mut touch = Touch {
            irq: true,
            schedule: true,
            windows: true,
            ..Touch::default()
        };
        match reg {
            REG_SMCR => {
                let sm = value & SM_MASK;
                let se = value & SE;
                // Reserved SM still stores. SLEEP will not enter it.
                self.smcr = sm | se;
            }
            REG_MCUSR => {
                // Flags clear when the written bit is zero (§11.9.1).
                self.mcusr &= value | !0x0F;
                self.mcusr &= 0x0F;
                self.force_wde();
            }
            REG_WDTCSR => self.write_wdt(value, now),
            REG_CLKPR => {
                touch.div = self.write_clkpr(value, now);
            }
            REG_PRR => {
                self.prr = value & PRR_MASK;
                touch.prr = true;
            }
            REG_OSCCAL => self.osccal = value,
            _ => {
                touch.irq = false;
                touch.schedule = false;
            }
        }
        touch
    }

    pub(crate) fn poke(&mut self, reg: u8, value: u8, now: u64) -> Touch {
        let out = self.write(reg, value, now);
        self.commit_windows(now);
        out
    }

    fn write_wdt(&mut self, value: u8, now: u64) {
        if value & WDIF != 0 {
            self.wdtcsr &= !WDIF;
        }
        let window = self.wdce_until != 0 && now < self.wdce_until;
        let opening = value & (WDCE | WDE) == (WDCE | WDE);
        let old_wdp = wdp_code(self.wdtcsr);
        let old_wde = self.wdtcsr & WDE != 0;
        if opening {
            // §11.8.2 step 1. The prescaler write is the next store.
            self.arm_wdce = true;
            self.wdtcsr |= WDCE;
            if !self.fuses.wdton {
                self.wdtcsr |= WDE;
            }
        } else if window && value & WDCE == 0 {
            let wde = value & WDE != 0 || self.mcusr & WDRF != 0 || self.fuses.wdton;
            self.wdtcsr = (self.wdtcsr & !(WDE | WDP3 | WDP2 | WDP1 | WDP0))
                | (if wde { WDE } else { 0 })
                | wdp_bits(wdp_code(value));
            self.wdce_until = 0;
            self.arm_wdce = false;
            self.wdtcsr &= !WDCE;
        } else if value & WDE != 0 && !self.fuses.wdton {
            // Setting WDE does not need the window. Clearing it does.
            self.wdtcsr |= WDE;
        }
        if self.fuses.wdton {
            self.wdtcsr = (self.wdtcsr & !WDIE) | WDE;
        } else if value & WDIE != 0 {
            self.wdtcsr |= WDIE;
        } else {
            self.wdtcsr &= !WDIE;
        }
        self.force_wde();
        let wdp_changed = wdp_code(self.wdtcsr) != old_wdp;
        let wde_rose = !old_wde && self.wdtcsr & WDE != 0;
        if wdp_changed || wde_rose || !self.running() || self.wdt_at == u64::MAX {
            self.rearm = true;
            self.wdt_at = u64::MAX;
        } else {
            self.wdt_resets = self.timeout_resets();
        }
    }

    fn write_clkpr(&mut self, value: u8, now: u64) -> bool {
        if value & CLKPCE != 0 && value & !CLKPCE == 0 {
            self.arm_clkpce = true;
            self.clkpr |= CLKPCE;
            return false;
        }
        let window = self.clkpce_until != 0 && now < self.clkpce_until;
        if !window || value & CLKPCE != 0 {
            return false;
        }
        let code = value & CLKPS_MASK;
        self.clkpce_until = 0;
        self.arm_clkpce = false;
        self.clkpr &= !CLKPCE;
        if fuses::clkps_reserved(code) || code == self.clkpr & CLKPS_MASK {
            return false;
        }
        self.clkpr = code;
        self.div = fuses::clkps_div(code);
        true
    }

    /// The watchdog's deadlines in CPU cycles, for a division of `old` replaced by `new`
    /// from CPU cycle `from`.
    pub(crate) fn rescale(&mut self, from: u64, old: u32, new: u32) {
        self.wdt_at = scale(self.wdt_at, from, old, new);
        if self.wdce_until != 0 {
            self.wdce_until = scale(self.wdce_until, from, old, new);
        }
    }

    pub(crate) fn commit_windows(&mut self, retire: u64) {
        if self.arm_wdce {
            self.arm_wdce = false;
            self.wdce_until = retire.saturating_add(4);
        }
        if self.rearm {
            self.rearm = false;
            self.schedule(retire);
        }
        if self.arm_clkpce {
            self.arm_clkpce = false;
            self.clkpce_until = retire.saturating_add(4);
        }
    }

    fn force_wde(&mut self) {
        if self.fuses.wdton || self.mcusr & WDRF != 0 {
            self.wdtcsr |= WDE;
        }
        if self.fuses.wdton {
            self.wdtcsr &= !WDIE;
            self.wdt_resets = true;
        }
    }

    fn running(&self) -> bool {
        if self.fuses.wdton {
            return true;
        }
        self.wdtcsr & (WDE | WDIE) != 0
    }

    fn schedule(&mut self, now: u64) {
        if !self.running() {
            self.wdt_at = u64::MAX;
            self.wdt_resets = false;
            return;
        }
        self.wdt_resets = self.timeout_resets();
        self.wdt_at = now.saturating_add(self.period_cycles());
    }

    fn period_cycles(&self) -> u64 {
        let n = wdp_cycles(wdp_code(self.wdtcsr));
        let hz = u64::from(self.cpu_hz.max(1));
        let wdt = u64::from(self.wdt_hz.max(1));
        (n.saturating_mul(hz) / wdt).max(1)
    }

    fn timeout_resets(&self) -> bool {
        if self.fuses.wdton {
            return true;
        }
        // Table 11-1. Interrupt-and-reset resets only after WDIE has cleared.
        self.wdtcsr & WDE != 0 && self.wdtcsr & WDIE == 0
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        if self.wdtcsr & WDIF != 0 && self.wdtcsr & WDIE != 0 && !self.fuses.wdton {
            1u64 << self.wdt_vec
        } else {
            0
        }
    }

    pub(crate) fn ack(&mut self, vector: u8) {
        if vector != self.wdt_vec {
            return;
        }
        self.wdtcsr &= !WDIF;
        // §11.9.2. Interrupt-and-reset: entry clears WDIE and WDIF.
        if self.wdtcsr & WDE != 0 {
            self.wdtcsr &= !WDIE;
            self.wdt_resets = true;
        }
    }

    pub(crate) fn next_event(&self) -> u64 {
        let wdce = if self.wdce_until == 0 {
            u64::MAX
        } else {
            self.wdce_until
        };
        let clk = if self.clkpce_until == 0 {
            u64::MAX
        } else {
            self.clkpce_until
        };
        self.wdt_at.min(wdce).min(clk)
    }

    /// A watchdog reset is reported through `take_wdt_reset`. An interrupt
    /// sets `WDIF`. A window close only clears the enable bit.
    pub(crate) fn fire(&mut self, at: u64) {
        if self.wdce_until != 0 && self.wdce_until == at {
            self.wdce_until = 0;
            self.wdtcsr &= !WDCE;
        }
        if self.clkpce_until != 0 && self.clkpce_until == at {
            self.clkpce_until = 0;
            self.clkpr &= !CLKPCE;
        }
        if self.wdt_at == at {
            if self.wdt_resets || self.timeout_resets() {
                self.wdt_reset = true;
                self.wdt_at = u64::MAX;
            } else {
                self.wdtcsr |= WDIF;
                if self.wdtcsr & WDE != 0 && self.wdtcsr & WDIE != 0 {
                    // §11.9.2. The vector entry clears WDIE. If that entry does
                    // not happen, the next timeout still resets.
                    self.wdt_resets = true;
                }
                self.wdt_at = at.saturating_add(self.period_cycles());
            }
        }
    }
}

/// Table 11-2. The code is `WDP3 WDP2 WDP1 WDP0`.
pub(crate) fn wdp_cycles(code: u8) -> u64 {
    match code & 0x0F {
        0 => 2_048,
        1 => 4_096,
        2 => 8_192,
        3 => 16_384,
        4 => 32_768,
        5 => 65_536,
        6 => 131_072,
        7 => 262_144,
        8 => 524_288,
        9 => 1_048_576,
        _ => 2_048,
    }
}

fn wdp_code(wdtcsr: u8) -> u8 {
    let low = wdtcsr & (WDP2 | WDP1 | WDP0);
    // WDP2 is bit 2 of the register, which is already bit 2 of the code.
    let high = if wdtcsr & WDP3 != 0 { 0x08 } else { 0 };
    high | (low & 0x07)
}

fn wdp_bits(code: u8) -> u8 {
    let mut bits = code & 0x07;
    if code & 0x08 != 0 {
        bits |= WDP3;
    }
    bits
}
