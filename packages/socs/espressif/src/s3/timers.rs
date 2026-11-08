//! SYSTIMER with its three alarms, and the watchdogs as timers that firmware feeds or disables.
//!
//! SYSTIMER counts at 16 MHz, the crystal times 2 over 5, whatever clock the CPU runs on. Each
//! of its two units counts while its `WORK_EN` bit is set. Each alarm compares one unit with
//! its target while its `WORK_EN` bit is set. A load through `COMPn_LOAD` takes the target, or
//! in period mode the unit's count plus the period. A write of `TARGETn_CONF` that sets period
//! mode on an alarm with no fire pending arms it at the unit's count plus the period, and one
//! that finds a fire pending keeps that count. A target alarm fires once its unit reaches the
//! target, and a period alarm fires each period after the load. A firing alarm sets its
//! `INT_RAW` bit, and its interrupt line is that bit and its `INT_ENA` bit.
//!
//! The watchdogs: the RTC watchdog, and the main watchdog of each timer group. Each runs while
//! its `EN` bit is set and a stage acts, or while its flash boot bit is set. That bit is set at
//! power-on on the RTC watchdog and the timer group 0 watchdog, as the boot ROM leaves them, and
//! in flash boot mode stage 0 resets the system, or on the RTC watchdog the RTC domain. A running
//! watchdog counts each stage's hold in turn from stage 0, and after stage 3 stage 0 again. A
//! stage that runs out does its action: nothing, its interrupt, or a reset of the CPU, the system
//! or the RTC domain, which ends the run. A timer group's watchdog interrupt sets the watchdog
//! bit of the group's `INT_RAW` until `INT_CLR` clears it, and its line is that bit and its
//! `INT_ENA` bit. A feed, or a write that starts it, counts stage 0 again. A write that leaves a
//! running watchdog a stage the model does not run is refused: an interrupt of the RTC watchdog,
//! a CPU reset without `PROCPU_RESET_EN`, or a code that names no action. Every write but to the
//! write-protect register needs the key there. Writes take effect at once, and
//! `TIMG_WDT_CONF_UPDATE_EN` is not needed. A hold written below the count runs out at the write.
//! A timer group's watchdog counts the APB clock through its prescaler, and keeps its count
//! when the APB clock changes. The RTC watchdog counts the 136 kHz RC slow clock. The super
//! watchdog's two registers are kept, unlocked at power-on. A write that leaves it neither
//! feeding itself nor disabled is refused, so it never runs out.

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
    /// The count at which it next fires, from its last load or its period-mode write.
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
            0x34..=0x3c => {
                let k = ((offset - 0x34) / 4) as usize;
                let was = self.alarms[k].conf;
                self.alarms[k].conf = value;
                // A rising period mode with nothing pending arms at the count plus the period.
                // The second core's tick is that write: its target alarm has already fired.
                let sets_period = was & PERIOD_MODE == 0 && value & PERIOD_MODE != 0;
                if sets_period && self.alarms[k].due.is_none() {
                    let unit = self.unit_of(k);
                    let period = u64::from(value & PERIOD);
                    self.alarms[k].due = Some(self.value(unit, now) + period);
                }
            }
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

/// What a watchdog's stage does when it runs out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Off,
    Interrupt,
    Reset(Reset),
}

/// What a watchdog's reset resets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reset {
    Cpu,
    System,
    Rtc,
}

impl Reset {
    pub fn name(self) -> &'static str {
        match self {
            Reset::Cpu => "CPU",
            Reset::System => "system",
            Reset::Rtc => "RTC domain",
        }
    }
}

/// The next stage of a watchdog to run out: when, and what it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Expiry {
    pub(crate) at: Tick,
    pub(crate) watchdog: Watchdog,
    pub(crate) stage: u8,
    pub(crate) action: Action,
}

/// `RTC_CNTL_WDT_WKEY_VALUE` and `TIMG_WDT_WKEY`, one value.
const WDT_KEY: u32 = 0x50d8_3aa1;

/// The watchdog's bit in a timer group's `INT_ENA`, `INT_RAW`, `INT_ST` and `INT_CLR`.
const WDT_INT: u32 = 1 << 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Watchdogs {
    timg: [Timer; 2],
    rtc: Timer,
    /// Each timer group's `INT_ENA`.
    int_ena: [u32; 2],
    /// `RTC_CNTL_SWD_CONF_REG` and `RTC_CNTL_SWD_WPROTECT_REG`.
    swd: [u32; 2],
    /// Ticks per APB cycle.
    apb: u64,
}

