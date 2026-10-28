//! SYSTIMER as a counter, and the watchdogs as timers that firmware feeds or disables.
//!
//! Both count on the 40 MHz crystal, which the CPU also runs on until clocks are modeled, so a
//! CPU cycle is a crystal cycle. SYSTIMER counts at 16 MHz, 2 counts every 5 cycles. Its
//! alarms and interrupts are not modeled.
//!
//! The watchdogs: the RTC watchdog, and the main watchdog of each timer group. Each runs while
//! its `EN` bit is set and its first stage acts, or while its flash boot bit is set. That bit is
//! set at power-on on the RTC watchdog and the timer group 0 watchdog, as the boot ROM leaves
//! them, so firmware that does not disable or feed them meets the first stage. A watchdog that
//! runs out its first stage ends the run, whatever that stage's action. Every write but to the
//! write-protect register needs the key there. Writes take effect at once, and
//! `TIMG_WDT_CONF_UPDATE_EN` is not needed. A hold written below the count runs out at the write.
//! The super watchdog's two registers are kept, unlocked at power-on, and it never runs out.

use sim_xtensa::Cycle;

/// Crystal cycles per RTC slow clock cycle, as 40 MHz over the 136 kHz RC slow clock's nominal
/// rate.
const SLOW_NUM: u64 = 40_000_000;
const SLOW_DEN: u64 = 136_000;

/// The RTC watchdog's first stage counts `WDTCONFIG1` times `2 << WDT_DELAY_SEL`, with that
/// eFuse 0 until eFuses are modeled. IDF v5.5.5 writes stage 0 shifted down by that much, in
/// `components/hal/esp32s3/include/hal/rwdt_ll.h:109-124` and
/// `components/esp_hw_support/rtc_wdt.c:93-95`.
const RTC_STAGE0_SCALE: u64 = 2;

const SYSTIMER_BITS: u64 = (1 << 52) - 1;
const UNIT_WORK_EN: [u32; 2] = [1 << 30, 1 << 29];
const VALUE_VALID: u32 = 1 << 29;
const UPDATE: u32 = 1 << 30;

/// The counts at crystal cycle `cycle`, from power-on.
fn ticks(cycle: Cycle) -> u64 {
    cycle.0 * 2 / 5
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Systimer {
    conf: u32,
    units: [Unit; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Unit {
    /// The count at `since`, from which it counts on while its unit works.
    base: u64,
    since: Cycle,
    latched: u64,
    valid: bool,
    load: u64,
}

impl Systimer {
    pub(crate) fn new() -> Systimer {
        Systimer {
            conf: 0x4600_0000,
            units: [Unit::default(), Unit::default()],
        }
    }

    fn value(&self, unit: usize, now: Cycle) -> u64 {
        let u = &self.units[unit];
        if self.conf & UNIT_WORK_EN[unit] == 0 {
            return u.base;
        }
        (u.base + ticks(now) - ticks(u.since)) & SYSTIMER_BITS
    }

    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        Some(match offset {
            0x00 => self.conf,
            0x04 | 0x08 => {
                let unit = (offset / 4 - 1) as usize;
                if self.units[unit].valid {
                    VALUE_VALID
                } else {
                    0
                }
            }
            0x0c | 0x14 => (self.units[unit_of(offset, 0x0c)].load >> 32) as u32,
            0x10 | 0x18 => self.units[unit_of(offset, 0x10)].load as u32,
            0x40 | 0x48 => (self.units[unit_of(offset, 0x40)].latched >> 32) as u32,
            0x44 | 0x4c => self.units[unit_of(offset, 0x44)].latched as u32,
            _ => return None,
        })
    }

    pub(crate) fn write(&mut self, now: Cycle, offset: u32, value: u32) -> bool {
        match offset {
            0x00 => {
                for unit in 0..2 {
                    let base = self.value(unit, now);
                    let u = &mut self.units[unit];
                    (u.base, u.since) = (base, now);
                }
                self.conf = value;
            }
            0x04 | 0x08 => {
                let unit = (offset / 4 - 1) as usize;
                if value & UPDATE != 0 {
                    self.units[unit].latched = self.value(unit, now);
                    self.units[unit].valid = true;
                }
            }
            0x0c | 0x14 => {
                let load = &mut self.units[unit_of(offset, 0x0c)].load;
                *load = (*load & 0xffff_ffff) | (u64::from(value & 0xf_ffff) << 32);
            }
            0x10 | 0x18 => {
                let load = &mut self.units[unit_of(offset, 0x10)].load;
                *load = (*load & !0xffff_ffff) | u64::from(value);
            }
            0x5c | 0x60 => {
                let u = &mut self.units[unit_of(offset, 0x5c)];
                if value & 1 != 0 {
                    (u.base, u.since) = (u.load, now);
                }
            }
            _ => return false,
        }
        true
    }
}

/// Which unit a register pair at `first` and `first + 8` (or `+ 4` for the load strobes)
/// belongs to.
fn unit_of(offset: u32, first: u32) -> usize {
    usize::from(offset != first)
}

/// A watchdog, as a diagnostic names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watchdog {
    Rtc,
    Timg0,
    Timg1,
}

impl Watchdog {
    pub fn name(self) -> &'static str {
        match self {
            Watchdog::Rtc => "RTC",
            Watchdog::Timg0 => "timer group 0",
            Watchdog::Timg1 => "timer group 1",
        }
    }
}

