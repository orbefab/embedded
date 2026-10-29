//! SYSTIMER with its three alarms, and the watchdogs as timers that firmware feeds or disables.
//!
//! SYSTIMER counts at 16 MHz, the crystal times 2 over 5, whatever clock the CPU runs on. Each
//! of its two units counts while its `WORK_EN` bit is set. Each alarm compares one unit with
//! its target while its `WORK_EN` bit is set. A load through `COMPn_LOAD` takes the target, or
//! in period mode the unit's count plus the period. A target alarm fires once its unit reaches
//! the target, and a period alarm fires each period after the load. A firing alarm sets its
//! `INT_RAW` bit, and its interrupt line is that bit and its `INT_ENA` bit.
//!
//! The watchdogs: the RTC watchdog, and the main watchdog of each timer group. Each runs while
//! its `EN` bit is set and its first stage acts, or while its flash boot bit is set. That bit is
//! set at power-on on the RTC watchdog and the timer group 0 watchdog, as the boot ROM leaves
//! them, so firmware that does not disable or feed them meets the first stage. A watchdog that
//! runs out its first stage ends the run, whatever that stage's action. Every write but to the
//! write-protect register needs the key there. Writes take effect at once, and
//! `TIMG_WDT_CONF_UPDATE_EN` is not needed. A hold written below the count runs out at the write.
//! A timer group's watchdog counts the APB clock through its prescaler, and keeps its count
//! when the APB clock changes. The RTC watchdog counts the 136 kHz RC slow clock. The super
//! watchdog's two registers are kept, unlocked at power-on, and it never runs out.

use super::clock::{TICK_HZ, Tick};

/// Ticks per SYSTIMER count.
const SYSTIMER_TICKS: u64 = TICK_HZ / 16_000_000;

/// The RC slow clock's nominal rate.
const SLOW_HZ: u64 = 136_000;

/// The RTC watchdog's first stage counts `WDTCONFIG1` times `2 << WDT_DELAY_SEL`, with that
/// eFuse 0. IDF v5.5.5 writes stage 0 shifted down by that much, in
/// `components/hal/esp32s3/include/hal/rwdt_ll.h:109-124` and
/// `components/esp_hw_support/rtc_wdt.c:93-95`.
const RTC_STAGE0_SCALE: u64 = 2;

const SYSTIMER_BITS: u64 = (1 << 52) - 1;
const UNIT_WORK_EN: [u32; 2] = [1 << 30, 1 << 29];
const TARGET_WORK_EN: [u32; 3] = [1 << 24, 1 << 23, 1 << 22];
const VALUE_VALID: u32 = 1 << 29;
const UPDATE: u32 = 1 << 30;
const PERIOD: u32 = (1 << 26) - 1;
const PERIOD_MODE: u32 = 1 << 30;
const UNIT_SEL: u32 = 1 << 31;

