//! Sleep, the fuse clock, and the module gates.
//!
//! These methods sit on [`super::Bus`] because the core steps through the bus.
//! The device indexes they use were resolved when the chip was built.

use super::Bus;
use super::domain_running;
use crate::chip::{ASSR_ADDR, ClockDomain, WAKE_CLASS, WAKE_MASK, WakeClass};
use crate::periph::Device;

/// CPU cycles to oscillator cycles from one `CLKPR` division on.
#[derive(Debug, Clone, Copy)]
struct Epoch {
    cycle: u64,
    osc: u64,
    div: u32,
}

impl Epoch {
    const START: Epoch = Epoch {
        cycle: 0,
        osc: 0,
        div: 1,
    };
}

/// The last two `CLKPR` divisions. They convert a cycle at or after the start of the first one.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Epochs {
    /// The division before `last`.
    before: Epoch,
    /// The division from its CPU cycle on, which can be a cycle past the core's.
    last: Epoch,
}

impl Epochs {
    pub(crate) const START: Epochs = Epochs {
        before: Epoch::START,
        last: Epoch::START,
    };

    /// The oscillator cycle of CPU cycle `cpu_cycles`, on either side of the last division.
    pub(crate) fn oscillator_cycles(&self, cpu_cycles: u64) -> u64 {
        let epoch = if cpu_cycles >= self.last.cycle {
            self.last
        } else {
            self.before
        };
        debug_assert!(cpu_cycles >= self.before.cycle);
        let stepped = cpu_cycles.saturating_sub(epoch.cycle);
        epoch
            .osc
            .saturating_add(stepped.saturating_mul(u64::from(epoch.div)))
    }

    /// The last CPU cycle at or before oscillator cycle `osc`, on either side of the last
    /// division.
    fn cpu_cycle(&self, osc: u64) -> u64 {
        let epoch = if osc >= self.last.osc {
            self.last
        } else {
            self.before
        };
        debug_assert!(osc >= self.before.osc);
        epoch.cycle + osc.saturating_sub(epoch.osc) / u64::from(epoch.div)
    }

    /// Division `div` from CPU cycle `from` on. A division that has not begun before `from`
    /// never does.
    fn begin(&mut self, from: u64, div: u32) {
        let osc = self.oscillator_cycles(from);
        if from > self.last.cycle {
            self.before = self.last;
        }
        self.last = Epoch {
            cycle: from,
            osc,
            div,
        };
    }
}

impl Bus {
    pub(crate) fn take_sleep_arm(&mut self) -> bool {
        let armed = self.sleep_armed;
        self.sleep_armed = false;
        armed
    }

    /// Apply Table 10-1. `false` when `SM` is reserved: the caller stays awake.
    pub(crate) fn begin_sleep(&mut self) -> bool {
        let Some(mode) = self.sleep_mode() else {
            return true;
        };
        if mode == crate::periph::system::SleepMode::Reserved {
            return false;
        }
        if mode == crate::periph::system::SleepMode::PowerSave && self.timer_as2() {
            self.log(ASSR_ADDR, 1 << 5);
        }
        self.latch_bod_off();
        self.apply_sleep_clocks(mode.io_clock(), mode.adc_clock());
        // §10.3 Idle and §24.6 noise reduction both start one conversion
        // when ADEN is set. The hook does not start a second if one is busy.
        if matches!(
            mode,
            crate::periph::system::SleepMode::Idle
                | crate::periph::system::SleepMode::AdcNoiseReduction
        ) {
            self.adc_noise_reduction();
        }
        true
    }

    pub(crate) fn end_sleep(&mut self) {
        if let Some(sys) = self.system_mut() {
            sys.end_sleep();
        }
        self.apply_sleep_clocks(true, true);
    }

    pub(crate) fn wake_delay(&self, cpu_hz: u32) -> u64 {
        let Some(mode) = self.sleep_mode() else {
            return 0;
        };
        let Some(fuses) = self.system().map(|sys| sys.fuses()) else {
            return 0;
        };
        let startup = fuses.startup.cycles(mode.delay(), cpu_hz);
        if !self.bod_off_in_sleep() {
            return startup;
        }
        // §10.2. "Approximately 60 µs" before the CPU runs again, so BOD is
        // working. `cpu_hz` already includes `CLKPR`: 960 cycles at 16 MHz.
        let extend = u64::from(cpu_hz) * 60 / 1_000_000;
        startup.saturating_add(extend)
    }