/// `RTC_CNTL_WDT_WKEY_VALUE` and `TIMG_WDT_WKEY`, one value.
const WDT_KEY: u32 = 0x50d8_3aa1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Watchdogs {
    timg: [Timer; 2],
    rtc: Timer,
    /// `RTC_CNTL_SWD_CONF_REG` and `RTC_CNTL_SWD_WPROTECT_REG`.
    swd: [u32; 2],
}

/// One watchdog: its config words from `CONFIG0`, its write-protect word, and the cycle its
/// count last started.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Timer {
    config: [u32; 6],
    wprotect: u32,
    fed: Cycle,
    /// The last write's cycle. A hold written below the count runs out there.
    written: Cycle,
    /// Where `EN`, the first stage's action and the flash boot bit sit in `CONFIG0`.
    layout: Layout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    en: u32,
    stg0_shift: u32,
    stg0_mask: u32,
    flashboot: u32,
    /// Whether the flash boot bit runs the timer. Only timer group 0's does.
    flashboot_runs: bool,
    /// Whether it counts the RTC slow clock, else the 40 MHz APB clock through its prescaler.
    slow: bool,
}

impl Timer {
    fn runs(&self) -> bool {
        let c = self.config[0];
        let l = self.layout;
        let stage = (c >> l.stg0_shift) & l.stg0_mask;
        (c & l.en != 0 && stage != 0) || (l.flashboot_runs && c & l.flashboot != 0)
    }

    /// The cycle its first stage runs out, while it runs.
    fn deadline(&self) -> Option<Cycle> {
        if !self.runs() {
            return None;
        }
        let hold = u64::from(self.config[if self.layout.slow { 1 } else { 2 }]);
        let cycles = if self.layout.slow {
            (hold * RTC_STAGE0_SCALE * SLOW_NUM).div_ceil(SLOW_DEN)
        } else {
            hold * u64::from((self.config[1] >> 16).max(1))
        };
        Some(Cycle((self.fed.0 + cycles.max(1)).max(self.written.0)))
    }

    fn write(&mut self, now: Cycle, index: Option<usize>, value: u32, feed: bool) -> bool {
        if self.wprotect != WDT_KEY {
            return true;
        }
        let ran = self.runs();
        self.written = now;
        if let Some(index) = index {
            self.config[index] = value;
        }
        if feed || (!ran && self.runs()) {
            self.fed = now;
        }
        true
    }
}

