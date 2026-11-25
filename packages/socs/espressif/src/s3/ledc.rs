//! The LED PWM controller, LEDC: its eight channels and four timers, as ESP-IDF's `ledc` driver
//! sets fixed duties, resets, pauses and reconfigures timers at runtime, and runs hardware duty
//! fades with their interrupts.
//!
//! A timer counts from the store that leaves its `RST` and `PAUSE` bits clear. Pausing a timer
//! with `LEDC_LSTIMERn_PAUSE` freezes its counter and holds the output levels of its channels
//! until `PAUSE` clears, when it resumes from the saved phase. Setting `LEDC_LSTIMERn_RST` resets
//! the timer's counter to 0 and holds it at 0 regardless of `PAUSE` (TRM v1.8, 35.3.2.3 and
//! Register 35.7): no overflows occur while `RST` is set, so pending updates wait and active fades
//! pause, and each channel on that timer with `LEDC_SIG_OUT_EN_LSCHn` set and nonzero duty sees
//! `Timerx_cnt == Hpoint` (0) and drives high (TRM 35.3.3). Clearing `RST` with `PAUSE` clear
//! starts counting from 0 at that store, restarting PWM cycles and remaining fade overflow counts
//! on the timer's current period; clearing `RST` with `PAUSE` set leaves the counter suspended at
//! 0. Its period is `LEDC_CLK_DIV` over 256, times `2^DUTY_RES`, cycles of the clock
//! `LEDC_APB_CLK_SEL` picks: the APB clock, or the 40 MHz crystal. While a timer runs or is
//! paused, writing `LEDC_CLK_DIV_TIMERx` or `LEDC_TIMERx_DUTY_RES` stages the new values without
//! changing the active period; setting `LEDC_LSTIMERn_PARA_UP` latches them to take effect at the
//! next overflow of that timer (TRM v1.8, 35.3.2.2, 35.3.2.3 and Register 35.7), or on a reset
//! before that overflow. A later `PARA_UP` before that overflow replaces it. A store that sets
//! `LEDC_PARA_UP_LSCHn` takes the channel's registers as they are then, and that update takes
//! effect at the next overflow of the timer the channel names, the next PWM cycle, as IDF's
//! `ledc_update_duty` says. A later update before that overflow replaces it. An update whose timer
//! does not count waits until it does. An update with `LEDC_DUTY_START_LSCHn` clear keeps the duty
//! the last one took and stops any fade. `ledc_stop` stores only the idle level, a clear
//! `LEDC_SIG_OUT_EN_LSCHn` and the update bit, and `LEDC_DUTY_START_LSCHn` does not clear itself,
//! so after `ledc_update_duty` that update takes the duty register again.
//!
//! When `LEDC_DUTY_START_LSCHn` is set and both `LEDC_DUTY_NUM_LSCHn` and `LEDC_DUTY_CYCLE_LSCHn`
//! are nonzero, after the initial duty takes effect at the first overflow, the channel steps its
//! duty by `LEDC_DUTY_SCALE_LSCHn` (up when `LEDC_DUTY_INC_LSCHn` is set, down when clear) every
//! `LEDC_DUTY_CYCLE_LSCHn` timer overflows for `LEDC_DUTY_NUM_LSCHn` steps (TRM v1.8, 35.3.4).
//! After the final step it sets `LEDC_DUTY_CHNG_END_LSCHn_INT_RAW`. Each timer overflow sets
//! `LEDC_LSTIMERn_OVF_INT_RAW`. Source 35 is high while `LEDC_INT_ST_REG` is nonzero.
//!
//! The channel's signal is the PWM of the ESP32-S3 TRM (v1.8, 35.3.3): high when the timer's
//! counter is `Hpoint`, which is 0, and low when it is `Lpoint`, the duty. So from each overflow
//! of its timer the signal is high for the duty's steps and low for the rest of the period. A
//! step that is not whole ticks of 480 MHz rounds the high time to the nearest tick. A duty of 0
//! is low throughout and a duty at or above `2^DUTY_RES` (and below the 14-bit counter limit
//! `2^14`) is high throughout. While `LEDC_SIG_OUT_EN_LSCHn` is clear the channel sends its idle
//! level, a duty of 0 or of every step. At power-on every signal is low.
//!
//! `LEDC_LSCHn_DUTY_R_REG` reads the duty in force, from the last update or fade step that took
//! effect, with its 4 fraction bits clear, and 0 before the first (TRM 35.5, Register 35.6).
//! ESP-IDF's `ledc_get_duty` reads it, and arduino-esp32's `ledcAttach` keeps the duty it reads.
//! The self-clearing update bits read 0. `LEDC_LSTIMERn_VALUE_REG` is not kept. The registers
//! come from IDF's `soc/esp32s3/register/soc/ledc_reg.h`, with the reset values its field
//! comments give.

use super::clock::{TICK_HZ, Tick, XTAL};

pub(crate) const CHANNELS: usize = 8;
const TIMERS: usize = 4;
const CHANNEL: u32 = 0x14;
const TIMER_BASE: u32 = 0xa0;
const TIMER: u32 = 0x08;
const INT_RAW: u32 = 0xc0;
const INT_ST: u32 = 0xc4;
/// `LEDC_INT_ENA_REG`.
pub(crate) const INT_ENA: u32 = 0xc8;
const INT_CLR: u32 = 0xcc;
const CONF: u32 = 0xd0;
const DATE: u32 = 0xfc;
const WORDS: usize = (DATE / 4 + 1) as usize;

/// `ETS_LEDC_INTR_SOURCE`, from IDF's `soc/esp32s3/include/soc/interrupts.h`.
pub(crate) const SOURCE: u32 = 35;
/// `LEDC_LSTIMER0..3_OVF_INT_ENA` (bits 0..=3) and `LEDC_DUTY_CHNG_END_LSCH0..7_INT_ENA`
/// (bits 4..=11). `LEDC_OVF_CNT_LSCH0..7_INT_ENA` (bits 12..=19) are not modeled and stay refused.
pub(crate) const INT_ENA_ALLOWED: u32 = 0x0fff;

/// `LEDC_LS_SIG_OUT0_IDX`, channel 0's output signal in the GPIO matrix, from IDF's
/// `soc/esp32s3/include/soc/gpio_sig_map.h`. Channel n's is this plus n.
pub(crate) const SIGNAL: u16 = 73;

const TIMER_SEL: u32 = 0x3;
const SIG_OUT_EN: u32 = 1 << 2;
const IDLE_LV: u32 = 1 << 3;
const PARA_UP: u32 = 1 << 4;
/// `LEDC_OVF_CNT_RESET_LSCHn` and `LEDC_PARA_UP_LSCHn` in `CONF0`.
const CHANNEL_STROBES: u32 = 1 << 16 | PARA_UP;
const HPOINT: u32 = 0x3fff;
/// `LEDC_DUTY_LSCHn`, with its 4 fraction bits.
const DUTY: u32 = 0x7ffff;
const DUTY_START: u32 = 1 << 31;
const DUTY_INC: u32 = 1 << 30;
const DUTY_NUM_SHIFT: u32 = 20;
const DUTY_CYCLE_SHIFT: u32 = 10;
const DUTY_SCALE: u32 = 0x3ff;

const DUTY_RES: u32 = 0xf;
/// `SOC_LEDC_TIMER_BIT_WIDTH`.
const MAX_RESOLUTION: u8 = 14;
const CLK_DIV: u32 = 0x3ffff << 4;
const PAUSE: u32 = 1 << 22;
const RST: u32 = 1 << 23;
const TICK_SEL: u32 = 1 << 24;
/// `LEDC_LSTIMERn_PARA_UP`.
const TIMER_STROBE: u32 = 1 << 25;
const TIMER_SETUP: u32 = DUTY_RES | CLK_DIV | TICK_SEL;

const CLK_SEL: u32 = 0x3;

/// `LEDC_DUTY_INC_LSCHn` in `CONF1`, and `LEDC_LSTIMERn_RST`.
const CONF1_RESET: u32 = DUTY_INC;
const TIMER_RESET: u32 = RST;
const DATE_RESET: u32 = 0x1904_0200;

/// One timer's counter state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    /// `RST` is set; counter is held at 0.
    Reset,
    /// `RST` and `PAUSE` are clear; effective cycle origin is `since`, so elapsed ticks within
    /// a period at tick `t >= since` is `(t - since) % period`.
    Running { since: Tick },
    /// `RST` is clear and `PAUSE` is set; counter was frozen at `at` with `elapsed` ticks into
    /// its period.
    Paused { at: Tick, elapsed: u64 },
}

/// Fade parameters latched from `LEDC_LSCHn_CONF1_REG` on `PARA_UP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FadeConfig {
    inc: bool,
    num: u32,
    cycle: u32,
    scale: u32,
}

/// A hardware fade currently running on a channel after its `PARA_UP` took effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveFade {
    timer: usize,
    out: bool,
    idle: bool,
    inc: bool,
    scale: u32,
    cycle: u32,
    /// Steps left to execute (at least 1).
    remaining: u32,
    /// The tick of the next fade step while `timer` is `Running`.
    next_step: Tick,
    /// Timer overflows left until `next_step` when `timer` is `Paused` or `Reset` (at least 1).
    cycles_left: u32,
}

/// A timer's `DUTY_RES | CLK_DIV | TICK_SEL` latched on `LEDC_LSTIMERn_PARA_UP`, waiting for
/// the timer's next overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimerUpdate {
    at: Tick,
    conf: u32,
}

/// A channel's applied output configuration from its last `PARA_UP` that took effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Applied {
    timer: usize,
    out: bool,
    idle: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ledc {
    regs: [u32; WORDS],
    /// Each timer's counter state.
    timers: [Timer; TIMERS],
    /// Each timer's active `DUTY_RES | CLK_DIV | TICK_SEL` currently in force.
    timer_conf: [u32; TIMERS],
    /// Each timer's latched `LEDC_LSTIMERn_PARA_UP` update waiting for its next overflow.
    timer_pending: [Option<TimerUpdate>; TIMERS],
    /// The latest tick settled or cleared for each timer's overflow interrupt.
    ovf_mark: [Tick; TIMERS],
    /// Latched `LEDC_INT_RAW_REG` bits (bits 0..=11).
    int_raw: u32,
    /// Each channel's update that waits for its timer's next overflow.
    pending: [Option<Update>; CHANNELS],
    /// Each channel's active hardware fade.
    fade: [Option<ActiveFade>; CHANNELS],
    /// Each channel's applied output routing from its last update that took effect.
    applied: [Option<Applied>; CHANNELS],
    /// Each channel's duty, as its last update or fade step that took effect set it.
    duty: [u32; CHANNELS],
    /// What each channel sends, as its last update or fade step that took effect set it.
    sent: [Option<Effect>; CHANNELS],
    /// The updates that took effect and that the chip has not taken.
    done: Vec<Effect>,
    /// Each channel's PWM, while its signal switches.
    pwm: [Option<Pwm>; CHANNELS],
    /// Each channel's signal.
    level: [bool; CHANNELS],
    /// The changes of a channel's signal that the chip has not taken, in time order.
    edges: Vec<Edge>,
}

/// A channel's PWM at a duty above 0 and below every step: high from `start`, the overflow its
/// cycle began at, for `high` ticks of the `period`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pwm {
    timer: usize,
    start: Tick,
    period: u64,
    high: u64,
}

/// Channel `channel`'s signal goes to `level` at `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Edge {
    pub(crate) at: Tick,
    pub(crate) channel: u8,
    pub(crate) level: bool,
}