    pub(crate) fn reset_delay(&self, cpu_hz: u32) -> u64 {
        let Some(fuses) = self.system().map(|sys| sys.fuses()) else {
            return 0;
        };
        fuses.startup.cycles(crate::fuses::DelayKind::Reset, cpu_hz)
    }

    pub(crate) fn take_wdt_reset(&mut self) -> bool {
        let flag = self.wdt_reset;
        self.wdt_reset = false;
        flag
    }

    /// Table 10-1. A pending request wakes only when its class is in the
    /// mode's mask.
    /// A master `TWINT` with `TWIE` set is vector 25, class `WakeClass::Twi`.
    pub(crate) fn sleep_wake_pending(&self) -> bool {
        let lines = self.pending_irq();
        if lines == 0 {
            return false;
        }
        let Some(mode) = self.sleep_mode() else {
            return true;
        };
        self.lines_wake(lines, mode)
    }

    pub(crate) fn bods_active(&self, cycle: u64) -> bool {
        match self.slot_dev(self.index.extint) {
            Some(Device::ExtInt(ext)) => ext.bods_active(cycle),
            _ => false,
        }
    }

    pub(crate) fn bod_window(&self) -> Option<(u16, u16)> {
        let level = self.system()?.fuses().bytes.extended & 0x07;
        crate::fuses::bod_window(level)
    }

    pub(crate) fn fuses(&self) -> crate::fuses::Fuses {
        self.system()
            .map(|sys| sys.fuses())
            .unwrap_or_else(|| crate::fuses::Fuses::decode(crate::fuses::FuseBytes::BARE))
    }

    pub(crate) fn set_fuses(&mut self, bytes: crate::fuses::FuseBytes) {
        let decoded = crate::fuses::Fuses::decode(bytes);
        self.mux.set_reset_pin(!decoded.rstdisbl);
        let osc = self.osc_hz;
        // Cycle 0 is before the first instruction, including the rebuilt die
        // inside a reset. A later override keeps the running `CLKPS`.
        let reload = self.cycle == 0;
        if let Some(sys) = self.system_mut() {
            sys.apply_fuses(decoded, reload);
            let div = sys.div().max(1);
            sys.set_cpu_hz(osc / div);
        }
        self.epochs.last.div = self.system().map(|sys| sys.div().max(1)).unwrap_or(1);
        if self.cycle == 0 {
            self.epochs.last.cycle = 0;
            self.epochs.last.osc = 0;
            self.epochs.before = self.epochs.last;
        }
        let hz = self.cpu_hz();
        if let Some(ee) = self.eeprom_mut() {
            ee.set_cpu_hz(hz);
        }
        self.set_twi_hz(hz);
    }

    pub(crate) fn set_osc_hz(&mut self, hz: u32) {
        self.osc_hz = hz.max(1);
        let cpu = self.cpu_hz();
        if let Some(sys) = self.system_mut() {
            sys.set_cpu_hz(cpu);
        }
        if let Some(ee) = self.eeprom_mut() {
            ee.set_cpu_hz(cpu);
        }
        self.set_twi_hz(cpu);
    }

    pub(crate) fn cpu_hz(&self) -> u32 {
        self.osc_hz / self.clock_div()
    }

    pub(crate) fn clock_div(&self) -> u32 {
        self.epochs.last.div
    }

    /// The oscillator cycle of CPU cycle `cpu_cycles`, on either side of the last `CLKPR`
    /// division.
    pub(crate) fn oscillator_cycles(&self, cpu_cycles: u64) -> u64 {
        self.epochs.oscillator_cycles(cpu_cycles)
    }

    /// The last CPU cycle at or before oscillator cycle `osc`, on either side of the last
    /// `CLKPR` division.
    pub(crate) fn cpu_cycle(&self, osc: u64) -> u64 {
        self.epochs.cpu_cycle(osc)
    }