impl Watchdogs {
    pub(crate) fn new() -> Watchdogs {
        let timg = |flashboot_runs| Timer {
            config: [
                0x0004_c000,
                0x0001_0000,
                26_000_000,
                134_217_727,
                1_048_575,
                1_048_575,
            ],
            wprotect: WDT_KEY,
            fed: Cycle(0),
            written: Cycle(0),
            layout: Layout {
                en: 1 << 31,
                stg0_shift: 29,
                stg0_mask: 0x3,
                flashboot: 1 << 14,
                flashboot_runs,
                slow: false,
            },
        };
        Watchdogs {
            timg: [timg(true), timg(false)],
            rtc: Timer {
                config: [0x0001_3214, 200_000, 80_000, 0xfff, 0xfff, 0],
                wprotect: WDT_KEY,
                fed: Cycle(0),
                written: Cycle(0),
                layout: Layout {
                    en: 1 << 31,
                    stg0_shift: 28,
                    stg0_mask: 0x7,
                    flashboot: 1 << 12,
                    flashboot_runs: true,
                    slow: true,
                },
            },
            swd: [SWD_CONF_RESET, SWD_KEY],
        }
    }

    /// The first watchdog to run out, and when.
    pub(crate) fn deadline(&self) -> Option<(Cycle, Watchdog)> {
        [
            (self.rtc.deadline(), Watchdog::Rtc),
            (self.timg[0].deadline(), Watchdog::Timg0),
            (self.timg[1].deadline(), Watchdog::Timg1),
        ]
        .into_iter()
        .filter_map(|(at, which)| Some((at?, which)))
        .min_by_key(|&(at, _)| at)
    }

    /// A timer group's watchdog register at `offset` in the group's block.
    pub(crate) fn read_timg(&self, group: usize, offset: u32) -> Option<u32> {
        let t = &self.timg[group];
        match offset {
            0x48..=0x5c => Some(t.config[((offset - 0x48) / 4) as usize]),
            0x60 => Some(0),
            0x64 => Some(t.wprotect),
            _ => None,
        }
    }

    pub(crate) fn write_timg(&mut self, now: Cycle, group: usize, offset: u32, value: u32) -> bool {
        let t = &mut self.timg[group];
        match offset {
            0x48..=0x5c => t.write(now, Some(((offset - 0x48) / 4) as usize), value, false),
            0x60 => t.write(now, None, value, true),
            0x64 => {
                t.wprotect = value;
                true
            }
            _ => false,
        }
    }

    /// An RTC_CNTL register the watchdogs own, at `offset` in that block.
    pub(crate) fn read_rtc(&self, offset: u32) -> Option<u32> {
        match offset {
            0x98..=0xa8 => Some(self.rtc.config[((offset - 0x98) / 4) as usize]),
            0xac => Some(0),
            0xb0 => Some(self.rtc.wprotect),
            0xb4 => Some(self.swd[0]),
            0xb8 => Some(self.swd[1]),
            _ => None,
        }
    }

    pub(crate) fn write_rtc(&mut self, now: Cycle, offset: u32, value: u32) -> bool {
        match offset {
            0x98..=0xa8 => {
                let index = ((offset - 0x98) / 4) as usize;
                self.rtc.write(now, Some(index), value, false)
            }
            0xac => self.rtc.write(now, None, value, value & (1 << 31) != 0),
            0xb0 => {
                self.rtc.wprotect = value;
                true
            }
            0xb4 => {
                if self.swd[1] == SWD_KEY {
                    self.swd[0] = value;
                }
                true
            }
            0xb8 => {
                self.swd[1] = value;
                true
            }
            _ => false,
        }
    }
}

