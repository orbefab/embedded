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
//!
//! Each timer group's general timers, T0 and T1, count a 54-bit value while `EN` is set, the
//! group's clock bit is set and its reset bit is clear. The divider is `TIMG_Tx_DIVIDER`, and a
//! stored 0 counts as 65536. The clock is the APB clock, or the 40 MHz crystal while
//! `TIMG_Tx_USE_XTAL` is set. The counter steps up or down, one step per divider period, the
//! first a whole period after it is enabled, reloaded, or `TIMG_Tx_DIVCNT_RST` is written. A
//! load copies `TIMG_Tx_LOAD` at that write. `TIMG_Tx_UPDATE` latches the count into the value
//! registers, which read the latch. An alarm fires when the count reaches `TIMG_Tx_ALARM` while
//! `TIMG_Tx_ALARM_EN` is set. A count that already equals the alarm, or has passed it by less
//! than half a turn in the counting direction, fires at once, as TRM §12.2.3's Tables 12.2-1 and
//! 12.2-2 say. The fire sets the timer's bit of `TIMG_INT_RAW_TIMERS`, clears
//! `TIMG_Tx_ALARM_EN`, and with `TIMG_Tx_AUTORELOAD` copies the load value into the count. The
//! interrupt source is high while that raw bit and its `TIMG_INT_ENA_TIMERS` bit are set. A full
//! turn of the counter whose product does not fit in the tick range is not scheduled. Writes
//! take effect at once. While the group's reset bit is set the two timers, the group's watchdog
//! and its interrupt enable sit at their reset values, and a store does not stick. While the
//! clock bit is clear the timers keep their count. `SYSTEM_WDG_RST` does the same for the RTC
//! watchdog: a stock boot sets that bit and leaves it set, and the timer-group watchdogs keep
//! running.