    pub(crate) fn set_wdt_hz(&mut self, hz: u32) {
        let now = self.cycle;
        if let Some(sys) = self.system_mut() {
            sys.set_wdt_hz(hz, now);
        }
        self.note_schedule();
    }

    pub(crate) fn mcusr(&self) -> u8 {
        self.system().map(|sys| sys.mcusr()).unwrap_or(0)
    }

    pub(crate) fn set_mcusr(&mut self, value: u8) {
        if let Some(sys) = self.system_mut() {
            sys.set_mcusr(value);
        }
        self.recompute_irq();
        self.note_schedule();
    }

    /// `WDTCSR = first`, retiring at CPU cycle `first_at`, then `WDTCSR = 0` at `second_at`.
    pub(crate) fn write_wdtcsr(&mut self, first: u8, first_at: u64, second_at: u64) {
        if let Some(sys) = self.system_mut() {
            sys.store_wdtcsr_pair(first, first_at, 0, second_at);
        }
        self.note_schedule();
    }

    /// §11.1. Prescaler, baud generator, and ADC prescaler read phase 0 here.
    pub(crate) fn release_epoch(&mut self, cycle: u64) {
        for dev in &mut self.devices {
            dev.release_epoch(cycle);
        }
    }

    pub(crate) fn arm_watchdog(&mut self) {
        let now = self.cycle;
        if let Some(sys) = self.system_mut() {
            sys.arm_wdt(now);
        }
        self.note_schedule();
    }

    /// Move to `cycle` while reset holds the core, where no device event fires. §11.1 holds the
    /// registers at their reset values, so no flag sets. It does not say whether the pin
    /// synchronizer samples meanwhile. This model lets it, so `ExtInt` takes the levels that came
    /// through it and sets no flag. A first `IN` from `PINx` reads them. A change on the last
    /// cycle of the start-up time-out reaches the edge detector after the first instruction. A
    /// first `STS` to `EICRA` or `PCMSK2` arms an edge for it only when the host has set the
    /// stored register, since reset clears the register file.
    pub(crate) fn set_cycle(&mut self, cycle: u64) {
        self.cycle = cycle;
        if let Some(Device::ExtInt(ext)) = self.slot_dev_mut(self.index.extint) {
            ext.settle(cycle);
        }
        self.note_schedule();
    }

    /// The last two divisions, which a reset keeps for [`Self::restore_epochs`].
    pub(crate) fn epochs(&self) -> Epochs {
        self.epochs
    }

    /// `epochs` from before a reset at CPU cycle `now`, then the fuses' division from `now`.
    pub(crate) fn restore_epochs(&mut self, epochs: Epochs, now: u64) {
        let div = self.clock_div();
        self.epochs = epochs;
        self.epochs.begin(now, div);
    }

    pub(crate) fn bod_off_in_sleep(&self) -> bool {
        self.system()
            .is_some_and(|sys| sys.bod_disabled_for_sleep())
    }

    pub(crate) fn latch_bod_off(&mut self) {
        if self.bods_active(self.cycle)
            && let Some(sys) = self.system_mut()
        {
            sys.latch_bod_off();
        }
    }

    pub(crate) fn prr(&self) -> u8 {
        self.system().map(|sys| sys.prr()).unwrap_or(0)
    }

    fn sleep_mode(&self) -> Option<crate::periph::system::SleepMode> {
        self.system().map(|sys| sys.mode())
    }

    fn timer_as2(&self) -> bool {
        self.index.timers[..usize::from(self.index.timer_n)]
            .iter()
            .flatten()
            .any(|slot| {
                matches!(
                    self.devices.get(usize::from(*slot)),
                    Some(Device::Timer(timer)) if timer.as2()
                )
            })
    }

    pub(super) fn poll_wdt_reset(&mut self) -> bool {
        self.system_mut().is_some_and(|sys| sys.take_wdt_reset())
    }

    /// The new `CLKPR` division counts from CPU cycle `from`.
    pub(super) fn absorb_div(&mut self, from: u64) {
        let new = self.system().map(|sys| sys.div().max(1)).unwrap_or(1);
        let old = self.clock_div();
        self.epochs.begin(from, new);
        let hz = self.cpu_hz();
        if let Some(ee) = self.eeprom_mut() {
            ee.set_cpu_hz(hz);
            ee.rescale(from, old, new);
        }
        if let Some(sys) = self.system_mut() {
            sys.set_cpu_hz(hz);
            sys.rescale(from, old, new);
        }
        self.set_twi_hz(hz);
    }

