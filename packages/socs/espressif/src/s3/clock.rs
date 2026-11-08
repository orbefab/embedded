//! The chip's timeline and its CPU clock.
//!
//! Every peripheral counts [`Tick`]s, 480 MHz, the least common multiple of the clocks the CPU
//! runs on: the 40 MHz crystal through its divider, and the PLL's 80, 160 and 240 MHz. The core
//! counts its own cycles, and [`CpuClock`] maps them to ticks from the last change of
//! `SYSTEM_SYSCLK_CONF_REG` or `SYSTEM_CPU_PER_CONF_REG`. The APB clock is 80 MHz while the CPU
//! runs from the PLL, and the CPU's own clock while it runs from the crystal, as
//! `components/hal/esp32s3/include/hal/clk_tree_ll.h` gives them in IDF v5.5.5.

use orbe_xtensa::Cycle;

#[cfg(test)]
std::thread_local! {
    static EARLY_TICKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// How many times this thread asked for a tick before the clock's last change, then clears it.
#[cfg(test)]
pub(crate) fn take_early_ticks() -> u32 {
    EARLY_TICKS.with(|count| count.replace(0))
}

/// Ticks per second.
pub const TICK_HZ: u64 = 480_000_000;

/// A time on the chip's timeline, in ticks from power-on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tick(pub u64);

/// Ticks per crystal cycle.
pub(crate) const XTAL: u64 = TICK_HZ / 40_000_000;

/// `SYSTEM_SOC_CLK_SEL`, bits 10 and 11 of `SYSTEM_SYSCLK_CONF_REG`.
const SOC_CLK_SEL_SHIFT: u32 = 10;
const PRE_DIV_CNT: u32 = 0x3ff;
/// `SYSTEM_CPUPERIOD_SEL`, bits 0 and 1 of `SYSTEM_CPU_PER_CONF_REG`.
const CPUPERIOD_SEL: u32 = 0x3;
/// `SYSTEM_PLL_FREQ_SEL`, bit 2: the PLL runs at 480 MHz when set, else at 320 MHz.
const PLL_FREQ_SEL: u32 = 1 << 2;

/// The source and divider of the CPU clock, as `SYSTEM_SYSCLK_CONF_REG` and
/// `SYSTEM_CPU_PER_CONF_REG` set it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The crystal divided by `PRE_DIV_CNT + 1`.
    Xtal(u32),
    /// The PLL at 80, 160 or 240 MHz.
    Pll(u32),
}

impl Source {
    /// The source the two registers select, or `None` for the RC fast clock or a reserved
    /// value, which the model does not run. 240 MHz needs the 480 MHz PLL.
    pub(crate) fn of(sysclk_conf: u32, cpu_per_conf: u32) -> Option<Source> {
        match (sysclk_conf >> SOC_CLK_SEL_SHIFT) & 0x3 {
            0 => Some(Source::Xtal((sysclk_conf & PRE_DIV_CNT) + 1)),
            1 => match cpu_per_conf & CPUPERIOD_SEL {
                0 => Some(Source::Pll(80)),
                1 => Some(Source::Pll(160)),
                2 if cpu_per_conf & PLL_FREQ_SEL != 0 => Some(Source::Pll(240)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Ticks per CPU cycle.
    pub(crate) fn cpu(self) -> u64 {
        match self {
            Source::Xtal(divider) => XTAL * u64::from(divider),
            Source::Pll(mhz) => TICK_HZ / (u64::from(mhz) * 1_000_000),
        }
    }

    /// Ticks per APB cycle.
    pub(crate) fn apb(self) -> u64 {
        match self {
            Source::Xtal(_) => self.cpu(),
            Source::Pll(_) => TICK_HZ / 80_000_000,
        }
    }
}

/// The map from core cycles to ticks: from cycle `since` on, each cycle is `per_cycle` ticks
/// after tick `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CpuClock {
    since: Cycle,
    at: Tick,
    pub(crate) source: Source,
}

impl CpuClock {
    pub(crate) fn new(source: Source) -> CpuClock {
        CpuClock {
            since: Cycle(0),
            at: Tick(0),
            source,
        }
    }

    /// The tick of core cycle `cycle`, at or after the last change. A release build gives a
    /// cycle before it the change's tick.
    pub(crate) fn tick(&self, cycle: Cycle) -> Tick {
        #[cfg(test)]
        if cycle < self.since {
            EARLY_TICKS.with(|count| count.set(count.get().saturating_add(1)));
        }
        debug_assert!(
            cycle >= self.since,
            "cycle {cycle:?} before the clock's change at {:?}",
            self.since
        );
        Tick(self.at.0 + cycle.0.saturating_sub(self.since.0) * self.source.cpu())
    }

    /// The first core cycle at or after `tick`, which may lie before the last change.
    pub(crate) fn cycle_after(&self, tick: Tick) -> Cycle {
        let ahead = tick.0.saturating_sub(self.at.0);
        Cycle(self.since.0 + ahead.div_ceil(self.source.cpu()))
    }

    /// `tick` rounded up to a cycle boundary of this clock.
    pub(crate) fn round_up(&self, tick: Tick) -> Tick {
        self.tick(self.cycle_after(tick))
    }

    /// Switches to `source` from core cycle `now` on.
    pub(crate) fn switch(&mut self, now: Cycle, source: Source) {
        self.at = self.tick(now);
        self.since = now;
        self.source = source;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_clock_counts_from_its_last_change() {
        let mut clock = CpuClock::new(Source::Xtal(1));
        assert_eq!(clock.tick(Cycle(10)), Tick(120));
        clock.switch(Cycle(10), Source::Pll(240));
        assert_eq!(clock.tick(Cycle(13)), Tick(126));
        assert_eq!(clock.cycle_after(Tick(127)), Cycle(14));
        assert_eq!(clock.round_up(Tick(121)), Tick(122));
        assert_eq!(Source::Pll(240).apb(), 6);
        assert_eq!(Source::Xtal(2).apb(), 24);
    }

    #[test]
    fn the_two_registers_select_the_source() {
        assert_eq!(Source::of(0x0000_0000, 0), Some(Source::Xtal(1)));
        assert_eq!(Source::of(0x000a_8400, 0x4), Some(Source::Pll(80)));
        assert_eq!(Source::of(0x0000_0400, 0x6), Some(Source::Pll(240)));
        assert_eq!(Source::of(0x0000_0800, 0), None, "RC fast");
        assert_eq!(
            Source::of(0x0000_0400, 0x2),
            None,
            "240 MHz from the 320 MHz PLL"
        );
        assert_eq!(
            Source::of(0x0000_0400, 0x1),
            Some(Source::Pll(160)),
            "160 MHz from the 320 MHz PLL"
        );
    }
}