use super::clock::{TICK_HZ, Tick, XTAL};

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
    /// `SYSTEM_SYSTIMER_RST` is set. The registers stay at reset and the count does not move.
    held: bool,
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
            held: false,
        }
    }

    /// `SYSTEM_SYSTIMER_RST`, TRM §11.3. While `reset` is set the timer sits at its power-on
    /// registers, count included, and a store does not stick. Clearing it starts the count from
    /// 0 at `now`.
    pub(crate) fn set_reset(&mut self, now: Tick, reset: bool) {
        if reset == self.held {
            return;
        }
        if reset {
            *self = Systimer::new();
            self.held = true;
        }
        for unit in &mut self.units {
            unit.since = now;
        }
        self.held = reset;
    }

    fn value(&self, unit: usize, now: Tick) -> u64 {
        let u = &self.units[unit];
        if self.held || self.conf & UNIT_WORK_EN[unit] == 0 {
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
        if self.held {
            return;
        }
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
        if self.held {
            return None;
        }
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
        if self.held {
            return true;
        }
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
/// T0's and T1's bits in those same words.
const T0_INT: u32 = 1;
const T1_INT: u32 = 1 << 1;

/// The general timer's counter, 32 low bits and 22 high bits.
const COUNT_MASK: u64 = (1 << 54) - 1;
const COUNT_SPAN: u64 = 1 << 54;
/// Half the counter. TRM §12.2.3 compares a count with the alarm against this boundary.
const HALF: u64 = 1 << 53;
const HI_MASK: u32 = (1 << 22) - 1;

const GP_EN: u32 = 1 << 31;
const GP_INCREASE: u32 = 1 << 30;
const GP_AUTORELOAD: u32 = 1 << 29;
/// `TIMG_Tx_DIVIDER`, bits 28:13. A field of 0 divides by 65536 and a field of 1 divides by 2.
const GP_DIVIDER: u32 = 0xffff << 13;
const GP_DIVCNT_RST: u32 = 1 << 12;
const GP_ALARM_EN: u32 = 1 << 10;
const GP_USE_XTAL: u32 = 1 << 9;
/// Divider field 1, auto-reload, counting up. `timer_group_struct.h`'s reset value, `0x6000_2000`.
/// The field is 1, so the counter divides by 2.
const GP_CONFIG_RESET: u32 = (1 << 13) | GP_AUTORELOAD | GP_INCREASE;

/// One general timer. `base` is the count at tick `since`. `latched` is what the value
/// registers read. `armed` is the tick the alarm was last enabled, or the count or the alarm
/// last changed while it was enabled. An immediate alarm fires at that tick, or at `since`
/// when the clock turns on later.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GpTimer {
    config: u32,
    latched: u64,
    base: u64,
    since: Tick,
    alarm: u64,
    load: u64,
    armed: Tick,
    raw: bool,
}

impl GpTimer {
    fn reset() -> GpTimer {
        GpTimer {
            config: GP_CONFIG_RESET,
            latched: 0,
            base: 0,
            since: Tick(0),
            alarm: 0,
            load: 0,
            armed: Tick(0),
            raw: false,
        }
    }

    /// TRM §12.2.1. A field of 0 divides by 65536 and a field of 1 divides by 2. The reset field is 1.
    fn divider(config: u32) -> u64 {
        match u64::from((config >> 13) & 0xffff) {
            0 => 65536,
            1 => 2,
            field => field,
        }
    }

    /// Ticks per count: the divider times the APB period, or the crystal's while `USE_XTAL` is set.
    fn period(config: u32, apb: u64) -> u64 {
        let source = if config & GP_USE_XTAL != 0 { XTAL } else { apb };
        Self::divider(config) * source
    }

    fn counting(&self, clocked: bool) -> bool {
        clocked && self.config & GP_EN != 0
    }

    /// TRM §12.2.3, the immediate rows of Table 12.2-1 and Table 12.2-2.
    ///
    /// Counting up, the alarm fires at the write when `0 ≤ value − alarm < 2^53`, which includes
    /// equality, or when `alarm − value > 2^53`. Counting down is that table with the two numbers
    /// swapped: `0 ≤ alarm − value < 2^53`, or `value − alarm > 2^53`. A difference of exactly
    /// `2^53` is not immediate.
    fn fires_now(config: u32, value: u64, alarm: u64) -> bool {
        let (value, alarm) = (value & COUNT_MASK, alarm & COUNT_MASK);
        let (left, right) = if config & GP_INCREASE != 0 {
            (value, alarm)
        } else {
            (alarm, value)
        };
        if right > left {
            right - left > HALF
        } else {
            left - right < HALF
        }
    }

    /// Counts from `value` until the alarm. An immediate pair is 0. Every other pair is the
    /// forward distance, at most half a turn.
    fn steps_until(config: u32, value: u64, alarm: u64) -> u64 {
        let (value, alarm) = (value & COUNT_MASK, alarm & COUNT_MASK);
        if Self::fires_now(config, value, alarm) {
            return 0;
        }
        let (left, right) = if config & GP_INCREASE != 0 {
            (value, alarm)
        } else {
            (alarm, value)
        };
        if right > left {
            right - left
        } else {
            COUNT_SPAN - (left - right)
        }
    }

    fn stepped(&self, steps: u64) -> u64 {
        let base = self.base & COUNT_MASK;
        if self.config & GP_INCREASE != 0 {
            base.wrapping_add(steps) & COUNT_MASK
        } else {
            base.wrapping_sub(steps) & COUNT_MASK
        }
    }

    fn value_at(&self, now: Tick, apb: u64, clocked: bool) -> u64 {
        if !self.counting(clocked) || now.0 <= self.since.0 {
            return self.base & COUNT_MASK;
        }
        let steps = (now.0 - self.since.0) / Self::period(self.config, apb);
        self.stepped(steps)
    }

    /// The tick the armed alarm fires, or `None` when that product does not fit in `u64`.
    /// An immediate alarm fires at `armed`, or at `since` when the clock turned on later.
    fn next_fire(&self, apb: u64, clocked: bool) -> Option<Tick> {
        if !self.counting(clocked) || self.config & GP_ALARM_EN == 0 {
            return None;
        }
        let period = Self::period(self.config, apb);
        let k_done = if self.armed.0 <= self.since.0 {
            0
        } else {
            (self.armed.0 - self.since.0) / period
        };
        let value = self.stepped(k_done);
        if Self::fires_now(self.config, value, self.alarm) {
            return Some(Tick(self.armed.0.max(self.since.0)));
        }
        let steps = Self::steps_until(self.config, value, self.alarm);
        let k = k_done.checked_add(steps)?;
        let ticks = k.checked_mul(period)?;
        Some(Tick(self.since.0.checked_add(ticks)?))
    }

    /// Sets the raw bit and clears `ALARM_EN`. Auto-reload copies the load and counts from this
    /// tick. A one-shot leaves the count where it is: already at the alarm when it reached it,
    /// and short of the alarm when the fire was immediate.
    fn fire(&mut self, at: Tick) {
        self.raw = true;
        self.config &= !GP_ALARM_EN;
        if self.config & GP_AUTORELOAD != 0 {
            self.base = self.load & COUNT_MASK;
            self.since = at;
        }
    }

    /// Fires every alarm due at or before `now`. Enabling the alarm clears it, so one settle
    /// fires once.
    fn settle(&mut self, now: Tick, apb: u64, clocked: bool) {
        loop {
            let Some(at) = self.next_fire(apb, clocked) else {
                return;
            };
            if at.0 > now.0 {
                return;
            }
            self.fire(at);
        }
    }

    /// Holds the count at `now` and starts the prescaler there. A partial period is dropped.
    fn rebase(&mut self, now: Tick, apb: u64, clocked: bool) {
        self.base = self.value_at(now, apb, clocked);
        self.since = now;
        if self.config & GP_ALARM_EN != 0 {
            self.armed = now;
        }
    }

    /// A change of the divider while `EN` is set is refused before this writes anything.
    /// TRM §12.2.1 says to disable the timer first. `DIVCNT_RST`, `INCREASE` and `USE_XTAL`
    /// stay allowed: the warning covers the prescaler, and `USE_XTAL` selects the clock.
    fn write_config(
        &mut self,
        now: Tick,
        value: u32,
        apb: u64,
        clocked: bool,
    ) -> Result<(), &'static str> {
        let old = self.config;
        let new = value & !GP_DIVCNT_RST;
        if old & GP_EN != 0 && (old ^ new) & GP_DIVIDER != 0 {
            return Err("divider");
        }
        self.settle(now, apb, clocked);
        let phase = GP_EN | GP_INCREASE | GP_DIVIDER | GP_USE_XTAL;
        let restart = (old ^ new) & phase != 0 || value & GP_DIVCNT_RST != 0;
        if restart {
            self.base = self.value_at(now, apb, clocked);
            self.since = now;
        }
        let rose = old & GP_ALARM_EN == 0 && new & GP_ALARM_EN != 0;
        self.config = new;
        if (restart || rose) && new & GP_ALARM_EN != 0 {
            self.armed = now;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Watchdogs {
    timg: [Timer; 2],
    rtc: Timer,
    /// T0 and T1 of each timer group.
    gpt: [[GpTimer; 2]; 2],
    /// Each group's clock bit and reset bit. The timers and the watchdog run the reset bit.
    clocked: [bool; 2],
    held: [bool; 2],
    /// `SYSTEM_WDG_RST`. The RTC watchdog sits at its reset values while this is set.
    wdg_held: bool,
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
    fn mwdt(flashboot_runs: bool) -> Timer {
        Timer {
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
        }
    }

    fn rtc_timer() -> Timer {
        Timer {
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
        }
    }

    pub(crate) fn new(apb: u64) -> Watchdogs {
        Watchdogs {
            timg: [Self::mwdt(true), Self::mwdt(false)],
            rtc: Self::rtc_timer(),
            gpt: [
                [GpTimer::reset(), GpTimer::reset()],
                [GpTimer::reset(), GpTimer::reset()],
            ],
            clocked: [true; 2],
            held: [false; 2],
            wdg_held: false,
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

    fn frozen(&self, watchdog: Watchdog) -> bool {
        match watchdog {
            Watchdog::Rtc => self.wdg_held,
            Watchdog::Timg0 => self.held[0],
            Watchdog::Timg1 => self.held[1],
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
            if self.frozen(watchdog) {
                return None;
            }
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

    /// T0 and T1 of group 0, then T0 and T1 of group 1. A line is high while its raw bit and its
    /// enable are set.
    pub(crate) fn timer_lines(&mut self, now: Tick) -> [bool; 4] {
        self.settle_timers(now);
        let on = |group: usize, index: usize, bit: u32| {
            self.gpt[group][index].raw && self.int_ena[group] & bit != 0
        };
        [
            on(0, 0, T0_INT),
            on(0, 1, T1_INT),
            on(1, 0, T0_INT),
            on(1, 1, T1_INT),
        ]
    }

    /// The next general-timer alarm strictly after `now`.
    pub(crate) fn next_alarm(&mut self, now: Tick) -> Option<Tick> {
        self.settle_timers(now);
        let mut best: Option<Tick> = None;
        for group in 0..2 {
            if self.held[group] {
                continue;
            }
            let apb = self.apb;
            let clocked = self.clocked[group];
            for timer in &self.gpt[group] {
                let Some(at) = timer.next_fire(apb, clocked) else {
                    continue;
                };
                if at.0 > now.0 {
                    best = Some(match best {
                        Some(so_far) if so_far.0 <= at.0 => so_far,
                        _ => at,
                    });
                }
            }
        }
        best
    }

    /// The group's clock and reset, as [`super::gates::Gates::timg`] reads them. A reset puts
    /// the two general timers, the group's watchdog and its interrupt enable back to their reset
    /// values, and they stay there until the bit clears. A clock that turns off freezes the
    /// count. A clock that turns on counts on from there.
    pub(crate) fn apply_gates(&mut self, now: Tick, clocked: [bool; 2], held: [bool; 2]) {
        for group in 0..2 {
            if held[group] {
                if !self.held[group] {
                    self.gpt[group] = [GpTimer::reset(), GpTimer::reset()];
                    self.timg[group] = Self::mwdt(group == 0);
                    self.timg[group].since = now;
                    self.timg[group].written = now;
                    self.int_ena[group] = 0;
                }
            } else if self.held[group] {
                self.timg[group].restart(now);
            } else if !clocked[group] && self.clocked[group] {
                let apb = self.apb;
                for timer in &mut self.gpt[group] {
                    timer.settle(now, apb, true);
                    timer.rebase(now, apb, true);
                }
            } else if clocked[group] && !self.clocked[group] {
                for timer in &mut self.gpt[group] {
                    timer.since = now;
                }
            }
            self.clocked[group] = clocked[group];
            self.held[group] = held[group];
        }
    }

    /// `SYSTEM_WDG_RST`. While set, the RTC watchdog sits at its power-on registers and does not
    /// count. Clearing the bit starts that count from `now`. The timer-group watchdogs are not
    /// this one: boot sets the bit and leaves it, and those watchdogs keep running.
    pub(crate) fn apply_system_wdg(&mut self, now: Tick, reset: bool) {
        if reset == self.wdg_held {
            return;
        }
        if reset {
            self.rtc = Self::rtc_timer();
            self.rtc.since = now;
            self.rtc.written = now;
            self.wdg_held = true;
            return;
        }
        self.rtc.restart(now);
        self.wdg_held = false;
    }

    fn settle_group(&mut self, now: Tick, group: usize) {
        if self.held[group] {
            return;
        }
        let apb = self.apb;
        let clocked = self.clocked[group];
        for timer in &mut self.gpt[group] {
            timer.settle(now, apb, clocked);
        }
    }

    fn settle_timers(&mut self, now: Tick) {
        for group in 0..2 {
            self.settle_group(now, group);
        }
    }

    fn raw_bits(&self, group: usize) -> u32 {
        let mut raw = u32::from(self.timg[group].raised) * WDT_INT;
        if self.gpt[group][0].raw {
            raw |= T0_INT;
        }
        if self.gpt[group][1].raw {
            raw |= T1_INT;
        }
        raw
    }

    /// The APB clock runs at `apb` ticks per cycle from `now` on. A general timer keeps the
    /// counts it has taken and drops a partial divider period.
    pub(crate) fn reclock(&mut self, now: Tick, apb: u64) {
        for (group, timer) in self.timg.iter_mut().enumerate() {
            if self.held[group] {
                continue;
            }
            timer.settle(now, self.apb);
        }
        let old = self.apb;
        for group in 0..2 {
            if self.held[group] {
                continue;
            }
            let clocked = self.clocked[group];
            for timer in &mut self.gpt[group] {
                timer.settle(now, old, clocked);
                timer.rebase(now, old, clocked);
            }
        }
        self.apb = apb;
    }

    /// A timer group's register at `offset` in the group's block. The counter value registers
    /// read the latch, and a read of the raw or status word settles a due alarm first.
    pub(crate) fn read_timg(&mut self, now: Tick, group: usize, offset: u32) -> Option<u32> {
        if offset <= 0x44 {
            self.settle_group(now, group);
            return Some(self.read_gpt(group, offset));
        }
        if matches!(offset, 0x74 | 0x78) {
            self.settle_group(now, group);
        }
        let raw = self.raw_bits(group);
        let t = &self.timg[group];
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

    fn read_gpt(&self, group: usize, offset: u32) -> u32 {
        let index = usize::from(offset >= 0x24);
        let timer = &self.gpt[group][index];
        match offset % 0x24 {
            0x00 => timer.config,
            0x04 => timer.latched as u32,
            0x08 => (timer.latched >> 32) as u32,
            0x0c | 0x20 => 0,
            0x10 => timer.alarm as u32,
            0x14 => (timer.alarm >> 32) as u32,
            0x18 => timer.load as u32,
            0x1c => (timer.load >> 32) as u32,
            _ => 0,
        }
    }

    /// Writes one general-timer register. While the group is held in reset the write does not
    /// stick. `LO` and `HI` are ignored. An update latches the live count, and a load copies
    /// the load registers into the counter, both at `now`.
    fn write_gpt(
        &mut self,
        now: Tick,
        group: usize,
        offset: u32,
        value: u32,
    ) -> Result<bool, &'static str> {
        if offset > 0x44 {
            return Ok(false);
        }
        if self.held[group] {
            return Ok(true);
        }
        let index = usize::from(offset >= 0x24);
        let local = offset % 0x24;
        let apb = self.apb;
        let clocked = self.clocked[group];
        let timer = &mut self.gpt[group][index];
        match local {
            0x00 => timer.write_config(now, value, apb, clocked)?,
            0x04 | 0x08 => {}
            0x0c => {
                timer.settle(now, apb, clocked);
                timer.latched = timer.value_at(now, apb, clocked);
            }
            0x10 => {
                timer.settle(now, apb, clocked);
                timer.alarm = (timer.alarm & !0xffff_ffff) | u64::from(value);
                if timer.config & GP_ALARM_EN != 0 {
                    timer.armed = now;
                }
            }
            0x14 => {
                timer.settle(now, apb, clocked);
                let hi = u64::from(value & HI_MASK) << 32;
                timer.alarm = (timer.alarm & 0xffff_ffff) | hi;
                if timer.config & GP_ALARM_EN != 0 {
                    timer.armed = now;
                }
            }
            0x18 => timer.load = (timer.load & !0xffff_ffff) | u64::from(value),
            0x1c => {
                let hi = u64::from(value & HI_MASK) << 32;
                timer.load = (timer.load & 0xffff_ffff) | hi;
            }
            0x20 => {
                timer.settle(now, apb, clocked);
                timer.base = timer.load & COUNT_MASK;
                timer.since = now;
                if timer.config & GP_ALARM_EN != 0 {
                    timer.armed = now;
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// A write of 1 to `TIMG_INT_RAW_TIMERS_REG` or `TIMG_INT_CLR_TIMERS_REG` clears that bit.
    /// Both names are write-to-clear, for T0, T1 and the watchdog. The status register is not.
    fn clear_raw(&mut self, now: Tick, group: usize, value: u32) {
        self.settle_group(now, group);
        if value & T0_INT != 0 {
            self.gpt[group][0].raw = false;
        }
        if value & T1_INT != 0 {
            self.gpt[group][1].raw = false;
        }
        self.timg[group].raised &= value & WDT_INT == 0;
    }

    /// Writes a timer group's register, or names the setting the write leaves that the model
    /// does not run. A general-timer write is kept, including while the group is held in reset,
    /// where it does not stick.
    pub(crate) fn write_timg(
        &mut self,
        now: Tick,
        group: usize,
        offset: u32,
        value: u32,
    ) -> Result<bool, &'static str> {
        if self.held[group] && offset > 0x44 {
            return Ok(true);
        }
        if offset <= 0x44 {
            return self.write_gpt(now, group, offset, value);
        }
        let refused = match offset {
            0x48..=0x5c => {
                let index = ((offset - 0x48) / 4) as usize;
                self.timg[group].write(now, Some(index), value, false)
            }
            0x60 => self.timg[group].write(now, None, value, true),
            0x64 => {
                self.timg[group].wprotect = value;
                None
            }
            0x70 => {
                self.int_ena[group] = value & 0x7;
                None
            }
            0x74 | 0x7c => {
                self.clear_raw(now, group, value);
                None
            }
            0x78 => None,
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
        if self.wdg_held && (0x98..=0xb0).contains(&offset) {
            return Ok(true);
        }
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
        if let Register::Timg(group, offset) = register
            && offset <= 0x44
        {
            let _ = self.write_gpt(now, group, offset, value);
            return;
        }
        match register {
            Register::Timg(group, 0x70) => self.int_ena[group] = value & 0x7,
            Register::Timg(group, 0x74) => {
                self.timg[group].raised = value & WDT_INT != 0;
                self.gpt[group][0].raw = value & T0_INT != 0;
                self.gpt[group][1].raw = value & T1_INT != 0;
            }
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
        assert_eq!(dogs.read_timg(x(10), 0, 0x74), Some(WDT_INT));
        assert_eq!(dogs.lines(), [false, false], "not enabled");
        dogs.write_timg(x(10), 0, 0x70, WDT_INT).unwrap();
        assert_eq!(dogs.lines(), [true, false]);
        assert_eq!(dogs.read_timg(x(10), 0, 0x78), Some(WDT_INT));
        dogs.write_timg(x(10), 0, 0x7c, WDT_INT).unwrap();
        assert_eq!(dogs.lines(), [false, false]);
        assert_eq!(dogs.read_timg(x(10), 0, 0x74), Some(0));
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

    /// `EN`, counting up, divider `divider`, `ALARM_EN`. Auto-reload stays clear.
    fn arm(divider: u32) -> u32 {
        GP_EN | GP_INCREASE | (divider << 13) | GP_ALARM_EN
    }

    #[test]
    fn a_general_timer_fires_one_divider_period_after_each_count() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(0), 0, 0x10, 10).unwrap();
        dogs.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(dogs.next_alarm(x(0)), Some(x(20)));
        assert_eq!(dogs.read_timg(x(19), 0, 0x74), Some(0), "not yet");
        assert_eq!(dogs.read_timg(x(20), 0, 0x74), Some(T0_INT));
        assert_eq!(dogs.read_timg(x(20), 0, 0x00).unwrap() & GP_ALARM_EN, 0);
        assert_eq!(dogs.timer_lines(x(20)), [false; 4], "not enabled");
        dogs.write_timg(x(20), 0, 0x70, T0_INT).unwrap();
        assert_eq!(dogs.timer_lines(x(20)), [true, false, false, false]);
        assert_eq!(dogs.read_timg(x(20), 0, 0x78), Some(T0_INT));
        dogs.write_timg(x(20), 0, 0x7c, T0_INT).unwrap();
        assert_eq!(dogs.timer_lines(x(20)), [false; 4]);
    }

    #[test]
    fn settling_twice_matches_settling_once_and_an_equal_alarm_fires_at_the_arm() {
        let mut early = Watchdogs::new(XTAL);
        early.write_timg(x(0), 0, 0x10, 10).unwrap();
        early.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        let mut late = early.clone();
        assert_eq!(early.read_timg(x(5), 0, 0x74), Some(0));
        assert_eq!(early.read_timg(x(30), 0, 0x74), Some(T0_INT));
        assert_eq!(late.read_timg(x(30), 0, 0x74), Some(T0_INT));
        assert_eq!(early, late);

        let mut same = Watchdogs::new(XTAL);
        same.write_timg(x(0), 0, 0x10, 0).unwrap();
        same.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(
            same.read_timg(x(0), 0, 0x74),
            Some(T0_INT),
            "equal fires now"
        );
        assert!(same.next_alarm(x(0)).is_none(), "the fire cleared ALARM_EN");
    }

    /// Copies `count` into T0 through the load registers. The timer is stopped, so this does not fire.
    fn load_count(dogs: &mut Watchdogs, count: u64) {
        dogs.write_timg(x(0), 0, 0x18, count as u32).unwrap();
        dogs.write_timg(x(0), 0, 0x1c, (count >> 32) as u32)
            .unwrap();
        dogs.write_timg(x(0), 0, 0x20, 1).unwrap();
    }

    #[test]
    fn every_alarm_scenario_in_the_manual_fires_now_or_on_the_way_to_the_alarm() {
        let half = HALF;
        let max = COUNT_MASK;
        // (up, value, alarm, immediate, forward steps). Tables 12.2-1 and 12.2-2.
        let cases = [
            (true, 0, half + 1, true, 0),
            (true, 0, half, false, half),
            (true, 0, 10, false, 10),
            (true, 10, 10, true, 0),
            (true, half - 1, 0, true, 0),
            (true, half, 0, false, half),
            (true, max, 0, false, 1),
            (false, half + 1, 0, true, 0),
            (false, half, 0, false, half),
            (false, 20, 5, false, 15),
            (false, 5, 5, true, 0),
            (false, 0, half - 1, true, 0),
            (false, 5, 20, true, 0),
            (false, 0, half, false, half),
            (false, 0, max, false, 1),
        ];
        for (up, value, alarm, now, steps) in cases {
            let config = if up { GP_INCREASE } else { 0 };
            assert_eq!(
                GpTimer::fires_now(config, value, alarm),
                now,
                "up={up} value={value:#x} alarm={alarm:#x}"
            );
            assert_eq!(
                GpTimer::steps_until(config, value, alarm),
                if now { 0 } else { steps },
                "up={up} value={value:#x} alarm={alarm:#x}"
            );
        }

        // Field 2 divides by 2, so one count is two crystal cycles. An immediate pair fires at
        // the arm. A pair half a turn away is scheduled, and the same pair at divider 65536 is not.
        let mut ahead = Watchdogs::new(XTAL);
        ahead.write_timg(x(0), 0, 0x10, 10).unwrap();
        ahead.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(ahead.read_timg(x(0), 0, 0x74), Some(0));
        assert_eq!(ahead.next_alarm(x(0)), Some(x(20)));

        let mut equal = Watchdogs::new(XTAL);
        load_count(&mut equal, 1000);
        equal.write_timg(x(0), 0, 0x10, 1000).unwrap();
        equal.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(equal.read_timg(x(0), 0, 0x74), Some(T0_INT));
        equal.write_timg(x(0), 0, 0x0c, 1).unwrap();
        assert_eq!(
            equal.read_timg(x(0), 0, 0x04),
            Some(1000),
            "a one-shot stays put"
        );

        let mut behind = Watchdogs::new(XTAL);
        load_count(&mut behind, 2000);
        behind.write_timg(x(0), 0, 0x10, 1000).unwrap();
        behind.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(behind.read_timg(x(0), 0, 0x74), Some(T0_INT));

        let mut half_turn = Watchdogs::new(XTAL);
        load_count(&mut half_turn, half);
        half_turn.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(half_turn.read_timg(x(0), 0, 0x74), Some(0));
        assert_eq!(half_turn.next_alarm(x(0)), Some(Tick(half * 2 * XTAL)));

        let mut wide = Watchdogs::new(XTAL);
        load_count(&mut wide, half);
        wide.write_timg(x(0), 0, 0x00, arm(0)).unwrap();
        assert_eq!(wide.read_timg(x(0), 0, 0x74), Some(0));
        assert!(
            wide.next_alarm(x(0)).is_none(),
            "half a turn at divider 65536 does not fit"
        );
    }

    #[test]
    fn auto_reload_copies_the_load_and_a_load_above_the_alarm_fires_again() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(0), 0, 0x18, 3).unwrap();
        dogs.write_timg(x(0), 0, 0x10, 10).unwrap();
        dogs.write_timg(x(0), 0, 0x00, arm(2) | GP_AUTORELOAD)
            .unwrap();
        assert_eq!(dogs.read_timg(x(20), 0, 0x74), Some(T0_INT));
        dogs.write_timg(x(20), 0, 0x0c, 1).unwrap();
        assert_eq!(dogs.read_timg(x(20), 0, 0x04), Some(3), "reloaded");
        dogs.write_timg(x(20), 0, 0x7c, T0_INT).unwrap();
        dogs.write_timg(x(20), 0, 0x00, arm(2) | GP_AUTORELOAD)
            .unwrap();
        assert_eq!(dogs.next_alarm(x(20)), Some(x(34)));

        // The driver's interrupt re-enables ALARM_EN after the reload. A load above the alarm
        // is an immediate pair, so the second fire is the re-enable itself.
        let mut above = Watchdogs::new(XTAL);
        above.write_timg(x(0), 0, 0x18, 50).unwrap();
        above.write_timg(x(0), 0, 0x10, 10).unwrap();
        above
            .write_timg(x(0), 0, 0x00, arm(2) | GP_AUTORELOAD)
            .unwrap();
        assert_eq!(above.read_timg(x(20), 0, 0x74), Some(T0_INT));
        above.write_timg(x(20), 0, 0x7c, T0_INT).unwrap();
        above
            .write_timg(x(20), 0, 0x00, arm(2) | GP_AUTORELOAD)
            .unwrap();
        assert_eq!(above.read_timg(x(20), 0, 0x74), Some(T0_INT));
        above.write_timg(x(20), 0, 0x0c, 1).unwrap();
        assert_eq!(above.read_timg(x(20), 0, 0x04), Some(50));

        let mut down = Watchdogs::new(XTAL);
        down.write_timg(x(0), 0, 0x18, 20).unwrap();
        down.write_timg(x(0), 0, 0x20, 1).unwrap();
        down.write_timg(x(0), 0, 0x10, 5).unwrap();
        down.write_timg(x(0), 0, 0x00, GP_EN | GP_ALARM_EN | (2 << 13))
            .unwrap();
        assert_eq!(down.next_alarm(x(0)), Some(x(30)));

        let mut zero = Watchdogs::new(XTAL);
        zero.write_timg(x(0), 0, 0x10, 1).unwrap();
        zero.write_timg(x(0), 0, 0x00, arm(0)).unwrap();
        assert_eq!(zero.next_alarm(x(0)), Some(Tick(65536 * XTAL)));
    }

    #[test]
    fn a_divider_field_of_one_divides_by_two_and_a_change_while_enabled_is_refused() {
        let mut one = Watchdogs::new(XTAL);
        one.write_timg(x(0), 0, 0x10, 4).unwrap();
        one.write_timg(x(0), 0, 0x00, arm(1)).unwrap();
        let mut two = Watchdogs::new(XTAL);
        two.write_timg(x(0), 0, 0x10, 4).unwrap();
        two.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(one.next_alarm(x(0)), Some(x(8)));
        assert_eq!(two.next_alarm(x(0)), one.next_alarm(x(0)));
        assert_eq!(one.read_timg(x(0), 0, 0x00).unwrap() & GP_DIVIDER, 1 << 13);

        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(0), 0, 0x10, 10).unwrap();
        dogs.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        let before = dogs.clone();
        assert_eq!(dogs.write_timg(x(30), 0, 0x00, arm(3)), Err("divider"));
        assert_eq!(dogs, before, "the refused store changes nothing");
        assert_eq!(
            dogs.write_timg(x(30), 0, 0x00, arm(2)),
            Ok(true),
            "the same field"
        );

        let mut stopped = Watchdogs::new(XTAL);
        assert_eq!(
            stopped.write_timg(x(0), 0, 0x00, GP_INCREASE | (4 << 13)),
            Ok(true),
            "EN is clear"
        );
        assert_eq!(
            stopped.write_timg(x(0), 0, 0x00, arm(8)),
            Ok(true),
            "the write that sets EN may change the field"
        );

        // DIVCNT_RST drops the partial period. At three crystal cycles, field 2 has taken one
        // count of an alarm at 4, so three counts remain and the next fire is six cycles on.
        let mut reset = Watchdogs::new(XTAL);
        reset.write_timg(x(0), 0, 0x10, 4).unwrap();
        reset.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(
            reset.write_timg(x(3), 0, 0x00, arm(2) | GP_DIVCNT_RST),
            Ok(true)
        );
        assert_eq!(reset.read_timg(x(3), 0, 0x00).unwrap() & GP_DIVCNT_RST, 0);
        assert_eq!(reset.next_alarm(x(3)), Some(x(9)));

        // Turning around at count 0 puts the alarm behind the counter, so it fires at the write.
        let mut down = Watchdogs::new(XTAL);
        down.write_timg(x(0), 0, 0x10, 10).unwrap();
        down.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(
            down.write_timg(x(0), 0, 0x00, GP_EN | GP_ALARM_EN | (2 << 13)),
            Ok(true)
        );
        assert_eq!(down.read_timg(x(0), 0, 0x74), Some(T0_INT));

        let mut xtal = Watchdogs::new(6);
        xtal.write_timg(Tick(0), 0, 0x10, 4).unwrap();
        xtal.write_timg(Tick(0), 0, 0x00, arm(2)).unwrap();
        assert_eq!(xtal.next_alarm(Tick(0)), Some(Tick(48)));
        assert_eq!(
            xtal.write_timg(Tick(0), 0, 0x00, arm(2) | GP_USE_XTAL),
            Ok(true)
        );
        assert_eq!(xtal.next_alarm(Tick(0)), Some(Tick(8 * XTAL)));
    }

    #[test]
    fn a_write_of_one_to_the_raw_register_clears_that_bit() {
        let mut dogs = Watchdogs::new(XTAL);
        dogs.write_timg(x(0), 0, 0x10, 1).unwrap();
        dogs.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        dogs.write_timg(x(2), 0, 0x74, 0).unwrap();
        assert_eq!(dogs.read_timg(x(2), 0, 0x74), Some(T0_INT), "a zero stays");
        dogs.write_timg(x(2), 0, 0x74, T1_INT).unwrap();
        assert_eq!(dogs.read_timg(x(2), 0, 0x74), Some(T0_INT));
        dogs.write_timg(x(2), 0, 0x78, T0_INT).unwrap();
        assert_eq!(
            dogs.read_timg(x(2), 0, 0x74),
            Some(T0_INT),
            "status is not write-to-clear"
        );
        dogs.write_timg(x(2), 0, 0x74, T0_INT).unwrap();
        assert_eq!(dogs.read_timg(x(2), 0, 0x74), Some(0));

        let mut both = Watchdogs::new(XTAL);
        both.write_timg(x(0), 0, 0x10, 1).unwrap();
        both.write_timg(x(0), 0, 0x34, 1).unwrap();
        both.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        both.write_timg(x(0), 0, 0x24, arm(2)).unwrap();
        assert_eq!(both.read_timg(x(2), 0, 0x74), Some(T0_INT | T1_INT));
        both.write_timg(x(2), 0, 0x74, T0_INT).unwrap();
        assert_eq!(both.read_timg(x(2), 0, 0x74), Some(T1_INT));

        // A clear at the fire tick settles the alarm and then clears the bit it names.
        let mut due = Watchdogs::new(XTAL);
        due.write_timg(x(0), 0, 0x10, 1).unwrap();
        due.write_timg(x(0), 0, 0x00, arm(2)).unwrap();
        due.write_timg(x(2), 0, 0x74, T0_INT).unwrap();
        assert_eq!(due.read_timg(x(2), 0, 0x74), Some(0));

        let mut wdt = group0(EN | 1 << 29 | 3 << 27);
        wdt.run_out(wdt.deadline().unwrap());
        assert_eq!(wdt.read_timg(x(10), 0, 0x74), Some(WDT_INT));
        wdt.write_timg(x(10), 0, 0x74, WDT_INT).unwrap();
        assert_eq!(wdt.read_timg(x(10), 0, 0x74), Some(0));
    }

    #[test]
    fn the_crystal_source_ignores_the_apb_period_and_a_gate_freezes_or_resets() {
        let mut dogs = Watchdogs::new(6);
        dogs.write_timg(x(0), 0, 0x10, 4).unwrap();
        dogs.write_timg(x(0), 0, 0x00, arm(2) | GP_USE_XTAL)
            .unwrap();
        assert_eq!(dogs.next_alarm(Tick(0)), Some(Tick(8 * XTAL)));

        // One count has landed at two crystal cycles. The clock stops one cycle later.
        dogs.apply_gates(Tick(3 * XTAL), [false, true], [false, false]);
        assert!(dogs.next_alarm(Tick(10 * XTAL)).is_none(), "clock off");
        dogs.write_timg(Tick(10 * XTAL), 0, 0x0c, 1).unwrap();
        assert_eq!(dogs.read_timg(Tick(10 * XTAL), 0, 0x04), Some(1));
        dogs.apply_gates(Tick(10 * XTAL), [true, true], [false, false]);
        assert_eq!(
            dogs.next_alarm(Tick(10 * XTAL)),
            Some(Tick(10 * XTAL + 6 * XTAL))
        );

        // A count already past the alarm fires when the clock comes back, at that tick.
        let mut late = Watchdogs::new(XTAL);
        load_count(&mut late, 20);
        late.write_timg(x(0), 0, 0x10, 5).unwrap();
        late.write_timg(x(0), 0, 0x00, GP_EN | GP_INCREASE | (2 << 13))
            .unwrap();
        late.apply_gates(x(1), [false, true], [false, false]);
        late.write_timg(x(2), 0, 0x00, arm(2)).unwrap();
        assert_eq!(late.read_timg(x(2), 0, 0x74), Some(0), "frozen");
        late.apply_gates(x(5), [true, true], [false, false]);
        assert_eq!(late.read_timg(x(5), 0, 0x74), Some(T0_INT));

        dogs.write_timg(Tick(10 * XTAL), 0, 0x48, EN | 3 << 29)
            .unwrap();
        dogs.write_timg(Tick(10 * XTAL), 0, 0x70, T0_INT).unwrap();
        dogs.apply_gates(Tick(10 * XTAL), [true, true], [true, false]);
        assert_eq!(
            dogs.read_timg(Tick(10 * XTAL), 0, 0x48),
            Some(0x0004_c000),
            "the watchdog is back at reset"
        );
        assert_ne!(dogs.deadline().unwrap().watchdog, Watchdog::Timg0);
        assert_eq!(dogs.write_timg(Tick(10 * XTAL), 0, 0x00, arm(80)), Ok(true));
        assert_eq!(
            dogs.read_timg(Tick(10 * XTAL), 0, 0x00),
            Some(GP_CONFIG_RESET),
            "a write while held does not stick"
        );
        assert_eq!(dogs.read_timg(Tick(10 * XTAL), 0, 0x70), Some(0));
    }
}