/// The SYSTIMER count at `tick`, from power-on.
fn counts(tick: Tick) -> u64 {
    tick.0 / SYSTIMER_TICKS
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Systimer {
    conf: u32,
    units: [Unit; 2],
    alarms: [Alarm; 3],
    int_ena: u32,
    int_raw: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Unit {
    /// The count at `since`, from which it counts on while its unit works.
    base: u64,
    since: Tick,
    latched: u64,
    valid: bool,
    load: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Alarm {
    target: u64,
    conf: u32,
    /// The count at which it next fires, from its last load.
    due: Option<u64>,
}

impl Systimer {
    pub(crate) fn new() -> Systimer {
        Systimer {
            conf: 0x4600_0000,
            units: [Unit::default(), Unit::default()],
            alarms: Default::default(),
            int_ena: 0,
            int_raw: 0,
        }
    }

    fn value(&self, unit: usize, now: Tick) -> u64 {
        let u = &self.units[unit];
        if self.conf & UNIT_WORK_EN[unit] == 0 {
            return u.base;
        }
        (u.base + counts(now) - counts(u.since)) & SYSTIMER_BITS
    }

    /// The first tick at which `unit` reaches `count`, if it counts and has not reached it at
    /// `now`.
    fn reaches(&self, unit: usize, count: u64, now: Tick) -> Option<Tick> {
        if self.conf & UNIT_WORK_EN[unit] == 0 || self.value(unit, now) >= count {
            return None;
        }
        let u = &self.units[unit];
        Some(Tick((count - u.base + counts(u.since)) * SYSTIMER_TICKS))
    }

    fn unit_of(&self, alarm: usize) -> usize {
        usize::from(self.alarms[alarm].conf & UNIT_SEL != 0)
    }

    /// Fires every alarm due at or before `now`.
    pub(crate) fn settle(&mut self, now: Tick) {
        for (k, work_en) in TARGET_WORK_EN.into_iter().enumerate() {
            let unit = self.unit_of(k);
            if self.conf & work_en == 0 {
                continue;
            }
            let Some(due) = self.alarms[k].due else {
                continue;
            };
            if self.conf & UNIT_WORK_EN[unit] == 0 || self.value(unit, now) < due {
                continue;
            }
            self.int_raw |= 1 << k;
            let count = self.value(unit, now);
            let alarm = &mut self.alarms[k];
            let period = u64::from(alarm.conf & PERIOD);
            alarm.due = if alarm.conf & PERIOD_MODE != 0 && period > 0 {
                Some(due + ((count - due) / period + 1) * period)
            } else {
                None
            };
        }
    }

    /// The tick of the next alarm after `now`, which [`Systimer::settle`] has reached.
    pub(crate) fn next_event(&self, now: Tick) -> Option<Tick> {
        (0..3)
            .filter(|&k| self.conf & TARGET_WORK_EN[k] != 0)
            .filter_map(|k| self.reaches(self.unit_of(k), self.alarms[k].due?, now))
            .min()
    }

    /// Bit `k` is alarm `k`'s interrupt line.
    pub(crate) fn lines(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    pub(crate) fn read(&mut self, now: Tick, offset: u32) -> Option<u32> {
        self.settle(now);
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
            0x0c | 0x14 => (self.units[pair(offset, 0x0c, 8)].load >> 32) as u32,
            0x10 | 0x18 => self.units[pair(offset, 0x10, 8)].load as u32,
            0x1c | 0x24 | 0x2c => (self.alarms[((offset - 0x1c) / 8) as usize].target >> 32) as u32,
            0x20 | 0x28 | 0x30 => self.alarms[((offset - 0x20) / 8) as usize].target as u32,
            0x34..=0x3c => self.alarms[((offset - 0x34) / 4) as usize].conf,
            0x40 | 0x48 => (self.units[pair(offset, 0x40, 8)].latched >> 32) as u32,
            0x44 | 0x4c => self.units[pair(offset, 0x44, 8)].latched as u32,
            0x50..=0x60 => 0,
            0x64 => self.int_ena,
            0x68 => self.int_raw,
            0x6c => 0,
            0x70 => self.lines(),
            0x74 | 0x7c | 0x84 => {
                self.alarms[((offset - 0x74) / 8) as usize].due.unwrap_or(0) as u32
            }
            0x78 | 0x80 | 0x88 => {
                (self.alarms[((offset - 0x78) / 8) as usize].due.unwrap_or(0) >> 32) as u32
            }
            _ => return None,
        })
    }

    pub(crate) fn write(&mut self, now: Tick, offset: u32, value: u32) -> bool {
        self.settle(now);
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
                let load = &mut self.units[pair(offset, 0x0c, 8)].load;
                *load = (*load & 0xffff_ffff) | (u64::from(value & 0xf_ffff) << 32);
            }
            0x10 | 0x18 => {
                let load = &mut self.units[pair(offset, 0x10, 8)].load;
                *load = (*load & !0xffff_ffff) | u64::from(value);
            }
            0x1c | 0x24 | 0x2c => {
                let target = &mut self.alarms[((offset - 0x1c) / 8) as usize].target;
                *target = (*target & 0xffff_ffff) | (u64::from(value & 0xf_ffff) << 32);
            }
            0x20 | 0x28 | 0x30 => {
                let target = &mut self.alarms[((offset - 0x20) / 8) as usize].target;
                *target = (*target & !0xffff_ffff) | u64::from(value);
            }
            0x34..=0x3c => self.alarms[((offset - 0x34) / 4) as usize].conf = value,
            0x50..=0x58 => {
                let k = ((offset - 0x50) / 4) as usize;
                if value & 1 != 0 {
                    let unit = self.unit_of(k);
                    let alarm = &self.alarms[k];
                    let due = if alarm.conf & PERIOD_MODE != 0 {
                        self.value(unit, now) + u64::from(alarm.conf & PERIOD)
                    } else {
                        alarm.target
                    };
                    self.alarms[k].due = Some(due);
                }
            }
            0x5c | 0x60 => {
                let u = &mut self.units[pair(offset, 0x5c, 4)];
                if value & 1 != 0 {
                    (u.base, u.since) = (u.load, now);
                }
            }
            0x64 => self.int_ena = value & 0x7,
            0x6c => self.int_raw &= !value,
            _ => return false,
        }
        self.settle(now);
        true
    }

    /// Keeps the register at `offset` as the boot left it. A unit's value registers, the
    /// count its last update latched, also set its count at `now`, since the count itself is
    /// not readable.
    pub(crate) fn restore(&mut self, now: Tick, offset: u32, value: u32) {
        match offset {
            0x40 | 0x48 | 0x44 | 0x4c => {
                let u = &mut self.units[pair(offset & !4, 0x40, 8)];
                let (shift, mask) = if offset & 4 == 0 {
                    (32, 0xf_ffff)
                } else {
                    (0, u32::MAX)
                };
                let field = u64::from(mask) << shift;
                u.latched = (u.latched & !field) | (u64::from(value & mask) << shift);
                (u.base, u.since) = (u.latched, now);
            }
            0x00 | 0x0c..=0x3c | 0x64 => {
                self.write(now, offset, value);
            }
            0x68 => self.int_raw = value & 0x7,
            _ => {}
        }
    }
}