/// `RTC_CNTL_SWD_WKEY_VALUE`, which is also the protect word's reset value, so the super
/// watchdog starts unlocked. IDF v5.5.5,
/// `components/soc/esp32s3/register/soc/rtc_cntl_reg.h:2168`.
const SWD_KEY: u32 = 0x8f1d_312a;
/// `RTC_CNTL_SWD_SIGNAL_WIDTH` 300 at bit 18, the rest 0, from the same header's line 2142.
const SWD_CONF_RESET: u32 = 300 << 18;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systimer_counts_two_every_five_cycles_and_latches_on_update() {
        let mut timer = Systimer::new();
        timer.write(Cycle(1000), 0x04, UPDATE);
        assert_eq!(timer.read(0x04), Some(VALUE_VALID));
        assert_eq!(timer.read(0x44), Some(400));
        timer.write(Cycle(1003), 0x04, UPDATE);
        assert_eq!(timer.read(0x44), Some(401));
        timer.write(Cycle(1003), 0x08, UPDATE);
        assert_eq!(
            timer.read(0x4c),
            Some(0),
            "unit 1 does not work at power-on"
        );
    }

    #[test]
    fn systimer_loads_and_stops() {
        let mut timer = Systimer::new();
        timer.write(Cycle(0), 0x0c, 1);
        timer.write(Cycle(0), 0x10, 5);
        timer.write(Cycle(10), 0x5c, 1);
        timer.write(Cycle(20), 0x00, 0);
        timer.write(Cycle(500), 0x04, UPDATE);
        assert_eq!(timer.read(0x40), Some(1));
        assert_eq!(timer.read(0x44), Some(5 + 8 - 4));
    }

    #[test]
    fn the_boot_watchdogs_run_until_disabled() {
        let mut dogs = Watchdogs::new();
        assert_eq!(dogs.deadline(), Some((Cycle(26_000_000), Watchdog::Timg0)));
        dogs.write_timg(Cycle(5), 0, 0x64, WDT_KEY);
        dogs.write_timg(Cycle(5), 0, 0x48, 0);
        let rtc = (400_000u64 * 40_000_000).div_ceil(136_000);
        assert_eq!(dogs.deadline(), Some((Cycle(rtc), Watchdog::Rtc)));
        dogs.write_rtc(Cycle(6), 0x98, 0);
        assert_eq!(dogs.deadline(), None);
    }

    #[test]
    fn the_rtc_feed_is_bit_31() {
        let mut dogs = Watchdogs::new();
        dogs.write_timg(Cycle(0), 0, 0x64, WDT_KEY);
        dogs.write_timg(Cycle(0), 0, 0x48, 0);
        let hold = (400_000u64 * 40_000_000).div_ceil(136_000);
        dogs.write_rtc(Cycle(100), 0xac, 1);
        assert_eq!(dogs.deadline(), Some((Cycle(hold), Watchdog::Rtc)));
        dogs.write_rtc(Cycle(100), 0xac, 1 << 31);
        assert_eq!(dogs.deadline(), Some((Cycle(100 + hold), Watchdog::Rtc)));
    }

    #[test]
    fn the_super_watchdog_starts_unlocked_with_its_width() {
        let mut dogs = Watchdogs::new();
        assert_eq!(dogs.read_rtc(0xb4), Some(300 << 18));
        assert_eq!(dogs.read_rtc(0xb8), Some(SWD_KEY));
        dogs.write_rtc(Cycle(0), 0xb4, 1 << 30);
        assert_eq!(
            dogs.read_rtc(0xb4),
            Some(1 << 30),
            "a write at reset sticks"
        );
        dogs.write_rtc(Cycle(0), 0xb8, 0);
        dogs.write_rtc(Cycle(0), 0xb4, 0);
        assert_eq!(dogs.read_rtc(0xb4), Some(1 << 30), "locked");
    }

    #[test]
    fn a_locked_watchdog_drops_writes_and_a_feed_restarts_it() {
        let mut dogs = Watchdogs::new();
        dogs.write_timg(Cycle(5), 0, 0x64, 0);
        dogs.write_timg(Cycle(5), 0, 0x48, 0);
        dogs.write_timg(Cycle(5), 0, 0x60, 1);
        assert_eq!(dogs.deadline(), Some((Cycle(26_000_000), Watchdog::Timg0)));
        dogs.write_timg(Cycle(5), 0, 0x64, WDT_KEY);
        dogs.write_timg(Cycle(100), 0, 0x60, 1);
        assert_eq!(dogs.deadline(), Some((Cycle(26_000_100), Watchdog::Timg0)));
    }

    #[test]
    fn a_hold_written_below_the_count_runs_out_at_the_write() {
        let mut dogs = Watchdogs::new();
        dogs.write_timg(Cycle(500), 0, 0x50, 10);
        assert_eq!(dogs.deadline(), Some((Cycle(500), Watchdog::Timg0)));
    }
}