/// One watchdog: its config words from `CONFIG0`, its write-protect word, and its count in its
/// stage, which is `done` counts at tick `since` and counts on from there.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Timer {
    config: [u32; 6],
    wprotect: u32,
    since: Tick,
    done: u64,
    /// The last write's tick. A hold written below the count runs out there.
    written: Tick,
    /// The stage that counts, 0 to 3.
    stage: usize,
    /// A stage that interrupts ran out, and `INT_CLR` has not cleared it since.
    raised: bool,
    /// Where `EN`, the stages' actions and the flash boot bit sit in `CONFIG0`.
    layout: Layout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    en: u32,
    /// Stage 0's action sits at this bit, and stage `n`'s `width * n` bits below it.
    stg0_shift: u32,
    width: u32,
    flashboot: u32,
    /// Whether the flash boot bit runs the timer. Only timer group 0's does.
    flashboot_runs: bool,
    /// `PROCPU_RESET_EN`, which lets a CPU reset reach the core.
    cpu_reset_en: u32,
    /// Whether it counts the RTC slow clock, else the APB clock through its prescaler.
    slow: bool,
}

impl Timer {
    fn flashboot(&self) -> bool {
        self.layout.flashboot_runs && self.config[0] & self.layout.flashboot != 0
    }

    /// The action of `stage`, or `None` for a code that names none. In flash boot mode stage 0
    /// resets the system, or the RTC domain on the RTC watchdog.
    fn action(&self, stage: usize) -> Option<Action> {
        let l = self.layout;
        if stage == 0 && self.flashboot() {
            return Some(Action::Reset(if l.slow {
                Reset::Rtc
            } else {
                Reset::System
            }));
        }
        let code =
            (self.config[0] >> (l.stg0_shift - l.width * stage as u32)) & ((1 << l.width) - 1);
        Some(match code {
            0 => Action::Off,
            1 => Action::Interrupt,
            2 => Action::Reset(Reset::Cpu),
            3 => Action::Reset(Reset::System),
            4 if l.slow => Action::Reset(Reset::Rtc),
            _ => return None,
        })
    }

    fn runs(&self) -> bool {
        self.flashboot()
            || (self.config[0] & self.layout.en != 0
                && (0..4).any(|stage| self.action(stage) != Some(Action::Off)))
    }