/// A channel's registers as a store of its update bit left them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Update {
    at: Tick,
    timer: usize,
    duty: u32,
    out: bool,
    idle: bool,
    fade: Option<FadeConfig>,
}

/// An update that took effect: at `at`, channel `channel` sends `duty` of `2^resolution` steps
/// at `hz`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Effect {
    pub(crate) at: Tick,
    pub(crate) channel: u8,
    pub(crate) duty: u32,
    pub(crate) resolution: u8,
    pub(crate) hz: u32,
}

/// What the register at `offset` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Kept,
    /// `LEDC_INT_RAW_REG`: read-only raw interrupt status.
    IntRaw,
    /// `LEDC_INT_ST_REG`: read-only masked interrupt status.
    IntSt,
    /// `LEDC_INT_CLR_REG`: write-only clear of raw interrupt bits, reads 0.
    IntClr,
    /// `LEDC_LSCHn_DUTY_R_REG`: it reads the channel's duty in force and keeps nothing.
    DutyRead,
    /// A word with bits that clear themselves.
    Strobed(u32),
}

fn kind(offset: u32) -> Option<Kind> {
    Some(match offset {
        _ if offset < CHANNELS as u32 * CHANNEL => match offset % CHANNEL {
            0x00 => Kind::Strobed(CHANNEL_STROBES),
            0x04 | 0x08 | 0x0c => Kind::Kept,
            0x10 => Kind::DutyRead,
            _ => return None,
        },
        TIMER_BASE..INT_RAW => match offset % TIMER {
            0 => Kind::Strobed(TIMER_STROBE),
            _ => return None,
        },
        INT_RAW => Kind::IntRaw,
        INT_ST => Kind::IntSt,
        INT_CLR => Kind::IntClr,
        INT_ENA | CONF | DATE => Kind::Kept,
        _ => return None,
    })
}

impl Ledc {
    pub(crate) fn new() -> Ledc {
        let mut regs = [0; WORDS];
        for channel in 0..CHANNELS as u32 {
            regs[((channel * CHANNEL + 0x0c) / 4) as usize] = CONF1_RESET;
        }
        for timer in 0..TIMERS as u32 {
            regs[((TIMER_BASE + timer * TIMER) / 4) as usize] = TIMER_RESET;
        }
        regs[(DATE / 4) as usize] = DATE_RESET;
        Ledc {
            regs,
            timers: [Timer::Reset; TIMERS],
            timer_conf: [0; TIMERS],
            timer_pending: [None; TIMERS],
            ovf_mark: [Tick(0); TIMERS],
            int_raw: 0,
            pending: [None; CHANNELS],
            fade: [None; CHANNELS],
            applied: [None; CHANNELS],
            duty: [0; CHANNELS],
            sent: [None; CHANNELS],
            done: Vec::new(),
            pwm: [None; CHANNELS],
            level: [false; CHANNELS],
            edges: Vec::new(),
        }
    }

    fn reg(&self, offset: u32) -> u32 {
        self.regs[(offset / 4) as usize]
    }

    fn channel(&self, channel: usize, offset: u32) -> u32 {
        self.reg(channel as u32 * CHANNEL + offset)
    }

    fn int_ena(&self) -> u32 {
        self.reg(INT_ENA) & INT_ENA_ALLOWED
    }

    fn running_since(&self, timer: usize) -> Option<Tick> {
        match self.timers[timer] {
            Timer::Running { since } => Some(since),
            Timer::Reset | Timer::Paused { .. } => None,
        }
    }

    /// Reads the register at `offset` at `now`, with `apb` ticks per APB cycle.
    pub(crate) fn read(&self, now: Tick, offset: u32, apb: u64) -> Option<u32> {
        let word = self.reg(offset);
        Some(match kind(offset)? {
            Kind::Kept => word,
            Kind::IntRaw => self.raw_at(now, apb),
            Kind::IntSt => self.raw_at(now, apb) & self.int_ena(),
            Kind::IntClr => 0,
            Kind::Strobed(strobes) => word & !strobes,
            Kind::DutyRead => self.duty_at((offset / CHANNEL) as usize, now, apb) << 4,
        })
    }

    /// Channel `channel`'s duty in force at `now`: taking into account any pending update or fade
    /// steps due at or before `now`.
    fn duty_at(&self, channel: usize, now: Tick, apb: u64) -> u32 {
        let (mut cur, active, timer_switched) =
            match (self.pending[channel], self.due(channel, apb)) {
                (Some(update), Some(due)) if due <= now => {
                    let switched = self
                        .timer_due(update.timer, apb)
                        .is_some_and(|tdue| tdue <= due);
                    let next_fade = update.fade.and_then(|cfg| {
                        let (period, _) = if switched {
                            self.period_for_conf(self.timer_pending[update.timer]?.conf, apb)
                                .ok()?
                        } else {
                            self.period(update.timer, apb).ok()?
                        };
                        Some(ActiveFade {
                            timer: update.timer,
                            out: update.out,
                            idle: update.idle,
                            inc: cfg.inc,
                            scale: cfg.scale,
                            cycle: cfg.cycle,
                            remaining: cfg.num,
                            next_step: Tick(due.0 + u64::from(cfg.cycle) * period),
                            cycles_left: cfg.cycle,
                        })
                    });
                    (update.duty, next_fade, switched)
                }
                _ => (self.duty[channel], self.fade[channel], false),
            };
        if let Some(fade) = active
            && fade.scale != 0
            && self.running_since(fade.timer).is_some()
        {
            let done = if !timer_switched
                && let Some(tdue) = self.timer_due(fade.timer, apb)
                && tdue <= now
                && let Ok((old_period, _)) = self.period(fade.timer, apb)
                && let Some(pending) = self.timer_pending[fade.timer]
                && let Ok((new_period, _)) = self.period_for_conf(pending.conf, apb)
            {
                let step_old = u64::from(fade.cycle) * old_period;
                let (done1, after_old) = if fade.next_step <= tdue {
                    let d =
                        ((tdue.0 - fade.next_step.0) / step_old + 1).min(u64::from(fade.remaining));
                    (d, fade.next_step.0 + d * step_old)
                } else {
                    (0, fade.next_step.0)
                };
                let rem = u64::from(fade.remaining) - done1;
                let done2 = if rem > 0 {
                    let cycles_left = ((after_old - tdue.0) / old_period).max(1);
                    let next2 = tdue.0 + cycles_left * new_period;
                    if next2 <= now.0 {
                        let step_new = u64::from(fade.cycle) * new_period;
                        ((now.0 - next2) / step_new + 1).min(rem)
                    } else {
                        0
                    }
                } else {
                    0
                };
                (done1 + done2) as u32
            } else {
                let period = if timer_switched {
                    self.timer_pending[fade.timer]
                        .and_then(|p| self.period_for_conf(p.conf, apb).ok())
                        .map(|(p, _)| p)
                } else {
                    self.period(fade.timer, apb).ok().map(|(p, _)| p)
                };
                if let Some(period) = period
                    && fade.next_step <= now
                {
                    let step_period = u64::from(fade.cycle) * period;
                    ((now.0 - fade.next_step.0) / step_period + 1).min(u64::from(fade.remaining))
                        as u32
                } else {
                    0
                }
            };
            if fade.inc {
                cur += done * fade.scale;
            } else {
                cur -= done * fade.scale;
            }
        }
        cur
    }

    /// The first overflow of `timer` strictly after `ovf_mark[timer]`, while it runs.
    fn next_ovf(&self, timer: usize, apb: u64) -> Option<Tick> {
        let since = self.running_since(timer)?;
        let (period, _) = self.period(timer, apb).ok()?;
        let mark = self.ovf_mark[timer];
        let cycles = mark.0.saturating_sub(since.0) / period + 1;
        Some(Tick(since.0 + cycles * period))
    }

    /// The earliest tick at which channel `channel` next finishes a fade and sets
    /// `LEDC_DUTY_CHNG_END_LSCHn_INT_RAW`, taking pending channel and timer updates into account.
    fn fade_end_at(&self, channel: usize, apb: u64) -> Option<Tick> {
        let active_end = self.fade[channel].and_then(|fade| {
            self.running_since(fade.timer)?;
            let (old_period, _) = self.period(fade.timer, apb).ok()?;
            if let Some(tdue) = self.timer_due(fade.timer, apb)
                && let Some(pending) = self.timer_pending[fade.timer]
                && let Ok((new_period, _)) = self.period_for_conf(pending.conf, apb)
            {
                if fade.next_step > tdue {
                    let cycles_left = ((fade.next_step.0 - tdue.0) / old_period).max(1);
                    let rest = u64::from(fade.remaining - 1) * u64::from(fade.cycle);
                    return Some(Tick(tdue.0 + (cycles_left + rest) * new_period));
                }
                let step_old = u64::from(fade.cycle) * old_period;
                let done_by_tdue =
                    ((tdue.0 - fade.next_step.0) / step_old + 1).min(u64::from(fade.remaining));
                let last_old = fade.next_step.0 + (done_by_tdue - 1) * step_old;
                if done_by_tdue == u64::from(fade.remaining) {
                    return Some(Tick(last_old));
                }
                let after_old = last_old + step_old;
                let cycles_left = ((after_old - tdue.0) / old_period).max(1);
                let rem_after = u64::from(fade.remaining) - done_by_tdue;
                let rest = (rem_after - 1) * u64::from(fade.cycle);
                return Some(Tick(tdue.0 + (cycles_left + rest) * new_period));
            }
            let rest = u64::from(fade.remaining - 1) * u64::from(fade.cycle) * old_period;
            Some(Tick(fade.next_step.0 + rest))
        });
        let due = self.due(channel, apb);
        if let Some(end) = active_end
            && due.is_none_or(|due| end < due)
        {
            return Some(end);
        }
        let update = self.pending[channel]?;
        let due = due?;
        let cfg = update.fade?;
        let (period, _) = if self
            .timer_due(update.timer, apb)
            .is_some_and(|tdue| tdue <= due)
        {
            self.period_for_conf(self.timer_pending[update.timer]?.conf, apb)
                .ok()?
        } else {
            self.period(update.timer, apb).ok()?
        };
        Some(Tick(
            due.0 + u64::from(cfg.num) * u64::from(cfg.cycle) * period,
        ))
    }

    /// `LEDC_INT_RAW_REG` as seen at `now`.
    fn raw_at(&self, now: Tick, apb: u64) -> u32 {
        let mut raw = self.int_raw;
        for timer in 0..TIMERS {
            if raw & (1 << timer) == 0
                && let Some(at) = self.next_ovf(timer, apb)
                && at <= now
            {
                raw |= 1 << timer;
            }
        }
        for channel in 0..CHANNELS {
            let bit = 1 << (4 + channel);
            if raw & bit == 0
                && let Some(at) = self.fade_end_at(channel, apb)
                && at <= now
            {
                raw |= bit;
            }
        }
        raw
    }

    /// Source 35's level at `now`: high while `LEDC_INT_ST_REG` is nonzero.
    pub(crate) fn line(&self, now: Tick, apb: u64) -> bool {
        (self.raw_at(now, apb) & self.int_ena()) != 0
    }