    fn set_twi_hz(&mut self, hz: u32) {
        for dev in &mut self.devices {
            if let Device::Twi(twi) = dev {
                twi.set_cpu_hz(hz);
            }
        }
    }

    pub(super) fn absorb_prr(&mut self) {
        self.refresh_prr();
    }

    fn refresh_prr(&mut self) {
        let prr = self.prr();
        let cycle = self.cycle;
        let clock = self.clock_view();
        for (index, dev) in self.devices.iter_mut().enumerate() {
            let stopped = self.row_gate[index]
                .prr_bit
                .is_some_and(|bit| prr & bit != 0);
            dev.gate(None, None, Some(stopped), cycle, &clock);
        }
        self.recompute_irq();
        self.note_schedule();
    }

    /// `io` is `clkI/O`. `adc` is `clkADC`. `PRR` still stops its own module.
    ///
    /// The prescaler shifts its origin when `clkI/O` resumes. Timers read that
    /// origin, so the prescaler updates before the clock snapshot.
    fn apply_sleep_clocks(&mut self, io: bool, adc: bool) {
        let prr = self.prr();
        let cycle = self.cycle;
        let prior = self.clock_view();
        if let Some(slot) = self.index.prescaler {
            let on = domain_running(self.row_gate[usize::from(slot)].domain, io, adc).unwrap_or(io);
            self.devices[usize::from(slot)].gate(Some(on), None, None, cycle, &prior);
        }
        let clock = self.clock_view();
        let prescaler = self.index.prescaler;
        for (index, dev) in self.devices.iter_mut().enumerate() {
            if prescaler == Some(u8::try_from(index).unwrap_or(0)) {
                continue;
            }
            let row = self.row_gate[index];
            let Some(running) = domain_running(row.domain, io, adc) else {
                continue;
            };
            let stopped = row.prr_bit.is_some_and(|bit| prr & bit != 0);
            let (io_gate, adc_gate) = match row.domain {
                ClockDomain::Adc => (None, Some(running)),
                _ => (Some(running), None),
            };
            dev.gate(io_gate, adc_gate, Some(stopped), cycle, &clock);
        }
        self.recompute_irq();
        self.note_schedule();
    }

    /// A raised line whose Table 10-1 class is set in this mode's mask.
    fn lines_wake(&self, lines: u64, mode: crate::periph::system::SleepMode) -> bool {
        let Some(mask) = WAKE_MASK.get(mode.mask_index()).copied() else {
            return true;
        };
        let mut bits = lines;
        while bits != 0 {
            let number = bits.trailing_zeros() as u8;
            bits &= !(1u64 << number);
            if number < 2 {
                continue;
            }
            let class = WAKE_CLASS
                .get(usize::from(number.wrapping_sub(1)))
                .copied()
                .unwrap_or(WakeClass::OtherIo);
            let column = match class {
                WakeClass::None => continue,
                WakeClass::Int => {
                    if self.int_is_level(number) {
                        crate::chip::WAKE_INT_LEVEL
                    } else {
                        crate::chip::WAKE_INT_EDGE
                    }
                }
                WakeClass::PinChange => crate::chip::WAKE_PIN_CHANGE,
                WakeClass::Twi => crate::chip::WAKE_TWI,
                WakeClass::Timer2 => crate::chip::WAKE_TIMER2,
                WakeClass::Eeprom => crate::chip::WAKE_EEPROM,
                WakeClass::Adc => crate::chip::WAKE_ADC,
                WakeClass::Wdt => crate::chip::WAKE_WDT,
                WakeClass::OtherIo => crate::chip::WAKE_OTHER_IO,
            };
            if mask & column != 0 {
                return true;
            }
        }
        false
    }

    fn int_is_level(&self, vector: u8) -> bool {
        match self.slot_dev(self.index.extint) {
            Some(Device::ExtInt(ext)) => ext.int_is_level(vector),
            _ => false,
        }
    }
}