    /// The setting a running watchdog's stages need that the model does not run, in the stages
    /// up to the first that resets.
    fn refusal(&self) -> Option<&'static str> {
        if !self.runs() || self.flashboot() {
            return None;
        }
        for stage in 0..4 {
            match self.action(stage) {
                None => return Some("watchdog_stage"),
                Some(Action::Interrupt) if self.layout.slow => return Some("watchdog_interrupt"),
                Some(Action::Reset(Reset::Cpu))
                    if self.config[0] & self.layout.cpu_reset_en == 0 =>
                {
                    return Some("watchdog_cpu_reset");
                }
                Some(Action::Reset(_)) => return None,
                Some(_) => {}
            }
        }
        None
    }

    /// Ticks per count, as a numerator over a denominator.
    fn rate(&self, apb: u64) -> (u64, u64) {
        if self.layout.slow {
            (TICK_HZ, SLOW_HZ)
        } else {
            (apb * u64::from((self.config[1] >> 16).max(1)), 1)
        }
    }

    /// The counts of the stage that counts.
    fn hold(&self) -> u64 {
        match (self.layout.slow, self.stage) {
            (true, 0) => u64::from(self.config[1]) * RTC_STAGE0_SCALE,
            (true, stage) => u64::from(self.config[1 + stage]),
            (false, stage) => u64::from(self.config[2 + stage]),
        }
    }

    /// The tick its stage runs out, while it runs.
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
        self.stage = 0;
    }

    /// Its stage ran out at `at` and did not reset: it raises its interrupt if the stage
    /// interrupts, and the next stage counts from `at`, after stage 3 stage 0.
    fn run_out(&mut self, at: Tick) {
        self.raised |= self.action(self.stage) == Some(Action::Interrupt);
        self.stage = (self.stage + 1) % 4;
        self.since = at;
        self.done = 0;
    }

    fn write(
        &mut self,
        now: Tick,
        index: Option<usize>,
        value: u32,
        feed: bool,
    ) -> Option<&'static str> {
        if self.wprotect != WDT_KEY {
            return None;
        }
        let ran = self.runs();
        self.written = now;
        if let Some(index) = index {
            self.config[index] = value;
        }
        if feed || (!ran && self.runs()) {
            self.restart(now);
        }
        self.refusal()
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
            stage: 0,
            raised: false,
            layout: Layout {
                en: 1 << 31,
                stg0_shift: 29,
                width: 2,
                flashboot: 1 << 14,
                flashboot_runs,
                cpu_reset_en: 1 << 13,
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
                stage: 0,
                raised: false,
                layout: Layout {
                    en: 1 << 31,
                    stg0_shift: 28,
                    width: 3,
                    flashboot: 1 << 12,
                    flashboot_runs: true,
                    cpu_reset_en: 1 << 11,
                    slow: true,
                },
            },
            int_ena: [0; 2],
            swd: [SWD_CONF_RESET, SWD_KEY],
            apb,
        }
    }

    fn timer(&mut self, watchdog: Watchdog) -> &mut Timer {
        match watchdog {
            Watchdog::Rtc => &mut self.rtc,
            Watchdog::Timg0 => &mut self.timg[0],
            Watchdog::Timg1 => &mut self.timg[1],
        }
    }

    /// The first watchdog stage to run out, when, and what it does.
    pub(crate) fn deadline(&self) -> Option<Expiry> {
        [
            (&self.rtc, Watchdog::Rtc),
            (&self.timg[0], Watchdog::Timg0),
            (&self.timg[1], Watchdog::Timg1),
        ]
        .into_iter()
        .filter_map(|(timer, watchdog)| {
            Some(Expiry {
                at: timer.deadline(self.apb)?,
                watchdog,
                stage: timer.stage as u8,
                action: timer.action(timer.stage).unwrap_or(Action::Off),
            })
        })
        .min_by_key(|expiry| expiry.at)
    }

    /// The stage of `expiry`, which does not reset, ran out.
    pub(crate) fn run_out(&mut self, expiry: Expiry) {
        self.timer(expiry.watchdog).run_out(expiry.at);
    }

    /// Each timer group's watchdog interrupt line.
    pub(crate) fn lines(&self) -> [bool; 2] {
        [0, 1].map(|group| self.timg[group].raised && self.int_ena[group] & WDT_INT != 0)
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
        let raw = if t.raised { WDT_INT } else { 0 };
        match offset {
            0x48..=0x5c => Some(t.config[((offset - 0x48) / 4) as usize]),
            0x60 => Some(0),
            0x64 => Some(t.wprotect),
            0x70 => Some(self.int_ena[group]),
            0x74 => Some(raw),
            0x78 => Some(raw & self.int_ena[group]),
            0x7c => Some(0),
            _ => None,
        }
    }

    /// Writes a timer group's watchdog register, or names the setting the write leaves that the
    /// model does not run.
    pub(crate) fn write_timg(
        &mut self,
        now: Tick,
        group: usize,
        offset: u32,
        value: u32,
    ) -> Result<bool, &'static str> {
        let t = &mut self.timg[group];
        let refused = match offset {
            0x48..=0x5c => t.write(now, Some(((offset - 0x48) / 4) as usize), value, false),
            0x60 => t.write(now, None, value, true),
            0x64 => {
                t.wprotect = value;
                None
            }
            0x70 => {
                self.int_ena[group] = value & 0x7;
                None
            }
            0x74 | 0x78 => None,
            0x7c => {
                t.raised &= value & WDT_INT == 0;
                None
            }
            _ => return Ok(false),
        };
        refused.map_or(Ok(true), Err)
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

    /// Writes an RTC_CNTL register the watchdogs own, or names the setting the write leaves
    /// that the model does not run.
    pub(crate) fn write_rtc(
        &mut self,
        now: Tick,
        offset: u32,
        value: u32,
    ) -> Result<bool, &'static str> {
        let refused = match offset {
            0x98..=0xa8 => {
                let index = ((offset - 0x98) / 4) as usize;
                self.rtc.write(now, Some(index), value, false)
            }
            0xac => self.rtc.write(now, None, value, value & (1 << 31) != 0),
            0xb0 => {
                self.rtc.wprotect = value;
                None
            }
            0xb4 if self.swd[1] != SWD_KEY => None,
            0xb4 if value & (SWD_AUTO_FEED_EN | SWD_DISABLE) == 0 => Some("super_watchdog"),
            0xb4 => {
                self.swd[0] = value;
                None
            }
            0xb8 => {
                self.swd[1] = value;
                None
            }
            _ => return Ok(false),
        };
        refused.map_or(Ok(true), Err)
    }

    /// Sets each kept register to `value` as the boot left it. A config word restarts the
    /// count at `now`, and a feed does nothing.
    pub(crate) fn restore(&mut self, now: Tick, register: Register, value: u32) {
        match register {
            Register::Timg(group, 0x70) => self.int_ena[group] = value & 0x7,
            Register::Timg(group, 0x74) => self.timg[group].raised = value & WDT_INT != 0,
            Register::Rtc(0xb4) => self.swd[0] = value,
            Register::Rtc(0xb8) => self.swd[1] = value,
            _ => {}
        }
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
/// `RTC_CNTL_SWD_AUTO_FEED_EN` and `RTC_CNTL_SWD_DISABLE`, lines 2120 and 2126.
const SWD_AUTO_FEED_EN: u32 = 1 << 31;
const SWD_DISABLE: u32 = 1 << 30;

#[cfg(test)]
mod tests {
    use super::super::clock::XTAL;
    use super::*;

    /// The next watchdog to run out a stage, and when.
    fn due(dogs: &Watchdogs) -> Option<(Tick, Watchdog)> {
        dogs.deadline().map(|expiry| (expiry.at, expiry.watchdog))
    }

    /// The tick of crystal cycle `cycle`.
    fn x(cycle: u64) -> Tick {
        Tick(cycle * XTAL)
    }

    fn dues(timer: &Systimer) -> [Option<u64>; 2] {
        [timer.alarms[0].due, timer.alarms[1].due]
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

    /// The two alarms as ESP-IDF's `vSystimerSetup` stores them for two cores. P is 16,000
    /// counts. Unit 1 stays stopped through step 10, so those stores share `x(100)`.
    #[test]
    fn the_dual_core_tick_arms_alarm_1_at_the_period_mode_write() {
        const P: u64 = 16_000;
        let mut timer = Systimer::new();
        let at = x(100);

        timer.write(at, 0x14, 0);
        timer.write(at, 0x18, 0);
        timer.write(at, 0x60, 1);
        assert_eq!(dues(&timer), [None, None]);

        timer.write(at, 0x34, 0);
        timer.write(at, 0x38, 0);
        assert_eq!(dues(&timer), [None, None]);

        timer.write(at, 0x34, UNIT_SEL);
        assert_eq!(dues(&timer), [None, None]);

        let conf = timer.read(at, 0x00).unwrap() & !TARGET_WORK_EN[0];
        timer.write(at, 0x00, conf);
        assert_eq!(dues(&timer), [None, None]);

        timer.write(at, 0x34, UNIT_SEL | P as u32);
        assert_eq!(dues(&timer), [None, None]);

        timer.write(at, 0x50, 1);
        assert_eq!(dues(&timer), [Some(0), None]);

        let conf = timer.read(at, 0x00).unwrap() | TARGET_WORK_EN[0];
        timer.write(at, 0x00, conf);
        assert_eq!(dues(&timer), [Some(0), None]);
        assert_eq!(timer.lines(), 0, "unit 1 is stopped");

        timer.write(at, 0x34, UNIT_SEL | PERIOD_MODE | P as u32);
        assert_eq!(
            dues(&timer),
            [Some(0), None],
            "a pending alarm keeps its count"
        );

        let conf = timer.read(at, 0x00).unwrap() | (1 << 26);
        timer.write(at, 0x00, conf);
        assert_eq!(dues(&timer), [Some(0), None]);

        timer.write(at, 0x64, 1);
        assert_eq!(dues(&timer), [Some(0), None]);

        let on = x(105);
        let conf = timer.read(on, 0x00).unwrap() | UNIT_WORK_EN[1];
        timer.write(on, 0x00, conf);
        assert_eq!(dues(&timer), [Some(P), None]);
        assert_eq!(timer.read(on, 0x68), Some(1));
        assert_eq!(timer.lines(), 1);

        timer.write(x(110), 0x08, UPDATE);
        assert_eq!(timer.read(x(110), 0x4c), Some(2));
        let advance = x(115);
        timer.write(advance, 0x14, 0);
        timer.write(advance, 0x18, 8_002);
        timer.write(advance, 0x60, 1);
        assert_eq!(dues(&timer), [Some(P), None]);

        let alarm1 = x(120);
        timer.write(alarm1, 0x38, UNIT_SEL);
        let conf = timer.read(alarm1, 0x00).unwrap() & !TARGET_WORK_EN[1];
        timer.write(alarm1, 0x00, conf);
        timer.write(alarm1, 0x38, UNIT_SEL | P as u32);
        timer.write(alarm1, 0x54, 1);
        let conf = timer.read(alarm1, 0x00).unwrap() | TARGET_WORK_EN[1];
        timer.write(alarm1, 0x00, conf);
        assert_eq!(dues(&timer), [Some(P), None]);
        assert_eq!(timer.read(alarm1, 0x68).unwrap() & 2, 2);

        let mode = x(125);
        timer.write(mode, 0x38, UNIT_SEL | PERIOD_MODE | P as u32);
        assert_eq!(dues(&timer), [Some(P), Some(24_006)]);

        let conf = timer.read(mode, 0x00).unwrap() | (1 << 25);
        timer.write(mode, 0x00, conf);
        assert_eq!(dues(&timer), [Some(P), Some(24_006)]);

        let mut now = mode;
        for count in [16_000, 24_006, 32_000, 40_006, 48_000] {
            timer.write(now, 0x6c, 0x7);
            let tick = timer.next_event(now).expect("a tick");
            timer.write(tick, 0x08, UPDATE);
            assert_eq!(timer.read(tick, 0x4c), Some(count), "latched {count}");
            now = tick;
        }
    }

    /// A period-mode write that leaves the bit set does not move a pending alarm.
    #[test]
    fn a_period_mode_write_that_keeps_the_bit_leaves_a_pending_alarm() {
        let mut timer = Systimer::new();
        timer.write(x(0), 0x34, PERIOD_MODE | 16_000);
        assert_eq!(timer.alarms[0].due, Some(16_000));
        timer.write(x(250), 0x34, PERIOD_MODE | 16_000);
        assert_eq!(timer.alarms[0].due, Some(16_000));
        assert_eq!(timer.value(0, x(250)), 100);
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
        assert_eq!(due(&dogs), Some((x(26_000_000), Watchdog::Timg0)));
        dogs.write_timg(x(5), 0, 0x64, WDT_KEY).unwrap();
        dogs.write_timg(x(5), 0, 0x48, 0).unwrap();
        let rtc = Tick((400_000u64 * TICK_HZ).div_ceil(SLOW_HZ));
        assert_eq!(due(&dogs), Some((rtc, Watchdog::Rtc)));
        dogs.write_rtc(x(6), 0x98, 0).unwrap();
        assert_eq!(due(&dogs), None);
    }

    #[test]
    fn the_rtc_feed_is_bit_31() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(0), 0, 0x64, WDT_KEY).unwrap();
        dogs.write_timg(x(0), 0, 0x48, 0).unwrap();
        let hold = (400_000u64 * TICK_HZ).div_ceil(SLOW_HZ);
        dogs.write_rtc(x(100), 0xac, 1).unwrap();
        assert_eq!(due(&dogs), Some((Tick(hold), Watchdog::Rtc)));
        dogs.write_rtc(x(100), 0xac, 1 << 31).unwrap();
        assert_eq!(due(&dogs), Some((Tick(x(100).0 + hold), Watchdog::Rtc)));
    }

    #[test]
    fn a_timer_group_watchdog_keeps_its_count_across_an_apb_change() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_rtc(x(0), 0x98, 0).unwrap();
        dogs.reclock(x(13_000_000), XTAL / 2);
        assert_eq!(
            due(&dogs),
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
        dogs.write_rtc(x(0), 0xb4, 1 << 30).unwrap();
        assert_eq!(
            dogs.read_rtc(0xb4),
            Some(1 << 30),
            "a write at reset sticks"
        );
        dogs.write_rtc(x(0), 0xb8, 0).unwrap();
        dogs.write_rtc(x(0), 0xb4, 0).unwrap();
        assert_eq!(dogs.read_rtc(0xb4), Some(1 << 30), "locked");
    }

    #[test]
    fn a_locked_watchdog_drops_writes_and_a_feed_restarts_it() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(5), 0, 0x64, 0).unwrap();
        dogs.write_timg(x(5), 0, 0x48, 0).unwrap();
        dogs.write_timg(x(5), 0, 0x60, 1).unwrap();
        assert_eq!(due(&dogs), Some((x(26_000_000), Watchdog::Timg0)));
        dogs.write_timg(x(5), 0, 0x64, WDT_KEY).unwrap();
        dogs.write_timg(x(100), 0, 0x60, 1).unwrap();
        assert_eq!(due(&dogs), Some((x(26_000_100), Watchdog::Timg0)));
    }

    #[test]
    fn a_restored_protect_word_locks_the_rtc_and_timer_group_watchdogs() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.restore(x(0), Register::Rtc(0xac), 1 << 31);
        dogs.restore(x(0), Register::Rtc(0xb0), 0);
        dogs.restore(x(0), Register::Timg(0, 0x64), 0);
        assert_eq!(dogs.read_rtc(0xb0), Some(0));
        let before = due(&dogs);
        dogs.write_rtc(x(10), 0x98, 0).unwrap();
        dogs.write_timg(x(10), 0, 0x48, 0).unwrap();
        assert_eq!(due(&dogs), before, "locked, so the writes drop");
        assert_eq!(dogs.rtc.config[0], 0x0001_3214);
    }

    #[test]
    fn a_hold_written_below_the_count_runs_out_at_the_write() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(500), 0, 0x50, 10).unwrap();
        assert_eq!(due(&dogs), Some((x(500), Watchdog::Timg0)));
    }

    const EN: u32 = 1 << 31;

    /// Timer group 0's watchdog, out of flash boot mode, with `config0`, prescaler 1 and stage
    /// holds 10, 20, 30 and 40, started at crystal cycle 0, and the others disabled.
    fn group0(config0: u32) -> Watchdogs {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_rtc(x(0), 0x98, 0).unwrap();
        for (k, hold) in [10, 20, 30, 40].into_iter().enumerate() {
            dogs.write_timg(x(0), 0, 0x50 + 4 * k as u32, hold).unwrap();
        }
        dogs.write_timg(x(0), 0, 0x48, 0).unwrap();
        dogs.write_timg(x(0), 0, 0x48, config0).unwrap();
        dogs
    }

    /// The stage, action and tick of each stage run out in turn, until one resets or `n` ran.
    fn stages(dogs: &mut Watchdogs, n: usize) -> Vec<(u8, Action, u64)> {
        let mut seen = Vec::new();
        while let Some(expiry) = dogs.deadline() {
            seen.push((expiry.stage, expiry.action, expiry.at.0 / XTAL));
            if matches!(expiry.action, Action::Reset(_)) || seen.len() == n {
                break;
            }
            dogs.run_out(expiry);
        }
        seen
    }

    #[test]
    fn each_stage_counts_its_hold_in_turn_and_does_its_action() {
        let mut dogs = group0(EN | 1 << 29 | 3 << 27);
        assert_eq!(
            stages(&mut dogs, 4),
            [
                (0, Action::Interrupt, 10),
                (1, Action::Reset(Reset::System), 30)
            ]
        );
        assert_eq!(dogs.read_timg(0, 0x74), Some(WDT_INT));
        assert_eq!(dogs.lines(), [false, false], "not enabled");
        dogs.write_timg(x(10), 0, 0x70, WDT_INT).unwrap();
        assert_eq!(dogs.lines(), [true, false]);
        assert_eq!(dogs.read_timg(0, 0x78), Some(WDT_INT));
        dogs.write_timg(x(10), 0, 0x7c, WDT_INT).unwrap();
        assert_eq!(dogs.lines(), [false, false]);
        assert_eq!(dogs.read_timg(0, 0x74), Some(0));
    }

    #[test]
    fn stages_that_do_nothing_count_and_stage_3_leads_back_to_stage_0() {
        let mut dogs = group0(EN | 1 << 25);
        assert_eq!(
            stages(&mut dogs, 5),
            [
                (0, Action::Off, 10),
                (1, Action::Off, 30),
                (2, Action::Interrupt, 60),
                (3, Action::Off, 100),
                (0, Action::Off, 110),
            ]
        );
    }

    #[test]
    fn a_feed_counts_stage_0_again() {
        let mut dogs = group0(EN | 1 << 29 | 3 << 27);
        let first = dogs.deadline().unwrap();
        dogs.run_out(first);
        dogs.write_timg(x(15), 0, 0x60, 1).unwrap();
        assert_eq!(stages(&mut dogs, 1), [(0, Action::Interrupt, 25)]);
    }

    #[test]
    fn flash_boot_mode_resets_the_system_or_the_rtc_domain_at_stage_0() {
        let dogs = Watchdogs::new(XTAL);
        let expiry = dogs.deadline().unwrap();
        assert_eq!(
            (expiry.watchdog, expiry.stage, expiry.action),
            (Watchdog::Timg0, 0, Action::Reset(Reset::System))
        );
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(0), 0, 0x48, 0).unwrap();
        let expiry = dogs.deadline().unwrap();
        assert_eq!(
            (expiry.watchdog, expiry.action),
            (Watchdog::Rtc, Action::Reset(Reset::Rtc))
        );
    }

    #[test]
    fn the_rtc_watchdog_takes_its_later_holds_from_config2_on() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(0), 0, 0x48, 0).unwrap();
        dogs.write_rtc(x(0), 0x9c, 34).unwrap();
        dogs.write_rtc(x(0), 0xa0, 68).unwrap();
        dogs.write_rtc(x(0), 0x98, EN | 3 << 25 | 1 << 11).unwrap();
        let slow = |counts: u64| (counts * TICK_HZ).div_ceil(SLOW_HZ);
        let first = dogs.deadline().unwrap();
        assert_eq!((first.at.0, first.action), (slow(68), Action::Off));
        dogs.run_out(first);
        let second = dogs.deadline().unwrap();
        assert_eq!(
            (second.at.0, second.stage, second.action),
            (slow(68) + slow(68), 1, Action::Reset(Reset::System))
        );
    }

    #[test]
    fn a_stage_the_model_does_not_run_is_refused_while_it_runs() {
        let timg = |config0| {
            let mut dogs = Watchdogs::new(XTAL);
            dogs.write_timg(x(0), 1, 0x48, config0)
        };
        assert_eq!(timg(EN | 2 << 29), Err("watchdog_cpu_reset"));
        assert_eq!(timg(EN | 1 << 29 | 2 << 27), Err("watchdog_cpu_reset"));
        assert_eq!(timg(EN | 2 << 29 | 1 << 13), Ok(true));
        assert_eq!(timg(EN | 3 << 29 | 2 << 27), Ok(true), "after a reset");
        assert_eq!(timg(2 << 29), Ok(true), "not running");
        let rtc = |config0| {
            let mut dogs = Watchdogs::new(XTAL);
            dogs.write_rtc(x(0), 0x98, config0)
        };
        assert_eq!(rtc(EN | 1 << 28), Err("watchdog_interrupt"));
        assert_eq!(rtc(EN | 5 << 28), Err("watchdog_stage"));
        assert_eq!(rtc(EN | 2 << 28), Err("watchdog_cpu_reset"));
        assert_eq!(rtc(EN | 2 << 28 | 1 << 11), Ok(true));
        assert_eq!(rtc(EN | 4 << 28), Ok(true));
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_rtc(x(0), 0xb0, 0).unwrap();
        assert_eq!(dogs.write_rtc(x(0), 0x98, EN | 1 << 28), Ok(true), "locked");
    }

    #[test]
    fn a_super_watchdog_left_to_be_fed_is_refused() {
        let mut dogs = Watchdogs::new(XTAL);
        assert_eq!(dogs.write_rtc(x(0), 0xb4, 300 << 18), Err("super_watchdog"));
        assert_eq!(dogs.write_rtc(x(0), 0xb4, SWD_AUTO_FEED_EN), Ok(true));
        assert_eq!(dogs.read_rtc(0xb4), Some(SWD_AUTO_FEED_EN));
        dogs.write_rtc(x(0), 0xb8, 0).unwrap();
        assert_eq!(dogs.write_rtc(x(0), 0xb4, 0), Ok(true), "locked");
    }
}