    /// The earliest tick after `now` at which source 35 can rise. `None` while the line is
    /// already high or no enabled overflow or fade-end interrupt is scheduled.
    pub(crate) fn next_int_event(&self, now: Tick, apb: u64) -> Option<Tick> {
        let ena = self.int_ena();
        if ena == 0 || (self.raw_at(now, apb) & ena) != 0 {
            return None;
        }
        let ovf = (0..TIMERS)
            .filter(|&t| ena & (1 << t) != 0)
            .filter_map(|t| self.next_ovf(t, apb))
            .min();
        let fade = (0..CHANNELS)
            .filter(|&c| ena & (1 << (4 + c)) != 0)
            .filter_map(|c| self.fade_end_at(c, apb))
            .min();
        [ovf, fade].into_iter().flatten().min()
    }

    /// Writes the register at `offset` at `now`, with `apb` ticks per APB cycle. `Ok(false)` for
    /// a register not kept, which drops the write, and `Err` names a setting the model does not
    /// run.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        offset: u32,
        value: u32,
        apb: u64,
    ) -> Result<bool, &'static str> {
        match kind(offset) {
            None => return Ok(false),
            Some(Kind::DutyRead | Kind::IntRaw | Kind::IntSt) => return Ok(true),
            Some(Kind::IntClr) => {
                self.settle(now, apb);
                self.int_raw &= !value;
                for timer in 0..TIMERS {
                    if value & (1 << timer) != 0 {
                        self.ovf_mark[timer] = now;
                    }
                }
                return Ok(true);
            }
            Some(_) => {}
        }
        self.settle(now, apb);
        let old_period = (TIMER_BASE..INT_RAW)
            .contains(&offset)
            .then(|| {
                self.period(((offset - TIMER_BASE) / TIMER) as usize, apb)
                    .ok()
            })
            .flatten();
        let old = self.reg(offset);
        self.regs[(offset / 4) as usize] = value;
        if offset < CHANNELS as u32 * CHANNEL {
            if offset.is_multiple_of(CHANNEL) && value & PARA_UP != 0 {
                self.update((offset / CHANNEL) as usize, now)?;
            }
        } else if (TIMER_BASE..INT_RAW).contains(&offset) {
            let timer = ((offset - TIMER_BASE) / TIMER) as usize;
            let rst = value & RST != 0;
            let pause = value & PAUSE != 0;
            let para_up = value & TIMER_STROBE != 0;
            let setup = value & TIMER_SETUP;
            let was = self.timers[timer];
            if !rst && matches!(was, Timer::Running { .. }) && (old ^ value) & TICK_SEL != 0 {
                return Err("tick_sel");
            }
            if rst {
                if let Timer::Running { since } = was {
                    self.save_fade_cycles(timer, now, since, old_period);
                }
                if para_up || matches!(was, Timer::Reset) {
                    self.timer_conf[timer] = setup;
                    self.timer_pending[timer] = None;
                } else if let Some(pending) = self.timer_pending[timer].take() {
                    self.timer_conf[timer] = pending.conf;
                }
                self.timers[timer] = Timer::Reset;
                self.drive_timer_reset(timer, now);
            } else if pause {
                match was {
                    Timer::Running { since } => {
                        let elapsed = self.save_fade_cycles(timer, now, since, old_period);
                        self.timers[timer] = Timer::Paused { at: now, elapsed };
                        if para_up {
                            self.latch_timer_update(timer, now, setup);
                        }
                    }
                    Timer::Reset => {
                        self.timer_conf[timer] = setup;
                        self.timer_pending[timer] = None;
                        self.timers[timer] = Timer::Paused {
                            at: now,
                            elapsed: 0,
                        };
                    }
                    Timer::Paused { at, elapsed } => {
                        if para_up {
                            if self.timer_conf[timer] == 0 {
                                self.timer_conf[timer] = setup;
                                self.timer_pending[timer] = None;
                            } else {
                                self.latch_timer_update(timer, now, setup);
                            }
                        }
                        self.timers[timer] = Timer::Paused { at, elapsed };
                    }
                }
            } else {
                match was {
                    Timer::Running { .. } => {
                        if para_up {
                            self.latch_timer_update(timer, now, setup);
                        }
                    }
                    Timer::Reset => {
                        self.timer_conf[timer] = setup;
                        self.timer_pending[timer] = None;
                        self.timers[timer] = Timer::Running { since: now };
                        self.ovf_mark[timer] = now;
                        self.start_timer(timer, now, now, None, apb);
                    }
                    Timer::Paused { at, elapsed } => {
                        if para_up {
                            if self.timer_conf[timer] == 0 {
                                self.timer_conf[timer] = setup;
                                self.timer_pending[timer] = None;
                            } else {
                                self.latch_timer_update(timer, now, setup);
                            }
                        }
                        let since = Tick(now.0 - elapsed);
                        self.timers[timer] = Timer::Running { since };
                        self.ovf_mark[timer] = now;
                        let resume_delta = (elapsed > 0).then_some(now.0 - at.0);
                        self.start_timer(timer, now, since, resume_delta, apb);
                    }
                }
            }
        } else if offset == CONF && (old ^ value) & CLK_SEL != 0 && self.counts() {
            return Err("clock_change_while_running");
        }
        self.check(apb)?;
        Ok(true)
    }

    /// Latches `setup` on `LEDC_LSTIMERn_PARA_UP` to take effect at timer `timer`'s next overflow
    /// (TRM v1.8, 35.3.2.2, 35.3.2.3 and Register 35.7).
    fn latch_timer_update(&mut self, timer: usize, now: Tick, setup: u32) {
        self.timer_pending[timer] = (setup != self.timer_conf[timer]).then_some(TimerUpdate {
            at: now,
            conf: setup,
        });
    }

    /// Saves each active fade's remaining overflow count on `timer` as it leaves `Running`, and
    /// returns how many ticks into its current period `timer` was at `now`.
    fn save_fade_cycles(
        &mut self,
        timer: usize,
        now: Tick,
        since: Tick,
        old_period: Option<(u64, u8)>,
    ) -> u64 {
        let elapsed = old_period.map_or(0, |(period, _)| (now.0 - since.0) % period);
        if let Some((period, _)) = old_period {
            let last_ovf = now.0 - elapsed;
            for fade in self.fade.iter_mut().flatten() {
                if fade.timer == timer {
                    fade.cycles_left =
                        ((fade.next_step.0.saturating_sub(last_ovf)) / period).max(1) as u32;
                }
            }
        }
        elapsed
    }

    /// Resets `timer`'s counter to 0: each enabled channel on `timer` with nonzero duty sees
    /// `Timerx_cnt == Hpoint` (0) and drives high (TRM v1.8, 35.3.3 and Register 35.7).
    fn drive_timer_reset(&mut self, timer: usize, now: Tick) {
        for ch in 0..CHANNELS {
            if let Some(applied) = self.applied[ch]
                && applied.timer == timer
                && applied.out
                && self.duty[ch] > 0
            {
                self.drive(now, ch, true);
            }
        }
    }

    /// Re-anchors channels and active fades on `timer` as it enters `Running { since }`.
    /// `resume_delta` is `Some(now - paused_at)` when resuming from `Paused` with `elapsed > 0`
    /// and an unchanged timer setup, preserving the saved phase and output level.
    fn start_timer(
        &mut self,
        timer: usize,
        now: Tick,
        since: Tick,
        resume_delta: Option<u64>,
        apb: u64,
    ) {
        let new_period = self.period(timer, apb).ok();
        for ch in 0..CHANNELS {
            if let Some(applied) = self.applied[ch]
                && applied.timer == timer
                && applied.out
                && let Some((period, resolution)) = new_period
            {
                let steps = 1u32 << resolution;
                let duty = self.duty[ch].min(steps);
                let hz = u32::try_from((TICK_HZ + period / 2) / period).expect("at most 480 MHz");
                let effect = Effect {
                    at: now,
                    channel: ch as u8,
                    duty,
                    resolution,
                    hz,
                };
                let changed = self.sent[ch].is_none_or(|prev| {
                    prev.duty != effect.duty
                        || prev.resolution != effect.resolution
                        || prev.hz != effect.hz
                });
                if changed {
                    self.sent[ch] = Some(effect);
                    self.done.push(effect);
                }
                if duty == 0 {
                    self.pwm[ch] = None;
                    self.drive(now, ch, false);
                } else if duty >= steps {
                    self.pwm[ch] = None;
                    self.drive(now, ch, true);
                } else if let (Some(delta), Some(pwm)) = (resume_delta, self.pwm[ch])
                    && pwm.period == period
                    && !changed
                {
                    self.pwm[ch] = Some(Pwm {
                        start: Tick(pwm.start.0 + delta),
                        ..pwm
                    });
                } else {
                    let high = (u64::from(duty) * period + u64::from(steps >> 1)) >> resolution;
                    self.pwm[ch] = Some(Pwm {
                        timer,
                        start: since,
                        period,
                        high,
                    });
                    self.drive(now, ch, true);
                }
            }
            if let Some(fade) = &mut self.fade[ch]
                && fade.timer == timer
                && let Some((period, _)) = new_period
            {
                fade.next_step = Tick(since.0 + u64::from(fade.cycles_left) * period);
            }
        }
    }

    /// Takes channel `channel`'s registers as its next update, from `now`.
    fn update(&mut self, channel: usize, now: Tick) -> Result<(), &'static str> {
        if self.channel(channel, 0x04) & HPOINT != 0 {
            return Err("hpoint");
        }
        let conf1 = self.channel(channel, 0x0c);
        let (duty, fade) = if conf1 & DUTY_START == 0 {
            (self.duty[channel], None)
        } else {
            let duty = self.channel(channel, 0x08) & DUTY;
            if duty & 0xf != 0 {
                return Err("duty_fraction");
            }
            let num = (conf1 >> DUTY_NUM_SHIFT) & 0x3ff;
            let cycle = (conf1 >> DUTY_CYCLE_SHIFT) & 0x3ff;
            let scale = conf1 & DUTY_SCALE;
            let fade = (num > 0 && cycle > 0).then_some(FadeConfig {
                inc: conf1 & DUTY_INC != 0,
                num,
                cycle,
                scale,
            });
            (duty >> 4, fade)
        };
        let conf0 = self.channel(channel, 0x00);
        self.pending[channel] = Some(Update {
            at: now,
            timer: (conf0 & TIMER_SEL) as usize,
            duty,
            out: conf0 & SIG_OUT_EN != 0,
            idle: conf0 & IDLE_LV != 0,
            fade,
        });
        Ok(())
    }

    /// Whether any timer counts.
    pub(crate) fn counts(&self) -> bool {
        self.timers
            .iter()
            .any(|t| matches!(t, Timer::Running { .. }))
    }

    /// Whether an update waits for its timer's overflow.
    pub(crate) fn pending(&self) -> bool {
        self.pending.iter().any(Option::is_some) || self.timer_pending.iter().any(Option::is_some)
    }

    /// Whether a channel has a live fade that still changes duty or can raise its interrupt.
    pub(crate) fn fading(&self) -> bool {
        let ena = self.int_ena();
        self.fade
            .iter()
            .enumerate()
            .any(|(ch, fade)| fade.is_some_and(|f| f.scale != 0 || ena & (1 << (4 + ch)) != 0))
    }

    /// Whether a timer counts the APB clock, so a change of the APB clock would move its period.
    pub(crate) fn counts_apb(&self) -> bool {
        self.counts() && self.reg(CONF) & CLK_SEL == 1
    }

    /// The ticks of one period for a timer configuration word, and its duty resolution.
    fn period_for_conf(&self, conf: u32, apb: u64) -> Result<(u64, u8), &'static str> {
        let clock = match self.reg(CONF) & CLK_SEL {
            1 => apb,
            3 => XTAL,
            _ => return Err("clock"),
        };
        if conf & TICK_SEL != 0 {
            return Err("tick_sel");
        }
        let divider = u64::from((conf & CLK_DIV) >> 4);
        if divider < 256 {
            return Err("clock_divider");
        }
        let resolution = (conf & DUTY_RES) as u8;
        if resolution == 0 || resolution > MAX_RESOLUTION {
            return Err("duty_resolution");
        }
        let ticks = (divider << resolution) * clock;
        if !ticks.is_multiple_of(256) {
            return Err("fractional_divider");
        }
        Ok((ticks / 256, resolution))
    }

    /// The ticks of one active period of timer `timer`, and its active duty resolution.
    fn period(&self, timer: usize, apb: u64) -> Result<(u64, u8), &'static str> {
        self.period_for_conf(self.timer_conf[timer], apb)
    }

    /// Refuses an update or fade the model cannot time or send: one whose timer counts with no
    /// period the model runs, or with a duty or fade target at or above `2^14` (which overflows
    /// the 14-bit counter comparator) or below 0. A duty of `2^14` at 14 bits is refused as
    /// `full_duty`: IDF's `ledc_channel_config` notes that it messes up the hardware's duty, and
    /// does not say what the pad then drives. A duty in `2^DUTY_RES..2^14` on a narrower timer
    /// stays high throughout because `Timerx_cnt` never reaches `Lpoint` (TRM 35.3.3).
    fn check(&self, apb: u64) -> Result<(), &'static str> {
        let check_duty = |duty: u32, resolution: u8| -> Result<(), &'static str> {
            let limit = 1u32 << MAX_RESOLUTION;
            if resolution == MAX_RESOLUTION && duty == limit {
                return Err("full_duty");
            }
            if duty >= limit {
                return Err("duty_range");
            }
            Ok(())
        };
        for timer in 0..TIMERS {
            if self.running_since(timer).is_none() {
                continue;
            }
            if let Some(pending) = self.timer_pending[timer] {
                let (_, resolution) = self.period_for_conf(pending.conf, apb)?;
                for ch in 0..CHANNELS {
                    if let Some(applied) = self.applied[ch]
                        && applied.timer == timer
                    {
                        check_duty(self.duty[ch], resolution)?;
                    }
                }
            }
            if self
                .applied
                .iter()
                .flatten()
                .any(|applied| applied.timer == timer)
            {
                let (_, resolution) = self.period(timer, apb)?;
                for ch in 0..CHANNELS {
                    if let Some(applied) = self.applied[ch]
                        && applied.timer == timer
                    {
                        check_duty(self.duty[ch], resolution)?;
                    }
                }
            }
        }
        for update in self.pending.iter().flatten() {
            if self.running_since(update.timer).is_none() {
                continue;
            }
            let (_, resolution) = match self.timer_pending[update.timer] {
                Some(pending) => self.period_for_conf(pending.conf, apb)?,
                None => self.period(update.timer, apb)?,
            };
            check_duty(update.duty, resolution)?;
            if let Some(fade) = update.fade {
                let delta = fade.num * fade.scale;
                let end = if fade.inc {
                    update.duty.checked_add(delta).ok_or("duty_range")?
                } else {
                    update.duty.checked_sub(delta).ok_or("duty_range")?
                };
                check_duty(end, resolution)?;
            }
        }
        for (ch, fade) in self.fade.iter().enumerate() {
            let Some(fade) = fade else {
                continue;
            };
            if self.running_since(fade.timer).is_none() {
                continue;
            }
            let (_, resolution) = self.period(fade.timer, apb)?;
            let delta = fade.remaining * fade.scale;
            let end = if fade.inc {
                self.duty[ch].checked_add(delta).ok_or("duty_range")?
            } else {
                self.duty[ch].checked_sub(delta).ok_or("duty_range")?
            };
            check_duty(end, resolution)?;
            if let Some(pending) = self.timer_pending[fade.timer] {
                let (_, new_res) = self.period_for_conf(pending.conf, apb)?;
                check_duty(end, new_res)?;
            }
        }
        Ok(())
    }

    /// The tick timer `timer`'s pending `PARA_UP` update takes effect, at its next overflow.
    fn timer_due(&self, timer: usize, apb: u64) -> Option<Tick> {
        let update = self.timer_pending[timer]?;
        let since = self.running_since(timer)?;
        let (period, _) = self.period(timer, apb).ok()?;
        let cycles = update.at.0.saturating_sub(since.0) / period + 1;
        Some(Tick(since.0 + cycles * period))
    }

    /// The tick channel `channel`'s update takes effect, once its timer counts.
    fn due(&self, channel: usize, apb: u64) -> Option<Tick> {
        let update = self.pending[channel]?;
        let since = self.running_since(update.timer)?;
        let (period, _) = self.period(update.timer, apb).ok()?;
        let cycles = update.at.0.saturating_sub(since.0) / period + 1;
        Some(Tick(since.0 + cycles * period))
    }

    /// The next active fade step across all channels. When `scaled_only` is true, ignores
    /// `scale == 0` steps that change no duty or PWM edge (their interrupt is woken through
    /// [`Self::next_int_event`]).
    fn next_fade_step(&self, scaled_only: bool) -> Option<(Tick, usize)> {
        (0..CHANNELS)
            .filter_map(|k| {
                let fade = self.fade[k]?;
                if scaled_only && fade.scale == 0 {
                    return None;
                }
                self.running_since(fade.timer)?;
                Some((fade.next_step, k))
            })
            .min()
    }

    /// The next change of a channel's signal from its PWM, as the tick, the channel and the
    /// level. A PWM whose timer does not count makes none.
    fn next_edge(&self) -> Option<(Tick, usize, bool)> {
        (0..CHANNELS)
            .filter_map(|k| {
                let pwm = self.pwm[k]?;
                self.running_since(pwm.timer)?;
                Some(if self.level[k] {
                    (Tick(pwm.start.0 + pwm.high), k, false)
                } else {
                    (Tick(pwm.start.0 + pwm.period), k, true)
                })
            })
            .min()
    }

    /// The tick of the next timer update, channel update, scaled fade step, or PWM edge to take
    /// effect.
    pub(crate) fn next(&self, apb: u64) -> Option<Tick> {
        let timer = (0..TIMERS).filter_map(|t| self.timer_due(t, apb)).min();
        let update = (0..CHANNELS).filter_map(|k| self.due(k, apb)).min();
        let fade = self.next_fade_step(true).map(|(at, _)| at);
        let edge = self.next_edge().map(|(at, ..)| at);
        [timer, update, fade, edge].into_iter().flatten().min()
    }

    /// Lets every timer update, channel update, fade step, and PWM edge due at or before
    /// `through` take effect, in time order, and latches timer overflows through `through`. On a
    /// tie at an overflow, a pending timer update runs first (taking any channel update due at
    /// that same overflow with the new period), then a channel update (replacing any active fade),
    /// then a fade step, then a PWM edge.
    pub(crate) fn settle(&mut self, through: Tick, apb: u64) {
        loop {
            let timer = (0..TIMERS)
                .filter_map(|t| Some((self.timer_due(t, apb)?, t)))
                .min()
                .filter(|&(at, _)| at <= through);
            let update = (0..CHANNELS)
                .filter_map(|k| Some((self.due(k, apb)?, k)))
                .min()
                .filter(|&(at, _)| at <= through);
            let fade = self.next_fade_step(false).filter(|&(at, _)| at <= through);
            let edge = self.next_edge().filter(|&(at, ..)| at <= through);
            let earliest = [
                timer.map(|(at, _)| (at, 0u8)),
                update.map(|(at, _)| (at, 1u8)),
                fade.map(|(at, _)| (at, 2u8)),
                edge.map(|(at, ..)| (at, 3u8)),
            ]
            .into_iter()
            .flatten()
            .min();
            match earliest {
                Some((_, 0)) => {
                    let (at, timer) = timer.expect("timer update");
                    self.take_timer_update(at, timer, apb);
                }
                Some((_, 1)) => {
                    let (at, channel) = update.expect("update");
                    self.take_update(at, channel, apb);
                }
                Some((_, 2)) => {
                    let (at, channel) = fade.expect("fade");
                    self.take_fade_step(at, channel, apb);
                }
                Some((_, 3)) => {
                    let (at, channel, level) = edge.expect("edge");
                    if level {
                        let pwm = self.pwm[channel].as_mut().expect("a PWM");
                        pwm.start = at;
                    }
                    self.drive(at, channel, level);
                }
                _ => break,
            }
        }
        for timer in 0..TIMERS {
            if let Some(at) = self.next_ovf(timer, apb)
                && at <= through
            {
                self.int_raw |= 1 << timer;
                self.ovf_mark[timer] = through;
            }
        }
    }

    /// Timer `timer`'s pending `PARA_UP` update takes effect at `at`, an overflow of `timer`.
    fn take_timer_update(&mut self, at: Tick, timer: usize, apb: u64) {
        let update = self.timer_pending[timer].take().expect("timer due");
        let (old_period, _) = self.period(timer, apb).expect("checked");
        if self.next_ovf(timer, apb).is_some_and(|ovf| ovf <= at) {
            self.int_raw |= 1 << timer;
            self.ovf_mark[timer] = at;
        }
        self.timer_conf[timer] = update.conf;
        self.timers[timer] = Timer::Running { since: at };
        let (new_period, new_res) = self.period(timer, apb).expect("checked");
        for ch in 0..CHANNELS {
            if self.pending[ch].is_some_and(|u| u.timer == timer && u.at < at) {
                self.take_update(at, ch, apb);
                continue;
            }
            if let Some(fade) = &mut self.fade[ch]
                && fade.timer == timer
                && fade.next_step > at
            {
                let cycles_left = ((fade.next_step.0 - at.0) / old_period).max(1) as u32;
                fade.next_step = Tick(at.0 + u64::from(cycles_left) * new_period);
                fade.cycles_left = cycles_left;
            }
            let has_fade_now = self.fade[ch]
                .is_some_and(|f| f.timer == timer && f.scale != 0 && f.next_step == at);
            if !has_fade_now
                && let Some(applied) = self.applied[ch]
                && applied.timer == timer
                && applied.out
            {
                self.apply_duty(
                    at,
                    ch,
                    timer,
                    self.duty[ch],
                    applied.out,
                    applied.idle,
                    new_period,
                    new_res,
                );
            }
        }
    }

    /// Applies `raw_duty` (with `out` and `idle`) to channel `channel` at `at`, updating its
    /// `Effect`, `Pwm` and signal level.
    #[allow(clippy::too_many_arguments)]
    fn apply_duty(
        &mut self,
        at: Tick,
        channel: usize,
        timer: usize,
        raw_duty: u32,
        out: bool,
        idle: bool,
        period: u64,
        resolution: u8,
    ) {
        let steps = 1 << resolution;
        let duty = match (out, idle) {
            (true, _) => raw_duty.min(steps),
            (false, true) => steps,
            (false, false) => 0,
        };
        let effect = Effect {
            at,
            channel: channel as u8,
            duty,
            resolution,
            hz: u32::try_from((TICK_HZ + period / 2) / period).expect("at most 480 MHz"),
        };
        self.sent[channel] = Some(effect);
        self.done.push(effect);
        self.pwm[channel] = (duty > 0 && duty < steps).then(|| Pwm {
            timer,
            start: at,
            period,
            high: (u64::from(duty) * period + u64::from(steps >> 1)) >> resolution,
        });
        self.drive(at, channel, duty > 0);
    }

    /// Channel `channel`'s pending update takes effect at `at`, an overflow of its timer.
    fn take_update(&mut self, at: Tick, channel: usize, apb: u64) {
        let update = self.pending[channel].take().expect("due");
        let (period, resolution) = self.period(update.timer, apb).expect("checked");
        self.applied[channel] = Some(Applied {
            timer: update.timer,
            out: update.out,
            idle: update.idle,
        });
        self.duty[channel] = update.duty;
        self.apply_duty(
            at,
            channel,
            update.timer,
            update.duty,
            update.out,
            update.idle,
            period,
            resolution,
        );
        self.fade[channel] = update.fade.map(|cfg| ActiveFade {
            timer: update.timer,
            out: update.out,
            idle: update.idle,
            inc: cfg.inc,
            scale: cfg.scale,
            cycle: cfg.cycle,
            remaining: cfg.num,
            next_step: Tick(at.0 + u64::from(cfg.cycle) * period),
            cycles_left: cfg.cycle,
        });
    }

    /// Channel `channel`'s active fade executes one step at `at`, an overflow of its timer.
    fn take_fade_step(&mut self, at: Tick, channel: usize, apb: u64) {
        let mut fade = self.fade[channel].take().expect("active fade");
        let (period, resolution) = self.period(fade.timer, apb).expect("checked");
        fade.remaining -= 1;
        if fade.scale != 0 {
            if fade.inc {
                self.duty[channel] += fade.scale;
            } else {
                self.duty[channel] -= fade.scale;
            }
            self.apply_duty(
                at,
                channel,
                fade.timer,
                self.duty[channel],
                fade.out,
                fade.idle,
                period,
                resolution,
            );
        }
        if fade.remaining == 0 {
            self.int_raw |= 1 << (4 + channel);
        } else {
            fade.next_step = Tick(at.0 + u64::from(fade.cycle) * period);
            fade.cycles_left = fade.cycle;
            self.fade[channel] = Some(fade);
        }
    }

    /// Puts channel `channel`'s signal at `level` from `at`, and notes the change.
    fn drive(&mut self, at: Tick, channel: usize, level: bool) {
        if self.level[channel] != level {
            self.level[channel] = level;
            self.edges.push(Edge {
                at,
                channel: channel as u8,
                level,
            });
        }
    }

    /// What channel `channel` sends, once an update took effect on it.
    pub(crate) fn sent(&self, channel: usize) -> Option<Effect> {
        self.sent[channel]
    }

    /// Whether any settled updates or signal edges wait for the chip to take them.
    pub(crate) fn has_output(&self) -> bool {
        !self.done.is_empty() || !self.edges.is_empty()
    }

    /// The earliest tick of any settled update or signal edge waiting to be taken.
    pub(crate) fn first_output(&self) -> Option<Tick> {
        [
            self.done.first().map(|e| e.at),
            self.edges.first().map(|e| e.at),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The updates that took effect at or before `through`, in the order they did.
    pub(crate) fn take_done_through(&mut self, through: Tick) -> Vec<Effect> {
        let count = self.done.partition_point(|e| e.at <= through);
        self.done.drain(..count).collect()
    }

    /// The changes of the channels' signals at or before `through`, in time order.
    pub(crate) fn take_edges_through(&mut self, through: Tick) -> Vec<Edge> {
        let count = self.edges.partition_point(|e| e.at <= through);
        self.edges.drain(..count).collect()
    }

    /// The updates that took effect since the last call, in the order they did.
    #[cfg(test)]
    pub(crate) fn take_done(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.done)
    }

    /// The changes of the channels' signals since the last call, in time order.
    #[cfg(test)]
    pub(crate) fn take_edges(&mut self) -> Vec<Edge> {
        std::mem::take(&mut self.edges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks per APB cycle at 80 MHz.
    const APB: u64 = 6;

    /// Timer 0 on the APB clock at `divider` and `resolution`, out of reset from `at`.
    fn timer(ledc: &mut Ledc, at: Tick, divider: u32, resolution: u32) {
        ledc.write(at, CONF, 1, APB).unwrap();
        let conf = divider << 4 | resolution;
        ledc.write(at, TIMER_BASE, conf | RST | TIMER_STROBE, APB)
            .unwrap();
        ledc.write(at, TIMER_BASE, conf, APB).unwrap();
    }

    /// Channel 0 sets `duty` and sends its update at `at`, as `ledc_set_duty` and
    /// `ledc_update_duty` write it.
    fn duty(ledc: &mut Ledc, at: Tick, duty: u32) -> Result<bool, &'static str> {
        ledc.write(at, 0x08, duty << 4, APB)?;
        ledc.write(at, 0x0c, DUTY_START | 1 << 30 | 1 << 20 | 1 << 10, APB)?;
        ledc.write(at, 0x00, SIG_OUT_EN | PARA_UP, APB)
    }

    #[test]
    fn a_channel_and_a_timer_keep_their_setup_and_their_update_bits_read_0() {
        let mut ledc = Ledc::new();
        assert_eq!(ledc.read(Tick(0), 0x0c, APB), Some(CONF1_RESET));
        assert_eq!(ledc.read(Tick(0), TIMER_BASE, APB), Some(TIMER_RESET));
        assert_eq!(ledc.read(Tick(0), DATE, APB), Some(DATE_RESET));
        assert_eq!(ledc.write(Tick(0), 0x14 + 0x08, 4095 << 4, APB), Ok(true));
        assert_eq!(ledc.write(Tick(0), 0x14, 1 << 2 | 1 << 4, APB), Ok(true));
        let conf = TIMER_STROBE | RST | 12;
        assert_eq!(ledc.write(Tick(0), TIMER_BASE + TIMER, conf, APB), Ok(true));
        assert_eq!(ledc.read(Tick(0), 0x14 + 0x08, APB), Some(4095 << 4));
        assert_eq!(ledc.read(Tick(0), 0x14, APB), Some(1 << 2));
        assert_eq!(ledc.read(Tick(0), TIMER_BASE + TIMER, APB), Some(RST | 12));
        assert_eq!(ledc.write(Tick(0), INT_CLR, u32::MAX, APB), Ok(true));
        assert_eq!(ledc.read(Tick(0), INT_ST, APB), Some(0));
        assert_eq!(ledc.read(Tick(0), INT_CLR, APB), Some(0));
        assert_eq!(ledc.read(Tick(0), 0x10, APB), Some(0), "DUTY_R");
        assert_eq!(ledc.read(Tick(0), TIMER_BASE + 4, APB), None, "VALUE");
        assert_eq!(ledc.write(Tick(0), TIMER_BASE + 4, 1, APB), Ok(false));
    }

    /// ESP-IDF's 5 kHz at 12 bits from the 80 MHz APB clock: a divider of 1000, so a period of
    /// 1000 / 256 × 4096 = 16,000 APB cycles, 200 µs.
    #[test]
    fn an_update_takes_effect_at_its_timers_next_overflow() {
        let mut ledc = Ledc::new();
        let start = Tick(600);
        timer(&mut ledc, start, 1000, 12);
        let period = 16_000 * APB;
        let at = Tick(start.0 + 5 * period + 7);
        duty(&mut ledc, at, 4095).unwrap();
        let due = Tick(start.0 + 6 * period);
        assert_eq!(ledc.next(APB), Some(due));
        ledc.settle(Tick(due.0 - 1), APB);
        assert_eq!(ledc.take_done(), []);
        ledc.settle(due, APB);
        assert_eq!(
            ledc.take_done(),
            [Effect {
                at: due,
                channel: 0,
                duty: 4095,
                resolution: 12,
                hz: 5000,
            }]
        );
        assert_eq!(
            ledc.next(APB),
            Some(Tick(due.0 + 95_977)),
            "the fall at 4095 of 4096 steps of 96,000 ticks"
        );
    }

    /// A divider of 1.5 at 1 bit is 18 ticks a period, 26,666,666.7 Hz, sent as 26,666,667.
    #[test]
    fn the_frequency_is_the_tick_rate_over_the_period_rounded() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 384, 1);
        duty(&mut ledc, Tick(0), 1).unwrap();
        ledc.settle(Tick(18), APB);
        let hz: Vec<u32> = ledc.take_done().iter().map(|effect| effect.hz).collect();
        assert_eq!(hz, [26_666_667]);
    }

    #[test]
    fn a_later_update_before_the_overflow_replaces_the_first() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(10), 8).unwrap();
        duty(&mut ledc, Tick(20), 3).unwrap();
        ledc.settle(Tick(16 * APB), APB);
        let done = ledc.take_done();
        assert_eq!(done.len(), 1, "{done:?}");
        assert_eq!((done[0].at, done[0].duty), (Tick(16 * APB), 3));
    }

    #[test]
    fn an_update_on_a_timer_in_reset_waits_until_it_counts() {
        let mut ledc = Ledc::new();
        ledc.write(Tick(0), CONF, 3, APB).unwrap();
        ledc.write(Tick(0), TIMER_BASE, 256 << 4 | 1 | RST, APB)
            .unwrap();
        duty(&mut ledc, Tick(5), 1).unwrap();
        assert_eq!(ledc.next(APB), None);
        ledc.write(Tick(100), TIMER_BASE, 256 << 4 | 1, APB)
            .unwrap();
        assert_eq!(
            ledc.next(APB),
            Some(Tick(100 + 2 * XTAL)),
            "two crystal cycles"
        );
    }

    /// `ledc_stop` stores the idle level, a clear `LEDC_SIG_OUT_EN_LSCHn` and the update bit,
    /// with `LEDC_DUTY_START_LSCHn` still set from `ledc_update_duty`.
    #[test]
    fn a_stopped_channel_sends_its_idle_level() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 5).unwrap();
        ledc.write(Tick(200), 0x00, IDLE_LV | PARA_UP, APB).unwrap();
        ledc.write(Tick(400), 0x00, SIG_OUT_EN | PARA_UP, APB)
            .unwrap();
        ledc.settle(Tick(10_000), APB);
        let duties: Vec<u32> = ledc.take_done().iter().map(|e| e.duty).collect();
        assert_eq!(duties, [5, 16, 5]);
    }

    #[test]
    fn an_update_with_duty_start_clear_keeps_the_last_duty() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 5).unwrap();
        ledc.write(Tick(200), 0x08, 9 << 4, APB).unwrap();
        ledc.write(Tick(200), 0x0c, 1 << 30, APB).unwrap();
        ledc.write(Tick(200), 0x00, SIG_OUT_EN | PARA_UP, APB)
            .unwrap();
        ledc.settle(Tick(10_000), APB);
        let duties: Vec<u32> = ledc.take_done().iter().map(|e| e.duty).collect();
        assert_eq!(duties, [5, 5], "the duty register's 9 is not taken");
    }

    /// `LEDC_DUTY_LSCHn` is bits 18:0, so bit 19 and above are not duty.
    #[test]
    fn the_duty_is_its_field() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 3 | 1 << 15).unwrap();
        ledc.settle(Tick(10_000), APB);
        let duties: Vec<u32> = ledc.take_done().iter().map(|e| e.duty).collect();
        assert_eq!(duties, [3]);
    }

    #[test]
    fn a_setting_the_model_does_not_run_is_refused() {
        let refused = |setup: &dyn Fn(&mut Ledc) -> Result<bool, &'static str>| {
            let mut ledc = Ledc::new();
            timer(&mut ledc, Tick(0), 256, 4);
            setup(&mut ledc).unwrap_err()
        };
        assert_eq!(
            refused(&|l| {
                l.write(Tick(1), 0x04, 3, APB)?;
                duty(l, Tick(1), 1)
            }),
            "hpoint"
        );
        assert_eq!(
            refused(&|l| {
                l.write(Tick(1), 0x08, 1 << 4 | 8, APB)?;
                l.write(Tick(1), 0x0c, DUTY_START, APB)?;
                l.write(Tick(1), 0x00, PARA_UP, APB)
            }),
            "duty_fraction"
        );
        assert_eq!(refused(&|l| duty(l, Tick(1), 1 << 14)), "duty_range");
        assert_eq!(
            refused(&|l| {
                l.write(Tick(1), 0x08, ((1 << 14) - 4) << 4, APB)?;
                l.write(
                    Tick(1),
                    0x0c,
                    DUTY_START | DUTY_INC | 4 << 20 | 1 << 10 | 2,
                    APB,
                )?;
                l.write(Tick(1), 0x00, SIG_OUT_EN | PARA_UP, APB)
            }),
            "duty_range",
            "fade up past 2^14"
        );
        assert_eq!(
            refused(&|l| {
                l.write(Tick(1), 0x08, 5 << 4, APB)?;
                l.write(Tick(1), 0x0c, DUTY_START | 3 << 20 | 1 << 10 | 2, APB)?;
                l.write(Tick(1), 0x00, SIG_OUT_EN | PARA_UP, APB)
            }),
            "duty_range",
            "fade down below 0"
        );
        assert_eq!(
            refused(&|l| l.write(Tick(1), TIMER_BASE, 256 << 4 | 4 | TICK_SEL, APB)),
            "tick_sel"
        );
        assert_eq!(
            refused(&|l| l.write(Tick(1), TIMER_BASE, 255 << 4 | 4 | TIMER_STROBE, APB)),
            "clock_divider"
        );
        assert_eq!(
            refused(&|l| l.write(Tick(1), TIMER_BASE, 256 << 4 | TIMER_STROBE, APB)),
            "duty_resolution"
        );
        assert_eq!(
            refused(&|l| l.write(Tick(1), TIMER_BASE, 300 << 4 | 1 | TIMER_STROBE, APB)),
            "fractional_divider"
        );
        assert_eq!(
            refused(&|l| l.write(Tick(1), CONF, 2, APB)),
            "clock_change_while_running"
        );
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 300, 1);
        assert_eq!(duty(&mut ledc, Tick(1), 1), Err("fractional_divider"));
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 255, 4);
        assert_eq!(duty(&mut ledc, Tick(1), 1), Err("clock_divider"));
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 0);
        assert_eq!(duty(&mut ledc, Tick(1), 0), Err("duty_resolution"));
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 15);
        assert_eq!(duty(&mut ledc, Tick(1), 0), Err("duty_resolution"));
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 14);
        assert_eq!(duty(&mut ledc, Tick(1), 1 << 14), Err("full_duty"));
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 14);
        ledc.write(Tick(1), 0x08, ((1 << 14) - 2) << 4, APB)
            .unwrap();
        ledc.write(
            Tick(1),
            0x0c,
            DUTY_START | DUTY_INC | 1 << 20 | 1 << 10 | 2,
            APB,
        )
        .unwrap();
        assert_eq!(
            ledc.write(Tick(1), 0x00, SIG_OUT_EN | PARA_UP, APB),
            Err("full_duty"),
            "fade up to 2^14"
        );
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 13);
        assert_eq!(
            duty(&mut ledc, Tick(1), 1 << 13),
            Ok(true),
            "full duty below 14 bits"
        );
        let mut ledc = Ledc::new();
        ledc.write(Tick(0), CONF, 1, APB).unwrap();
        ledc.write(Tick(0), TIMER_BASE, 256 << 4 | 4 | TICK_SEL, APB)
            .unwrap();
        assert_eq!(duty(&mut ledc, Tick(1), 1), Err("tick_sel"));
        let mut ledc = Ledc::new();
        ledc.write(Tick(0), TIMER_BASE, 256 << 4 | 4, APB).unwrap();
        assert_eq!(duty(&mut ledc, Tick(1), 1), Err("clock"), "no clock picked");
    }

    /// `LEDC_LSCH0_DUTY_R_REG` reads the duty in force: 0 at reset, the old duty until the
    /// update's overflow and the new one from it, with the fraction bits clear. A store to it
    /// keeps nothing.
    #[test]
    fn duty_r_reads_the_duty_in_force() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        let period = 16 * APB;
        assert_eq!(ledc.read(Tick(0), 0x10, APB), Some(0));
        duty(&mut ledc, Tick(1), 5).unwrap();
        assert_eq!(ledc.read(Tick(period - 1), 0x10, APB), Some(0));
        assert_eq!(ledc.read(Tick(period), 0x10, APB), Some(5 << 4));
        ledc.settle(Tick(period), APB);
        duty(&mut ledc, Tick(period + 1), 9).unwrap();
        assert_eq!(ledc.read(Tick(2 * period - 1), 0x10, APB), Some(5 << 4));
        assert_eq!(ledc.write(Tick(period + 2), 0x10, 3 << 4, APB), Ok(true));
        assert_eq!(ledc.read(Tick(2 * period), 0x10, APB), Some(9 << 4));
        assert_eq!(
            ledc.read(Tick(2 * period), 0x14 + 0x10, APB),
            Some(0),
            "channel 1"
        );
    }

    fn edges(ledc: &mut Ledc, through: u64) -> Vec<(u64, bool)> {
        ledc.settle(Tick(through), APB);
        ledc.take_edges()
            .iter()
            .map(|edge| (edge.at.0, edge.level))
            .collect()
    }

    /// TRM 35.3.3: high at each overflow, where the counter is `Hpoint` 0, and low at the duty.
    /// Duty 5 of 16 steps of 6 ticks: high for 30 of each 96 ticks.
    #[test]
    fn a_partial_duty_is_high_from_each_overflow_for_its_steps() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 5).unwrap();
        assert_eq!(ledc.next(APB), Some(Tick(96)));
        assert_eq!(
            edges(&mut ledc, 300),
            [
                (96, true),
                (126, false),
                (192, true),
                (222, false),
                (288, true)
            ]
        );
        assert_eq!(ledc.next(APB), Some(Tick(318)));
    }

    /// A divider of 264 at 5 bits is 198 ticks a period, so a step is 6.1875 ticks. Duty 3 is
    /// 18.5625 ticks high, which rounds to 19.
    #[test]
    fn a_high_time_that_is_not_whole_ticks_rounds_to_the_nearest() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 264, 5);
        duty(&mut ledc, Tick(1), 3).unwrap();
        assert_eq!(edges(&mut ledc, 198 + 19), [(198, true), (217, false)]);
    }

    /// An update due at the overflow of a rising edge starts its own cycle there, and the edges
    /// after it follow the new duty.
    #[test]
    fn an_update_at_an_overflow_starts_the_new_duty_there() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 5).unwrap();
        duty(&mut ledc, Tick(100), 10).unwrap();
        assert_eq!(
            edges(&mut ledc, 288),
            [
                (96, true),
                (126, false),
                (192, true),
                (252, false),
                (288, true)
            ]
        );
        duty(&mut ledc, Tick(290), 0).unwrap();
        assert_eq!(edges(&mut ledc, 1000), [(348, false)]);
        assert_eq!(ledc.next(APB), None, "duty 0 has no edge");
    }

    /// Every step, an idle level and a stopped output hold one level and have no edge.
    #[test]
    fn a_duty_of_every_step_or_an_idle_level_holds_its_level() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 16).unwrap();
        assert_eq!(edges(&mut ledc, 1000), [(96, true)]);
        assert_eq!(ledc.next(APB), None);
        duty(&mut ledc, Tick(1001), 5).unwrap();
        ledc.write(Tick(1002), 0x00, PARA_UP, APB).unwrap();
        assert_eq!(edges(&mut ledc, 2000), [(1056, false)], "idle low");
        assert_eq!(ledc.next(APB), None);
    }

    /// Setting `LEDC_LSTIMERn_RST` while a channel PWM runs resets the counter to 0, drives the
    /// channel high (`Timerx_cnt == Hpoint == 0`, TRM 35.3.3) with no further edges while `RST`
    /// stays set, and restarts the PWM cycle from 0 when `RST` clears.
    #[test]
    fn a_timer_reset_holds_counter_at_zero_and_restarts_pwm_on_release() {
        let mut ledc = Ledc::new();
        let conf = 256 << 4 | 4;
        // Period = 96 ticks, duty 5 -> high for 30 ticks [96, 126), low [126, 192).
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 5).unwrap();
        assert_eq!(edges(&mut ledc, 150), [(96, true), (126, false)]);
        // Assert RST at tick 150 while output is low: counter resets to 0 -> signal goes high.
        ledc.write(Tick(150), TIMER_BASE, conf | RST, APB).unwrap();
        assert!(!ledc.counts());
        assert_eq!(ledc.next(APB), None);
        assert_eq!(edges(&mut ledc, 400), [(150, true)]);
        // Clear RST at tick 400: cycle restarts at 400 -> fall at 430, rise at 496, fall at 526.
        ledc.write(Tick(400), TIMER_BASE, conf, APB).unwrap();
        assert!(ledc.counts());
        assert_eq!(ledc.next(APB), Some(Tick(430)));
        assert_eq!(
            edges(&mut ledc, 530),
            [(430, false), (496, true), (526, false)]
        );
    }

    /// `RST` overrides `PAUSE`: resetting a paused timer clears its saved phase and drives an
    /// active PWM channel high (`Timerx_cnt == 0`). Clearing `RST` while `PAUSE` stays set leaves
    /// the counter suspended at 0 until `PAUSE` clears.
    #[test]
    fn timer_reset_overrides_pause_and_resumes_from_zero() {
        let mut ledc = Ledc::new();
        let conf = 256 << 4 | 4;
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 5).unwrap();
        // Period 96, high [96, 126), low [126, 192). Pause in the low half at tick 140.
        assert_eq!(edges(&mut ledc, 140), [(96, true), (126, false)]);
        ledc.write(Tick(140), TIMER_BASE, conf | PAUSE, APB)
            .unwrap();
        assert_eq!(edges(&mut ledc, 200), []);
        // Set RST while PAUSE is set -> counter resets to 0, output goes high.
        ledc.write(Tick(200), TIMER_BASE, conf | PAUSE | RST, APB)
            .unwrap();
        assert_eq!(edges(&mut ledc, 250), [(200, true)]);
        // Clear RST with PAUSE still set -> stays suspended at 0, no edges.
        ledc.write(Tick(250), TIMER_BASE, conf | PAUSE, APB)
            .unwrap();
        assert!(!ledc.counts());
        assert_eq!(edges(&mut ledc, 300), []);
        // Clear PAUSE at tick 300 -> starts from 0: fall at 330, next rise at 396.
        ledc.write(Tick(300), TIMER_BASE, conf, APB).unwrap();
        assert_eq!(edges(&mut ledc, 400), [(330, false), (396, true)]);
    }

    /// Setting `RST` mid-fade stalls the fade and suppresses `DUTY_CHNG_END` while `RST` is set;
    /// releasing `RST` (even with a new period) re-anchors the remaining fade overflow cycles from
    /// the release tick.
    #[test]
    fn timer_reset_pauses_a_running_fade_and_reanchors_it_on_release() {
        let mut ledc = Ledc::new();
        let conf = 256 << 4 | 4;
        // Period = 96 ticks. Fade: start duty 2, +4 every 2 overflows for 3 steps -> 2, 6, 10, 14.
        timer(&mut ledc, Tick(0), 256, 4);
        ledc.write(Tick(1), INT_ENA, 1 << 4, APB).unwrap();
        ledc.write(Tick(1), 0x08, 2 << 4, APB).unwrap();
        ledc.write(
            Tick(1),
            0x0c,
            DUTY_START | DUTY_INC | 3 << 20 | 2 << 10 | 4,
            APB,
        )
        .unwrap();
        ledc.write(Tick(1), 0x00, SIG_OUT_EN | PARA_UP, APB)
            .unwrap();
        // Initial load at 96 (duty 2), first step at 288 (duty 6), next step due at 480 (after
        // overflows at 384 and 480). Reset at tick 400 (after overflow 384, so 1 overflow left!).
        ledc.settle(Tick(400), APB);
        assert_eq!(ledc.read(Tick(400), 0x10, APB), Some(6 << 4));
        ledc.write(Tick(400), TIMER_BASE, conf | RST, APB).unwrap();
        assert_eq!(ledc.next_int_event(Tick(400), APB), None);
        assert!(!ledc.line(Tick(2000), APB));
        // Double the period (divider 512 -> period 192 ticks) while in reset, then release at 1000.
        // 1 overflow left to step 2 -> at 1000 + 192 = 1192 (duty 10);
        // 2 more overflows to final step 3 -> at 1192 + 2*192 = 1576 (duty 14, DUTY_CHNG_END).
        let slow = 512 << 4 | 4;
        ledc.write(Tick(1000), TIMER_BASE, slow, APB).unwrap();
        assert_eq!(ledc.next_int_event(Tick(1000), APB), Some(Tick(1576)));
        assert_eq!(ledc.read(Tick(1191), 0x10, APB), Some(6 << 4));
        assert_eq!(ledc.read(Tick(1192), 0x10, APB), Some(10 << 4));
        assert!(!ledc.line(Tick(1575), APB));
        assert!(ledc.line(Tick(1576), APB));
        assert_eq!(ledc.read(Tick(1576), 0x10, APB), Some(14 << 4));
    }

    /// A duty in `2^DUTY_RES..2^14` (as `ledcAttach` writes after reading an older higher-bit
    /// `DUTY_R`) holds the output high throughout while `DUTY_R` keeps the written value.
    #[test]
    fn a_duty_above_timer_resolution_below_14_bits_holds_high() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 8);
        duty(&mut ledc, Tick(1), 512).unwrap();
        let period = 256 * APB;
        assert_eq!(edges(&mut ledc, period * 3), [(period, true)]);
        assert_eq!(ledc.next(APB), None, "constant high has no PWM edge");
        assert_eq!(ledc.read(Tick(period * 3), 0x10, APB), Some(512 << 4));
        let done = ledc.take_done();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].duty, 256);
    }

    #[test]
    fn a_reset_timer_stops_and_counts_again_from_its_release() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 2).unwrap();
        ledc.write(Tick(50), TIMER_BASE, 256 << 4 | 4 | RST, APB)
            .unwrap();
        assert_eq!(ledc.next(APB), None);
        ledc.write(Tick(70), TIMER_BASE, 256 << 4 | 4, APB).unwrap();
        assert_eq!(ledc.next(APB), Some(Tick(70 + 16 * APB)));
    }

    #[test]
    fn a_paused_timer_holds_its_level_and_resumes_from_its_saved_phase() {
        let mut ledc = Ledc::new();
        let conf = 256 << 4 | 4;
        timer(&mut ledc, Tick(0), 256, 4);
        // Period = 96 ticks, duty 5 -> high for 30 ticks [96, 126), low [126, 192).
        duty(&mut ledc, Tick(1), 5).unwrap();
        assert_eq!(edges(&mut ledc, 110), [(96, true)]);
        // Pause at tick 110 (14 ticks into the 30-tick high pulse).
        ledc.write(Tick(110), TIMER_BASE, conf | PAUSE, APB)
            .unwrap();
        assert!(!ledc.counts());
        assert_eq!(ledc.next(APB), None);
        assert_eq!(edges(&mut ledc, 500), []);
        // Resume at tick 500: 16 ticks of high left -> fall at 516, next rise at 500 + 82 = 582.
        ledc.write(Tick(500), TIMER_BASE, conf, APB).unwrap();
        assert!(ledc.counts());
        assert_eq!(ledc.next(APB), Some(Tick(516)));
        assert_eq!(
            edges(&mut ledc, 620),
            [(516, false), (582, true), (612, false)]
        );
    }

    #[test]
    fn a_hardware_fade_steps_duty_and_raises_duty_chng_end() {
        let mut ledc = Ledc::new();
        // 4-bit timer, period = 16 * 6 = 96 ticks.
        timer(&mut ledc, Tick(0), 256, 4);
        // Enable DUTY_CHNG_END_LSCH0 (bit 4).
        ledc.write(Tick(1), INT_ENA, 1 << 4, APB).unwrap();
        // Start duty 2, inc=1, num=3, cycle=2, scale=4 -> 2 -> 6 -> 10 -> 14.
        ledc.write(Tick(1), 0x08, 2 << 4, APB).unwrap();
        ledc.write(
            Tick(1),
            0x0c,
            DUTY_START | DUTY_INC | 3 << 20 | 2 << 10 | 4,
            APB,
        )
        .unwrap();
        ledc.write(Tick(1), 0x00, SIG_OUT_EN | PARA_UP, APB)
            .unwrap();

        // Update takes effect at t0 = 96. Steps at 96 + 2*96 = 288, 480, 672.
        assert_eq!(ledc.next_int_event(Tick(1), APB), Some(Tick(672)));
        assert!(!ledc.line(Tick(671), APB));
        assert!(ledc.line(Tick(672), APB));
        assert_eq!(ledc.read(Tick(95), 0x10, APB), Some(0));
        assert_eq!(ledc.read(Tick(96), 0x10, APB), Some(2 << 4));
        assert_eq!(ledc.read(Tick(287), 0x10, APB), Some(2 << 4));
        assert_eq!(ledc.read(Tick(288), 0x10, APB), Some(6 << 4));
        assert_eq!(ledc.read(Tick(480), 0x10, APB), Some(10 << 4));
        assert_eq!(ledc.read(Tick(672), 0x10, APB), Some(14 << 4));
        assert_eq!(ledc.read(Tick(1000), 0x10, APB), Some(14 << 4));

        ledc.settle(Tick(700), APB);
        let steps: Vec<(u64, u32)> = ledc.take_done().iter().map(|e| (e.at.0, e.duty)).collect();
        assert_eq!(steps, [(96, 2), (288, 6), (480, 10), (672, 14)]);
        assert_eq!(ledc.read(Tick(700), INT_ST, APB), Some(1 << 4));
        assert_eq!(ledc.read(Tick(700), INT_RAW, APB), Some(1 << 4 | 1));
        ledc.write(Tick(700), INT_CLR, 1 << 4, APB).unwrap();
        assert!(!ledc.line(Tick(700), APB));
        assert_eq!(ledc.next_int_event(Tick(700), APB), None);
    }

    #[test]
    fn a_hardware_fade_down_reaches_zero_and_a_scale_zero_step_raises_fade_end() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        ledc.write(Tick(1), INT_ENA, 1 << 4, APB).unwrap();
        // Start duty 6, inc=0, num=3, cycle=1, scale=2 -> 6 -> 4 -> 2 -> 0 at 96, 192, 288, 384.
        ledc.write(Tick(1), 0x08, 6 << 4, APB).unwrap();
        ledc.write(Tick(1), 0x0c, DUTY_START | 3 << 20 | 1 << 10 | 2, APB)
            .unwrap();
        ledc.write(Tick(1), 0x00, SIG_OUT_EN | PARA_UP, APB)
            .unwrap();
        assert_eq!(ledc.next_int_event(Tick(1), APB), Some(Tick(384)));
        ledc.settle(Tick(384), APB);
        let steps: Vec<(u64, u32)> = ledc.take_done().iter().map(|e| (e.at.0, e.duty)).collect();
        assert_eq!(steps, [(96, 6), (192, 4), (288, 2), (384, 0)]);
        assert!(ledc.line(Tick(384), APB));
        ledc.write(Tick(390), INT_CLR, 1 << 4, APB).unwrap();
        assert!(!ledc.line(Tick(390), APB));

        // Remainder step (scale=0, num=1, cycle=1): duty loads at 480, DUTY_CHNG_END fires at 576.
        duty(&mut ledc, Tick(400), 1).unwrap();
        assert_eq!(ledc.next_int_event(Tick(400), APB), Some(Tick(576)));
        assert!(!ledc.line(Tick(575), APB));
        assert!(ledc.line(Tick(576), APB));
        ledc.write(Tick(580), INT_CLR, 1 << 4, APB).unwrap();
        assert!(!ledc.line(Tick(580), APB));
    }

    #[test]
    fn timer_overflow_interrupt_fires_each_period_and_pauses_cleanly() {
        let mut ledc = Ledc::new();
        let conf = 256 << 4 | 4;
        timer(&mut ledc, Tick(0), 256, 4);
        ledc.write(Tick(1), INT_ENA, 1, APB).unwrap();
        assert_eq!(ledc.next_int_event(Tick(1), APB), Some(Tick(96)));
        assert!(!ledc.line(Tick(95), APB));
        assert!(ledc.line(Tick(96), APB));
        assert_eq!(ledc.next_int_event(Tick(96), APB), None);
        ledc.write(Tick(110), INT_CLR, 1, APB).unwrap();
        assert!(!ledc.line(Tick(110), APB));
        assert_eq!(ledc.next_int_event(Tick(110), APB), Some(Tick(192)));
        // Pause at 120 (24 ticks into period): no overflow while paused.
        ledc.write(Tick(120), TIMER_BASE, conf | PAUSE, APB)
            .unwrap();
        assert_eq!(ledc.next_int_event(Tick(120), APB), None);
        assert!(!ledc.line(Tick(1000), APB));
        // Resume at 500: 72 ticks left -> overflow at 572.
        ledc.write(Tick(500), TIMER_BASE, conf, APB).unwrap();
        assert_eq!(ledc.next_int_event(Tick(500), APB), Some(Tick(572)));
        assert!(ledc.line(Tick(572), APB));
    }

    /// TRM v1.8, 35.3.2.2 and Register 35.7: writing `CLK_DIV` on a running timer does not change
    /// its period until `LEDC_LSTIMERn_PARA_UP` is set, and then the new period takes effect at
    /// the next counter overflow without corrupting the in-flight cycle.
    #[test]
    fn a_running_timer_divider_change_waits_for_para_up_and_takes_effect_at_next_overflow() {
        let mut ledc = Ledc::new();
        // Period = 16 * 6 = 96 ticks, duty 5 -> high 30 ticks [96, 126), low [126, 192).
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 5).unwrap();
        assert_eq!(edges(&mut ledc, 150), [(96, true), (126, false)]);

        // Write divider 512 WITHOUT PARA_UP at tick 150: register reads new divider, but active
        // period stays 96 ticks across the overflow at 192 -> [192, 222), [222, 288).
        ledc.write(Tick(150), TIMER_BASE, 512 << 4 | 4, APB)
            .unwrap();
        assert_eq!(ledc.read(Tick(150), TIMER_BASE, APB), Some(512 << 4 | 4));
        assert_eq!(
            edges(&mut ledc, 250),
            [(192, true), (222, false)],
            "without PARA_UP the old 96-tick period continues"
        );

        // Set PARA_UP at tick 250: current cycle still ends at 288, then new 192-tick period
        // starts at 288 -> high 60 ticks [288, 348), low [348, 480), next rise 480.
        ledc.write(Tick(250), TIMER_BASE, 512 << 4 | 4 | TIMER_STROBE, APB)
            .unwrap();
        assert_eq!(
            edges(&mut ledc, 550),
            [(288, true), (348, false), (480, true), (540, false)]
        );
        let done: Vec<(u64, u32, u8, u32)> = ledc
            .take_done()
            .iter()
            .map(|e| (e.at.0, e.duty, e.resolution, e.hz))
            .collect();
        assert_eq!(
            done,
            [(96, 5, 4, 5_000_000), (288, 5, 4, 2_500_000)],
            "Effect updates when the new period takes effect at overflow 288"
        );
    }

    /// TRM v1.8, 35.3.2.3: changing `LEDC_TIMERx_DUTY_RES` on a running timer and setting
    /// `LEDC_LSTIMERn_PARA_UP` applies the new resolution at the next counter overflow, keeping
    /// each channel's raw duty value.
    #[test]
    fn a_running_timer_resolution_change_takes_effect_at_next_overflow() {
        let mut ledc = Ledc::new();
        // Divider 256, 4 bits -> period 96 ticks. Duty 4/16 -> high 24 ticks.
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 4).unwrap();
        assert_eq!(edges(&mut ledc, 150), [(96, true), (120, false)]);

        // Switch to 3 bits (8 steps, period 48 ticks) at tick 150 with PARA_UP.
        // Current 96-tick cycle finishes at 192; from 192 onward duty 4/8 -> high 24 of 48 ticks.
        ledc.write(Tick(150), TIMER_BASE, 256 << 4 | 3 | TIMER_STROBE, APB)
            .unwrap();
        assert_eq!(
            edges(&mut ledc, 265),
            [(192, true), (216, false), (240, true), (264, false)]
        );
        let done = ledc.take_done();
        assert_eq!(done.len(), 2);
        assert_eq!(
            (done[1].at, done[1].duty, done[1].resolution),
            (Tick(192), 4, 3)
        );
    }

    /// Two channels sharing one timer both switch to the new period at the next overflow after
    /// `LEDC_LSTIMERn_PARA_UP`, each keeping its own duty.
    #[test]
    fn two_channels_on_one_timer_follow_a_running_timer_change() {
        let mut ledc = Ledc::new();
        // Period = 96 ticks, ch0 duty 4 (24 ticks), ch1 duty 8 (48 ticks).
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 4).unwrap();
        ledc.write(Tick(1), CHANNEL + 0x08, 8 << 4, APB).unwrap();
        ledc.write(Tick(1), CHANNEL + 0x0c, DUTY_START, APB)
            .unwrap();
        ledc.write(Tick(1), CHANNEL, SIG_OUT_EN | PARA_UP, APB)
            .unwrap();
        ledc.settle(Tick(160), APB);
        ledc.take_edges();

        // Double the divider (period = 192 ticks) at tick 160 -> takes effect at 192.
        // ch0: high 48 ticks [192, 240); ch1: high 96 ticks [192, 288); both rise at 384.
        ledc.write(Tick(160), TIMER_BASE, 512 << 4 | 4 | TIMER_STROBE, APB)
            .unwrap();
        ledc.settle(Tick(400), APB);
        let got: Vec<(u64, u8, bool)> = ledc
            .take_edges()
            .iter()
            .map(|e| (e.at.0, e.channel, e.level))
            .collect();
        assert_eq!(
            got,
            [
                (192, 0, true),
                (192, 1, true),
                (240, 0, false),
                (288, 1, false),
                (384, 0, true),
                (384, 1, true),
            ]
        );
    }

    /// Changing a running timer's period mid-fade with `PARA_UP` re-anchors remaining fade
    /// overflows at the next overflow and keeps `DUTY_R` and `next_int_event` exact.
    #[test]
    fn a_running_timer_change_mid_fade_reanchors_remaining_fade_steps() {
        let mut ledc = Ledc::new();
        // Period = 96 ticks. Fade: start duty 2, +4 every 2 overflows for 3 steps -> 2, 6, 10, 14.
        timer(&mut ledc, Tick(0), 256, 4);
        ledc.write(Tick(1), INT_ENA, 1 << 4, APB).unwrap();
        ledc.write(Tick(1), 0x08, 2 << 4, APB).unwrap();
        ledc.write(
            Tick(1),
            0x0c,
            DUTY_START | DUTY_INC | 3 << 20 | 2 << 10 | 4,
            APB,
        )
        .unwrap();
        ledc.write(Tick(1), 0x00, SIG_OUT_EN | PARA_UP, APB)
            .unwrap();
        // Initial load at 96 (duty 2), step 1 at 288 (duty 6). Overflows after 288: 384, 480.
        ledc.settle(Tick(300), APB);
        assert_eq!(ledc.read(Tick(300), 0x10, APB), Some(6 << 4));

        // At tick 300 double the period (192 ticks) via PARA_UP.
        // Takes effect at overflow 384: 1 overflow was left to step 2 -> step 2 at 384 + 192 = 576
        // (duty 10), step 3 at 576 + 2*192 = 960 (duty 14, DUTY_CHNG_END).
        ledc.write(Tick(300), TIMER_BASE, 512 << 4 | 4 | TIMER_STROBE, APB)
            .unwrap();
        assert_eq!(ledc.next_int_event(Tick(300), APB), Some(Tick(960)));
        assert_eq!(ledc.read(Tick(575), 0x10, APB), Some(6 << 4));
        assert_eq!(ledc.read(Tick(576), 0x10, APB), Some(10 << 4));
        assert_eq!(ledc.read(Tick(959), 0x10, APB), Some(10 << 4));
        assert_eq!(ledc.read(Tick(960), 0x10, APB), Some(14 << 4));
        assert!(!ledc.line(Tick(959), APB));
        assert!(ledc.line(Tick(960), APB));

        ledc.settle(Tick(960), APB);
        assert_eq!(ledc.read(Tick(960), 0x10, APB), Some(14 << 4));
        assert!(ledc.line(Tick(960), APB));
    }

    /// ESP-IDF `ledc_timer_config` writes `CLK_DIV`, `DUTY_RES`, `PARA_UP`, `PAUSE=0`, `RST=1`,
    /// `RST=0` on a running timer: `RST` applies the latched `PARA_UP` parameters so the
    /// restarted timer immediately runs at the new period.
    #[test]
    fn idf_timer_reconfig_with_para_up_then_reset_applies_new_period() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 4).unwrap();
        assert_eq!(edges(&mut ledc, 150), [(96, true), (120, false)]);

        // Replicate ledc_timer_config: CLK_DIV -> DUTY_RES -> PARA_UP -> PAUSE=0 -> RST=1 -> RST=0.
        let new_conf = 512 << 4 | 4;
        ledc.write(Tick(150), TIMER_BASE, new_conf, APB).unwrap();
        ledc.write(Tick(151), TIMER_BASE, new_conf, APB).unwrap();
        ledc.write(Tick(152), TIMER_BASE, new_conf | TIMER_STROBE, APB)
            .unwrap();
        ledc.write(Tick(153), TIMER_BASE, new_conf, APB).unwrap();
        ledc.write(Tick(154), TIMER_BASE, new_conf | RST, APB)
            .unwrap();
        ledc.write(Tick(155), TIMER_BASE, new_conf, APB).unwrap();

        // RST at 154 drives channel high; release at 155 runs 192-tick period with duty 4/16
        // (48 ticks high): fall at 155 + 48 = 203, rise at 155 + 192 = 347, fall at 395.
        assert_eq!(
            edges(&mut ledc, 400),
            [(154, true), (203, false), (347, true), (395, false)]
        );
    }

    /// A second timer `PARA_UP` before overflow replaces the first; a channel `PARA_UP` due at
    /// that same overflow applies immediately with the new period, and timer overflow interrupts
    /// follow the new period after that overflow.
    #[test]
    fn a_timer_para_up_replacement_and_simultaneous_channel_update_land_on_the_same_overflow() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        ledc.write(Tick(1), INT_ENA, 1, APB).unwrap();
        // Queue a channel update at tick 50 and two timer PARA_UPs at ticks 60 and 70.
        duty(&mut ledc, Tick(50), 8).unwrap();
        ledc.write(Tick(60), TIMER_BASE, 512 << 4 | 4 | TIMER_STROBE, APB)
            .unwrap();
        ledc.write(Tick(70), TIMER_BASE, 1024 << 4 | 4 | TIMER_STROBE, APB)
            .unwrap();
        // Overflow at 96 applies divider 1024 (period 384) and duty 8/16 (high 192 ticks).
        assert_eq!(ledc.next_int_event(Tick(70), APB), Some(Tick(96)));
        assert_eq!(edges(&mut ledc, 96), [(96, true)]);
        assert!(ledc.line(Tick(96), APB));
        let done = ledc.take_done();
        assert_eq!(done.len(), 1);
        assert_eq!(
            (done[0].at, done[0].duty, done[0].hz),
            (Tick(96), 8, 1_250_000)
        );
        // Clear OVF at 100 -> next OVF is at 96 + 384 = 480.
        ledc.write(Tick(100), INT_CLR, 1, APB).unwrap();
        assert_eq!(ledc.next_int_event(Tick(100), APB), Some(Tick(480)));
        assert_eq!(edges(&mut ledc, 500), [(288, false), (480, true)]);
        assert!(ledc.line(Tick(480), APB));
    }

    /// Dropping a running timer's resolution so `2^DUTY_RES <= duty` holds the channel steady high
    /// from the switch on while keeping `DUTY_R` unchanged; raising resolution back above `duty`
    /// resumes PWM at that same raw duty (TRM v1.8, 35.3.2.3 and 35.3.3).
    #[test]
    fn a_running_timer_resolution_drop_below_duty_holds_high_and_resumes_when_raised() {
        let mut ledc = Ledc::new();
        // Divider 256, 4 bits -> period 96 ticks. Duty 8/16 -> high 48 ticks [96, 144), low [144, 192).
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 8).unwrap();
        assert_eq!(edges(&mut ledc, 150), [(96, true), (144, false)]);

        // Drop resolution to 3 bits (8 steps, period 48 ticks) via PARA_UP at tick 150.
        // At overflow 192, raw duty 8 >= 2^3 -> signal rises and holds steady high, DUTY_R stays 8.
        ledc.write(Tick(150), TIMER_BASE, 256 << 4 | 3 | TIMER_STROBE, APB)
            .unwrap();
        assert_eq!(edges(&mut ledc, 500), [(192, true)]);
        assert_eq!(ledc.next(APB), None, "steady high has no PWM edge");
        assert_eq!(ledc.read(Tick(500), 0x10, APB), Some(8 << 4));

        // Raise resolution back to 4 bits via PARA_UP at tick 500 -> next 48-tick overflow of the
        // 3-bit timer is 192 + 7 * 48 = 528. From 528 on, duty 8/16 resumes: fall at 576, rise 624.
        ledc.write(Tick(500), TIMER_BASE, 256 << 4 | 4 | TIMER_STROBE, APB)
            .unwrap();
        assert_eq!(edges(&mut ledc, 650), [(576, false), (624, true)]);
    }
}
