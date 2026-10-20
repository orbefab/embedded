//! What [`crate::Timeline`] needs of the core beyond a test host: running to the next thing the
//! host must act on, the `CLKPR` epoch, samples the host answers, pads driven from a cycle the
//! core has already passed, and the host's side of the brown-out window.

use super::Cpu;
use crate::Level;

impl Cpu {
    /// [`Self::run_until`], returning after the first step that leaves the host something to
    /// act on: a pin report, a sample-and-hold waiting for [`Self::answer_sample`], or a new
    /// `CLKPR` division. A start-up or a sleep never coasts past `limit`.
    pub fn run_to_news(&mut self, limit: u64) {
        self.run(limit, true);
    }

    /// The oscillator cycle of CPU cycle `cycle`, which is at or after where the last two
    /// `CLKPR` divisions began.
    pub(crate) fn oscillator_cycle_of(&self, cycle: u64) -> u64 {
        self.bus.oscillator_cycles(cycle)
    }

    /// The last CPU cycle at or before oscillator cycle `osc`, which is at or after where the
    /// last two `CLKPR` divisions began.
    pub(crate) fn cpu_cycle_of(&self, osc: u64) -> u64 {
        self.bus.cpu_cycle(osc)
    }

    /// Ask the host for each sample-and-hold's volts. A reset keeps asking. The host answers
    /// before running past [`Self::sample_due`], or the conversion reads 0 V.
    pub fn ask_for_samples(&mut self) {
        self.bus.ask_for_samples();
    }

    /// The cycle of a sample-and-hold waiting for [`Self::answer_sample`].
    pub fn sample_due(&self) -> Option<u64> {
        self.bus.sample_due()
    }

    /// The volts of ADC0 to ADC7, `AVCC`, and `AREF`, each against `GND`, at the waiting
    /// sample-and-hold. Ignored when none waits.
    pub fn answer_sample(&mut self, channels: [f32; 8], avcc: f32, aref: f32) {
        self.bus.answer_sample(
            channels.map(crate::analog::microvolts),
            crate::analog::microvolts(avcc),
            crate::analog::microvolts(aref),
        );
    }

    /// [`Self::drive_pin`] from `cycle`, at or before [`Self::cycles`]. The synchronizer, the
    /// edge detectors, input capture, and the `T0`/`T1` inputs count from `cycle`, whatever
    /// instruction the core ran across it. What they made by [`Self::cycles`] is in place
    /// before the next instruction, and the next run takes an interrupt it raised at this
    /// boundary.
    pub(crate) fn drive_pin_at(&mut self, pin: u32, level: Level, cycle: u64) {
        self.bus.drive_pin_at(pin, level, cycle.min(self.cycles()));
        if !self.counters_held() {
            self.bus.fire_overdue();
        }
    }

    /// Before a run to `limit`, enters the interrupt pending at the boundary the core sits on,
    /// by vector (§7.7.1). Whether the core advanced.
    pub(super) fn take_boundary_interrupt(&mut self, limit: u64) -> bool {
        let before = self.cycles();
        let boundary = before < limit && self.core.can_fetch() && !self.core.is_stopped();
        if boundary && self.bus.pending_irq() != 0 && self.service_interrupt(false) {
            self.finish_step();
        }
        self.cycles() != before
    }

    /// A pin report, a sample-and-hold the host must answer, or a division other than `div`.
    pub(super) fn has_news(&self, div: u32) -> bool {
        self.bus.has_pin_reports() || self.bus.sample_due().is_some() || self.clock_div() != div
    }

    pub(crate) fn is_reset_pin(&self, pin: u32) -> bool {
        self.bus.is_reset_pin(pin)
    }

    /// `VCC` against the fuses' brown-out window, from a host that watches the rail at
    /// [`Self::brown_out_window`] itself: `true` once it fell through `VBOT-`, `false` once it
    /// rose through `VBOT+`. It holds until the next call here or to [`Self::set_vcc`]. Ignored
    /// with brown-out detection off.
    pub fn set_brown_out(&mut self, low: bool) {
        if self.bus.bod_window().is_none() {
            return;
        }
        self.bod_low = Some(low);
        self.consider_bod();
    }

    /// `VBOT-` and `VBOT+` in millivolts, from the fuses. `None` with brown-out detection off.
    pub fn brown_out_window(&self) -> Option<(u16, u16)> {
        self.bus.bod_window()
    }
}

#[cfg(test)]
mod tests {
    use super::Cpu;
    use crate::Level;

    const PD2: u32 = 18;
    const EICRA: u16 = 0x0069;
    const ADCSRA: u16 = 0x007A;

    /// The first conversion from an `ADCSRA` write at cycle 0 with divisor 4 samples here.
    const SAMPLE: u64 = 56;

    /// `I` set and `INT0` armed on a rising `PD2`, then `NOP`s. The vector is `INC r17`. The
    /// core sits one cycle before the sample with a `PD2` rise from cycle 51, so `INTF0` has
    /// been due since 53.
    fn overdue_before_the_sample() -> Cpu {
        let inc_r17 = 0x9513;
        let mut words = vec![0xC003, 0, inc_r17, 0x9518];
        words.extend([0xE003, 0x9300, EICRA, 0xE001, 0xBB0D, 0x9478]);
        words.extend([0; 80]);
        let mut cpu = Cpu::new();
        cpu.load_words(&words);
        cpu.ask_for_samples();
        cpu.poke(ADCSRA, 0xC2);
        cpu.drive_pin(PD2, Level::Low);
        cpu.run_until(SAMPLE - 1);
        assert_eq!((cpu.cycles(), cpu.sample_due()), (SAMPLE - 1, None));
        cpu.drive_pin_at(PD2, Level::High, 51);
        cpu
    }

    /// The entry's 4 cycles reach the sample-and-hold, so the run returns on the vector
    /// before `INC r17`, however far the limit lies.
    #[test]
    fn an_overdue_entry_that_reaches_a_sample_returns_before_the_vector_runs() {
        for limit in [SAMPLE, SAMPLE + 39] {
            let mut cpu = overdue_before_the_sample();
            cpu.run_to_news(limit);
            let at = (cpu.cycles(), cpu.pc(), cpu.reg(17), cpu.sample_due());
            assert_eq!(at, (SAMPLE + 3, 2, 0, Some(SAMPLE)), "limit {limit}");
        }
    }
}