/// Which of a register pair at `first` and `first + step` the register at `offset` is.
fn pair(offset: u32, first: u32, step: u32) -> usize {
    ((offset - first) / step) as usize
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
    /// Ticks per APB cycle.
    apb: u64,
}

/// One watchdog: its config words from `CONFIG0`, its write-protect word, and its count, which
/// is `done` counts at tick `since` and counts on from there.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Timer {
    config: [u32; 6],
    wprotect: u32,
    since: Tick,
    done: u64,
    /// The last write's tick. A hold written below the count runs out there.
    written: Tick,
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
    /// Whether it counts the RTC slow clock, else the APB clock through its prescaler.
    slow: bool,
}

impl Timer {
    fn runs(&self) -> bool {
        let c = self.config[0];
        let l = self.layout;
        let stage = (c >> l.stg0_shift) & l.stg0_mask;
        (c & l.en != 0 && stage != 0) || (l.flashboot_runs && c & l.flashboot != 0)
    }

    /// Ticks per count, as a numerator over a denominator.
    fn rate(&self, apb: u64) -> (u64, u64) {
        if self.layout.slow {
            (TICK_HZ, SLOW_HZ)
        } else {
            (apb * u64::from((self.config[1] >> 16).max(1)), 1)
        }
    }

    /// The counts of the first stage.
    fn hold(&self) -> u64 {
        if self.layout.slow {
            u64::from(self.config[1]) * RTC_STAGE0_SCALE
        } else {
            u64::from(self.config[2])
        }
    }

    /// The tick its first stage runs out, while it runs.
    fn deadline(&self, apb: u64) -> Option<Tick> {
        if !self.runs() {
            return None;
        }
        let (num, den) = self.rate(apb);
        let left = self.hold().saturating_sub(self.done);
        let ticks = (u128::from(left) * u128::from(num)).div_ceil(u128::from(den));
        let ticks = u64::try_from(ticks).expect("a deadline inside u64");
        let ticks = if self.done == 0 { ticks.max(1) } else { ticks };
        Some(Tick((self.since.0 + ticks).max(self.written.0)))
    }

