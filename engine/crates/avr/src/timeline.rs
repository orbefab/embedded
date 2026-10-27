//! The core on a host's timeline. Time is the oscillator's cycle count from power-on, which no
//! `CLKPR` write changes, so a host converts it to its own time with one fixed rate.
//!
//! The core runs whole instructions, so it can sit a few cycles past where the host asked it to
//! stop. Every effect keeps its own cycle anyway: a pin's drive changes in [`Timeline::drives`]
//! only once the host has reached that cycle, and a pad the host drives changes at the cycle the
//! host reached.

use crate::{Channel, Cpu, I2cStamp, Level, PIN_COUNT, PinDrive, Sent};

/// Where [`Timeline::run_until`] stopped: at oscillator cycle `at` a drive changed, or the ADC
/// samples, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Halt {
    pub at: u64,
    /// The ADC's sample-and-hold is at `at`, waiting for [`Timeline::sample`].
    pub sample: bool,
}

#[derive(Debug)]
pub struct Timeline {
    cpu: Cpu,
    /// Oscillator cycles. Everything up to here is done.
    reached: u64,
    /// Each pin's drive at `reached`.
    drives: [PinDrive; PIN_COUNT],
    /// Drive changes the core made past `reached`: oscillator cycle, pin, drive. Ascending
    /// cycle, and in the order the core made them within one.
    pending: Vec<(u64, u32, PinDrive)>,
    /// The oscillator cycle of a sample-and-hold the host has not answered.
    sample: Option<u64>,
}

clone_fields!(Timeline {
    cpu,
    reached,
    drives,
    pending,
    sample
});

impl Timeline {
    /// A programmed, powered core at oscillator cycle 0. Its ADC asks [`Timeline::sample`] for
    /// every sample-and-hold.
    pub fn new(mut cpu: Cpu) -> Timeline {
        cpu.ask_for_samples();
        let levels = cpu.pin_levels();
        let drives = std::array::from_fn(|pin| levels.drive(pin as u32).unwrap_or(PinDrive::Off));
        Timeline {
            cpu,
            reached: 0,
            drives,
            pending: Vec::new(),
            sample: None,
        }
    }

    /// Runs from where it reached and stops at the first oscillator cycle in `(reached, limit]`
    /// where a drive changes or the ADC samples. `None` once everything up to `limit` is done.
    /// Two calls to `l1` then `l2` leave the state one call to `l2` does.
    pub fn run_until(&mut self, limit: u64) -> Option<Halt> {
        assert!(
            limit >= self.reached,
            "a core runs only past where it reached"
        );
        assert!(
            self.sample.is_none_or(|at| at > self.reached),
            "answer the sample before running on"
        );
        loop {
            let edge = self.pending.first().map(|&(at, _, _)| at);
            match edge.into_iter().chain(self.sample).min() {
                Some(at) if at <= limit => {
                    if at <= self.done() {
                        return Some(self.halt(at));
                    }
                }
                _ if self.cpu.is_stopped() || self.cpu.cycles() >= self.cpu.cpu_cycle_of(limit) => {
                    self.reached = limit;
                    return None;
                }
                _ => {}
            }
            self.advance(limit);
        }
    }

    /// Each pin's drive, by pin id, at the cycle it reached.
    pub fn drives(&self) -> &[PinDrive; PIN_COUNT] {
        &self.drives
    }

    /// The volts of ADC0 to ADC7, `AVCC`, and `AREF` at the halt that asked for them.
    pub fn sample(&mut self, channels: [f32; 8], avcc: f32, aref: f32) {
        assert_eq!(
            self.sample,
            Some(self.reached),
            "a sample arrives at its halt"
        );
        self.sample = None;
        self.cpu.answer_sample(channels, avcc, aref);
    }

    /// A pad's level, from the CPU cycle it reached on, whatever instruction the core ran
    /// across that cycle. The synchronizer times the core's first sight of it. While `PC6` is
    /// `RESET`, low on it holds the chip in reset from the instruction boundary the core is on.
    pub fn set_pad(&mut self, pin: u32, high: bool) {
        let level = if high { Level::High } else { Level::Low };
        let cycle = self.cpu.cpu_cycle_of(self.reached);
        if !self.cpu.is_reset_pin(pin) {
            self.cpu.drive_pin_at(pin, level, cycle);
        } else if high {
            self.cpu.drive_pin(pin, level);
            self.cpu.release_reset();
        } else {
            self.cpu.pull_reset();
            self.cpu.drive_pin(pin, level);
        }
        self.collect_input();
    }

    /// `VCC` below the brown-out window, from where it reached on. See [`Cpu::set_brown_out`].
    pub fn set_brown_out(&mut self, low: bool) {
        self.cpu.set_brown_out(low);
        self.collect_input();
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// See [`Cpu::take`].
    pub fn take(&mut self, channel: Channel) -> Option<(u8, u64)> {
        self.cpu.take(channel)
    }

    /// See [`Cpu::take_sent`]. `through` is an oscillator cycle, the time this timeline runs on.
    pub fn take_sent(&mut self, through: u64) -> Option<Sent> {
        self.cpu.take_sent(through)
    }

    /// See [`Cpu::take_i2c_through`]. `through` is an oscillator cycle.
    pub fn take_i2c_through(&mut self, through: u64) -> Option<I2cStamp> {
        self.cpu.take_i2c_through(through)
    }

    /// The oscillator cycle at and before which the core has made every report.
    fn done(&self) -> u64 {
        if self.cpu.is_stopped() {
            return u64::MAX;
        }
        self.cpu.oscillator_cycles()
    }

    fn halt(&mut self, at: u64) -> Halt {
        let due = self.pending.partition_point(|&(when, _, _)| when <= at);
        for (_, pin, drive) in self.pending.drain(..due) {
            self.drives[pin as usize] = drive;
        }
        self.reached = at;
        Halt {
            at,
            sample: self.sample == Some(at),
        }
    }

    /// Runs the core at least one step toward `limit`, then files what it reported.
    fn advance(&mut self, limit: u64) {
        let target = self.cpu.cpu_cycle_of(limit).max(self.cpu.cycles() + 1);
        self.cpu.run_to_news(target);
        self.collect(0);
    }

    /// What an input caused comes at least one CPU cycle after where it reached.
    fn collect_input(&mut self) {
        let next = self.cpu.cpu_cycle_of(self.reached) + 1;
        self.collect(self.cpu.oscillator_cycle_of(next));
    }

    fn collect(&mut self, floor: u64) {
        while let Some(report) = self.cpu.take_pin() {
            let at = self.cpu.oscillator_cycle_of(report.cycle).max(floor);
            assert!(at > self.reached, "a report after the cycle a host reached");
            let last = self
                .pending
                .iter()
                .rev()
                .find(|&&(_, pin, _)| pin == report.pin)
                .map_or(self.drives[report.pin as usize], |&(_, _, drive)| drive);
            if last == report.drive {
                continue;
            }
            let k = self.pending.partition_point(|&(when, _, _)| when <= at);
            self.pending.insert(k, (at, report.pin, report.drive));
        }
        if self.sample.is_none()
            && let Some(cycle) = self.cpu.sample_due()
        {
            self.sample = Some(self.cpu.oscillator_cycle_of(cycle).max(floor));
        }
    }
}