    /// Takes the whole counts up to `now` at the APB rate `apb`, so it counts on from there at
    /// another rate. Only a timer group's watchdog counts the APB clock.
    fn settle(&mut self, now: Tick, apb: u64) {
        let (per_count, _) = self.rate(apb);
        if !self.runs() {
            self.since = now;
            return;
        }
        let counts = (now.0 - self.since.0) / per_count;
        self.done += counts;
        self.since.0 += counts * per_count;
    }

    fn restart(&mut self, now: Tick) {
        self.since = now;
        self.done = 0;
    }

    fn write(&mut self, now: Tick, index: Option<usize>, value: u32, feed: bool) {
        if self.wprotect != WDT_KEY {
            return;
        }
        let ran = self.runs();
        self.written = now;
        if let Some(index) = index {
            self.config[index] = value;
        }
        if feed || (!ran && self.runs()) {
            self.restart(now);
        }
    }
}

impl Watchdogs {
    pub(crate) fn new(apb: u64) -> Watchdogs {
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
            since: Tick(0),
            done: 0,
            written: Tick(0),
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
                since: Tick(0),
                done: 0,
                written: Tick(0),
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
            apb,
        }
    }

    /// The first watchdog to run out, and when.
    pub(crate) fn deadline(&self) -> Option<(Tick, Watchdog)> {
        [
            (self.rtc.deadline(self.apb), Watchdog::Rtc),
            (self.timg[0].deadline(self.apb), Watchdog::Timg0),
            (self.timg[1].deadline(self.apb), Watchdog::Timg1),
        ]
        .into_iter()
        .filter_map(|(at, which)| Some((at?, which)))
        .min_by_key(|&(at, _)| at)
    }

    /// The APB clock runs at `apb` ticks per cycle from `now` on.
    pub(crate) fn reclock(&mut self, now: Tick, apb: u64) {
        for timer in &mut self.timg {
            timer.settle(now, self.apb);
        }
        self.apb = apb;
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

    pub(crate) fn write_timg(&mut self, now: Tick, group: usize, offset: u32, value: u32) -> bool {
        let t = &mut self.timg[group];
        match offset {
            0x48..=0x5c => t.write(now, Some(((offset - 0x48) / 4) as usize), value, false),
            0x60 => t.write(now, None, value, true),
            0x64 => t.wprotect = value,
            _ => return false,
        }
        true
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

    pub(crate) fn write_rtc(&mut self, now: Tick, offset: u32, value: u32) -> bool {
        match offset {
            0x98..=0xa8 => {
                let index = ((offset - 0x98) / 4) as usize;
                self.rtc.write(now, Some(index), value, false);
            }
            0xac => self.rtc.write(now, None, value, value & (1 << 31) != 0),
            0xb0 => self.rtc.wprotect = value,
            0xb4 => {
                if self.swd[1] == SWD_KEY {
                    self.swd[0] = value;
                }
            }
            0xb8 => self.swd[1] = value,
            _ => return false,
        }
        true
    }

    /// Sets each kept register to `value` as the boot left it. A config word restarts the
    /// count at `now`, and a feed does nothing.
    pub(crate) fn restore(&mut self, now: Tick, register: Register, value: u32) {
        // A timer group has six config words, the feed and the protect word. The RTC watchdog
        // has five config words, so its feed and protect word come one word earlier.
        let (timer, index, configs) = match register {
            Register::Timg(group, offset) => (&mut self.timg[group], offset.checked_sub(0x48), 6),
            Register::Rtc(offset) => (&mut self.rtc, offset.checked_sub(0x98), 5),
        };
        match index.map(|o| (o / 4) as usize) {
            Some(index) if index < configs => {
                timer.config[index] = value;
                timer.restart(now);
                timer.written = now;
            }
            Some(index) if index == configs + 1 => timer.wprotect = value,
            _ => {}
        }
    }
}

/// A watchdog register by block and offset, for [`Watchdogs::restore`].
#[derive(Clone, Copy, Debug)]
pub(crate) enum Register {
    Timg(usize, u32),
    Rtc(u32),
}

/// `RTC_CNTL_SWD_WKEY_VALUE`, which is also the protect word's reset value, so the super
/// watchdog starts unlocked. IDF v5.5.5,
/// `components/soc/esp32s3/register/soc/rtc_cntl_reg.h:2168`.
const SWD_KEY: u32 = 0x8f1d_312a;
/// `RTC_CNTL_SWD_SIGNAL_WIDTH` 300 at bit 18, the rest 0, from the same header's line 2142.
const SWD_CONF_RESET: u32 = 300 << 18;

#[cfg(test)]
mod tests {
    use super::super::clock::XTAL;
    use super::*;

    /// The tick of crystal cycle `cycle`.
    fn x(cycle: u64) -> Tick {
        Tick(cycle * XTAL)
    }

    #[test]
    fn systimer_counts_two_every_five_crystal_cycles_and_latches_on_update() {
        let mut timer = Systimer::new();
        timer.write(x(1000), 0x04, UPDATE);
        assert_eq!(timer.read(x(1000), 0x04), Some(VALUE_VALID));
        assert_eq!(timer.read(x(1000), 0x44), Some(400));
        timer.write(x(1003), 0x04, UPDATE);
        assert_eq!(timer.read(x(1003), 0x44), Some(401));
        timer.write(x(1003), 0x08, UPDATE);
        assert_eq!(
            timer.read(x(1003), 0x4c),
            Some(0),
            "unit 1 does not work at power-on"
        );
    }

    #[test]
    fn systimer_loads_and_stops() {
        let mut timer = Systimer::new();
        timer.write(x(0), 0x0c, 1);
        timer.write(x(0), 0x10, 5);
        timer.write(x(10), 0x5c, 1);
        timer.write(x(20), 0x00, 0);
        timer.write(x(500), 0x04, UPDATE);
        assert_eq!(timer.read(x(500), 0x40), Some(1));
        assert_eq!(timer.read(x(500), 0x44), Some(5 + 8 - 4));
    }

    #[test]
    fn a_period_alarm_fires_each_period_after_its_load() {
        let mut timer = Systimer::new();
        timer.write(x(0), 0x34, PERIOD_MODE | 100);
        timer.write(x(0), 0x50, 1);
        timer.write(x(0), 0x64, 1);
        timer.write(x(0), 0x00, 0x4600_0000 | TARGET_WORK_EN[0]);
        let first = timer.next_event(x(0)).expect("armed");
        assert_eq!(first, Tick(100 * SYSTIMER_TICKS));
        timer.settle(Tick(first.0 - 1));
        assert_eq!(timer.lines(), 0);
        timer.settle(first);
        assert_eq!(timer.lines(), 1);
        timer.write(first, 0x6c, 1);
        assert_eq!(timer.lines(), 0);
        assert_eq!(timer.next_event(first), Some(Tick(200 * SYSTIMER_TICKS)));
    }

    #[test]
    fn a_target_alarm_fires_once() {
        let mut timer = Systimer::new();
        timer.write(x(0), 0x20, 40);
        timer.write(x(0), 0x50, 1);
        timer.write(x(0), 0x00, 0x4600_0000 | TARGET_WORK_EN[0]);
        timer.settle(x(100));
        assert_eq!(timer.read(x(100), 0x68), Some(1));
        assert_eq!(timer.lines(), 0, "not enabled");
        assert_eq!(timer.next_event(x(100)), None);
        timer.write(x(100), 0x6c, 1);
        timer.settle(x(200));
        assert_eq!(timer.read(x(200), 0x68), Some(0), "fired once");
    }

    #[test]
    fn the_boot_watchdogs_run_until_disabled() {
        let mut dogs = Watchdogs::new(XTAL);
        assert_eq!(dogs.deadline(), Some((x(26_000_000), Watchdog::Timg0)));
        dogs.write_timg(x(5), 0, 0x64, WDT_KEY);
        dogs.write_timg(x(5), 0, 0x48, 0);
        let rtc = Tick((400_000u64 * TICK_HZ).div_ceil(SLOW_HZ));
        assert_eq!(dogs.deadline(), Some((rtc, Watchdog::Rtc)));
        dogs.write_rtc(x(6), 0x98, 0);
        assert_eq!(dogs.deadline(), None);
    }

    #[test]
    fn the_rtc_feed_is_bit_31() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(0), 0, 0x64, WDT_KEY);
        dogs.write_timg(x(0), 0, 0x48, 0);
        let hold = (400_000u64 * TICK_HZ).div_ceil(SLOW_HZ);
        dogs.write_rtc(x(100), 0xac, 1);
        assert_eq!(dogs.deadline(), Some((Tick(hold), Watchdog::Rtc)));
        dogs.write_rtc(x(100), 0xac, 1 << 31);
        assert_eq!(
            dogs.deadline(),
            Some((Tick(x(100).0 + hold), Watchdog::Rtc))
        );
    }

    #[test]
    fn a_timer_group_watchdog_keeps_its_count_across_an_apb_change() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_rtc(x(0), 0x98, 0);
        dogs.reclock(x(13_000_000), XTAL / 2);
        assert_eq!(
            dogs.deadline(),
            Some((
                Tick(x(13_000_000).0 + 13_000_000 * XTAL / 2),
                Watchdog::Timg0
            )),
            "half the count left, at twice the rate"
        );
    }

    #[test]
    fn the_super_watchdog_starts_unlocked_with_its_width() {
        let mut dogs = Watchdogs::new(XTAL);
        assert_eq!(dogs.read_rtc(0xb4), Some(300 << 18));
        assert_eq!(dogs.read_rtc(0xb8), Some(SWD_KEY));
        dogs.write_rtc(x(0), 0xb4, 1 << 30);
        assert_eq!(
            dogs.read_rtc(0xb4),
            Some(1 << 30),
            "a write at reset sticks"
        );
        dogs.write_rtc(x(0), 0xb8, 0);
        dogs.write_rtc(x(0), 0xb4, 0);
        assert_eq!(dogs.read_rtc(0xb4), Some(1 << 30), "locked");
    }

    #[test]
    fn a_locked_watchdog_drops_writes_and_a_feed_restarts_it() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(5), 0, 0x64, 0);
        dogs.write_timg(x(5), 0, 0x48, 0);
        dogs.write_timg(x(5), 0, 0x60, 1);
        assert_eq!(dogs.deadline(), Some((x(26_000_000), Watchdog::Timg0)));
        dogs.write_timg(x(5), 0, 0x64, WDT_KEY);
        dogs.write_timg(x(100), 0, 0x60, 1);
        assert_eq!(dogs.deadline(), Some((x(26_000_100), Watchdog::Timg0)));
    }

    #[test]
    fn a_restored_protect_word_locks_the_rtc_and_timer_group_watchdogs() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.restore(x(0), Register::Rtc(0xac), 1 << 31);
        dogs.restore(x(0), Register::Rtc(0xb0), 0);
        dogs.restore(x(0), Register::Timg(0, 0x64), 0);
        assert_eq!(dogs.read_rtc(0xb0), Some(0));
        let before = dogs.deadline();
        dogs.write_rtc(x(10), 0x98, 0);
        dogs.write_timg(x(10), 0, 0x48, 0);
        assert_eq!(dogs.deadline(), before, "locked, so the writes drop");
        assert_eq!(dogs.rtc.config[0], 0x0001_3214);
    }

    #[test]
    fn a_hold_written_below_the_count_runs_out_at_the_write() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(500), 0, 0x50, 10);
        assert_eq!(dogs.deadline(), Some((x(500), Watchdog::Timg0)));
    }
}
