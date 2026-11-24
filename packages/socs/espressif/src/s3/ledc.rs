//! The LED PWM controller, LEDC: its eight channels and four timers, as ESP-IDF's `ledc` driver
//! sets a fixed duty.
//!
//! A timer counts from the store that leaves its `RST` and `PAUSE` bits clear. Its period is
//! `LEDC_CLK_DIV` over 256, times `2^DUTY_RES`, cycles of the clock `LEDC_APB_CLK_SEL` picks: the
//! APB clock, or the 40 MHz crystal. A store that sets `LEDC_PARA_UP_LSCHn` takes the channel's
//! registers as they are then, and that update takes effect at the next overflow of the timer the
//! channel names, the next PWM cycle, as IDF's `ledc_update_duty` says. A later update before that
//! overflow replaces it. An update whose timer does not count waits until it does. An update with
//! `LEDC_DUTY_START_LSCHn` clear keeps the duty the last one took. `ledc_stop` stores only the
//! idle level, a clear `LEDC_SIG_OUT_EN_LSCHn` and the update bit, and `LEDC_DUTY_START_LSCHn`
//! does not clear itself, so after `ledc_update_duty` that update takes the duty register again.
//!
//! The channel's signal is the PWM of the ESP32-S3 TRM (v1.8, 35.3.3): high when the timer's
//! counter is `Hpoint`, which is 0, and low when it is `Lpoint`, the duty. So from each overflow
//! of its timer the signal is high for the duty's steps and low for the rest of the period. A
//! step that is not whole ticks of 480 MHz rounds the high time to the nearest tick. A duty of 0
//! is low throughout and a duty of every step high throughout. While `LEDC_SIG_OUT_EN_LSCHn` is
//! clear the channel sends its idle level, a duty of 0 or of every step. At power-on every signal
//! is low.
//!
//! `LEDC_LSCHn_DUTY_R_REG` reads the duty in force, from the last update that took effect, with
//! its 4 fraction bits clear, and 0 before the first (TRM 35.5, Register 35.6). ESP-IDF's
//! `ledc_get_duty` reads it, and arduino-esp32's `ledcAttach` keeps the duty it reads. The
//! interrupt registers read 0, and the self-clearing update bits read 0.
//! `LEDC_LSTIMERn_VALUE_REG` is not kept. The registers come from IDF's
//! `soc/esp32s3/register/soc/ledc_reg.h`, with the reset values its field comments give.

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
const CONF1_RESET: u32 = 1 << 30;
const TIMER_RESET: u32 = RST;
const DATE_RESET: u32 = 0x1904_0200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ledc {
    regs: [u32; WORDS],
    /// The tick each timer counts from, while it counts.
    started: [Option<Tick>; TIMERS],
    /// Each channel's update that waits for its timer's next overflow.
    pending: [Option<Update>; CHANNELS],
    /// Each channel's duty, as its last update that took effect set it.
    duty: [u32; CHANNELS],
    /// What each channel sends, as its last update that took effect set it.
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
    /// It reads 0 and keeps nothing.
    Clear,
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
        INT_RAW | INT_ST | INT_CLR => Kind::Clear,
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
            started: [None; TIMERS],
            pending: [None; CHANNELS],
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

    fn timer(&self, timer: usize) -> u32 {
        self.reg(TIMER_BASE + timer as u32 * TIMER)
    }

    /// Reads the register at `offset` at `now`, with `apb` ticks per APB cycle.
    pub(crate) fn read(&self, now: Tick, offset: u32, apb: u64) -> Option<u32> {
        let word = self.reg(offset);
        Some(match kind(offset)? {
            Kind::Kept => word,
            Kind::Clear => 0,
            Kind::Strobed(strobes) => word & !strobes,
            Kind::DutyRead => self.duty_at((offset / CHANNEL) as usize, now, apb) << 4,
        })
    }

    /// Channel `channel`'s duty in force at `now`: that of an update due by then, or else that
    /// of the last one that took effect.
    fn duty_at(&self, channel: usize, now: Tick, apb: u64) -> u32 {
        match (self.pending[channel], self.due(channel, apb)) {
            (Some(update), Some(due)) if due <= now => update.duty,
            _ => self.duty[channel],
        }
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
            Some(Kind::DutyRead) => return Ok(true),
            Some(_) => {}
        }
        self.settle(now, apb);
        let old = self.reg(offset);
        self.regs[(offset / 4) as usize] = value;
        if offset < CHANNELS as u32 * CHANNEL {
            if offset.is_multiple_of(CHANNEL) && value & PARA_UP != 0 {
                self.update((offset / CHANNEL) as usize, now)?;
            }
        } else if (TIMER_BASE..INT_RAW).contains(&offset) {
            let timer = ((offset - TIMER_BASE) / TIMER) as usize;
            let counts = value & (RST | PAUSE) == 0;
            if self.started[timer].is_some() {
                if (old ^ value) & TIMER_SETUP != 0 {
                    return Err("timer_change_while_running");
                }
                if value & (RST | PAUSE) == PAUSE {
                    return Err("pause");
                }
                if !counts && self.pwm.iter().flatten().any(|pwm| pwm.timer == timer) {
                    return Err("timer_reset_while_running");
                }
            }
            self.started[timer] = match self.started[timer] {
                _ if !counts => None,
                Some(since) => Some(since),
                None => Some(now),
            };
        } else if offset == CONF && (old ^ value) & CLK_SEL != 0 && self.counts() {
            return Err("clock_change_while_running");
        }
        self.check(apb)?;
        Ok(true)
    }

    /// Takes channel `channel`'s registers as its next update, from `now`.
    fn update(&mut self, channel: usize, now: Tick) -> Result<(), &'static str> {
        if self.channel(channel, 0x04) & HPOINT != 0 {
            return Err("hpoint");
        }
        let conf1 = self.channel(channel, 0x0c);
        let duty = if conf1 & DUTY_START == 0 {
            self.duty[channel]
        } else if conf1 & DUTY_SCALE != 0 {
            return Err("fade");
        } else {
            let duty = self.channel(channel, 0x08) & DUTY;
            if duty & 0xf != 0 {
                return Err("duty_fraction");
            }
            duty >> 4
        };
        let conf0 = self.channel(channel, 0x00);
        self.pending[channel] = Some(Update {
            at: now,
            timer: (conf0 & TIMER_SEL) as usize,
            duty,
            out: conf0 & SIG_OUT_EN != 0,
            idle: conf0 & IDLE_LV != 0,
        });
        Ok(())
    }

    /// Whether any timer counts.
    pub(crate) fn counts(&self) -> bool {
        self.started.iter().any(Option::is_some)
    }

    /// Whether an update waits for its timer's overflow.
    pub(crate) fn pending(&self) -> bool {
        self.pending.iter().any(Option::is_some)
    }

    /// Whether a timer counts the APB clock, so a change of the APB clock would move its period.
    pub(crate) fn counts_apb(&self) -> bool {
        self.counts() && self.reg(CONF) & CLK_SEL == 1
    }

    /// The ticks of one period of timer `timer`, and its duty resolution.
    fn period(&self, timer: usize, apb: u64) -> Result<(u64, u8), &'static str> {
        let clock = match self.reg(CONF) & CLK_SEL {
            1 => apb,
            3 => XTAL,
            _ => return Err("clock"),
        };
        let conf = self.timer(timer);
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

    /// Refuses an update the model cannot time or send: one whose timer counts with no period
    /// the model runs, or with more duty than steps. A duty of every step at the 14 bits of
    /// `SOC_LEDC_TIMER_BIT_WIDTH` is refused too. IDF's `ledc_channel_config` notes that it
    /// messes up the hardware's duty, and does not say what the pad then drives.
    fn check(&self, apb: u64) -> Result<(), &'static str> {
        for update in self.pending.iter().flatten() {
            if self.started[update.timer].is_none() {
                continue;
            }
            let (_, resolution) = self.period(update.timer, apb)?;
            if update.duty > 1 << resolution {
                return Err("duty_range");
            }
            if resolution == MAX_RESOLUTION && update.duty == 1 << resolution {
                return Err("full_duty");
            }
        }
        Ok(())
    }

    /// The tick channel `channel`'s update takes effect, once its timer counts.
    fn due(&self, channel: usize, apb: u64) -> Option<Tick> {
        let update = self.pending[channel]?;
        let since = self.started[update.timer]?;
        let (period, _) = self.period(update.timer, apb).ok()?;
        let cycles = update.at.0.saturating_sub(since.0) / period + 1;
        Some(Tick(since.0 + cycles * period))
    }

    /// The next change of a channel's signal from its PWM, as the tick, the channel and the
    /// level. A PWM whose timer does not count makes none.
    fn next_edge(&self) -> Option<(Tick, usize, bool)> {
        (0..CHANNELS)
            .filter_map(|k| {
                let pwm = self.pwm[k]?;
                self.started[pwm.timer]?;
                Some(if self.level[k] {
                    (Tick(pwm.start.0 + pwm.high), k, false)
                } else {
                    (Tick(pwm.start.0 + pwm.period), k, true)
                })
            })
            .min()
    }

    /// The tick of the next update to take effect or of the next PWM edge.
    pub(crate) fn next(&self, apb: u64) -> Option<Tick> {
        let update = (0..CHANNELS).filter_map(|k| self.due(k, apb)).min();
        let edge = self.next_edge().map(|(at, ..)| at);
        [update, edge].into_iter().flatten().min()
    }

    /// Lets every update due at or before `through` take effect and every PWM edge then pass, in
    /// time order. An update at the overflow of a PWM's rising edge starts its own cycle there.
    pub(crate) fn settle(&mut self, through: Tick, apb: u64) {
        loop {
            let update = (0..CHANNELS)
                .filter_map(|k| Some((self.due(k, apb)?, k)))
                .min()
                .filter(|&(at, _)| at <= through);
            let edge = self.next_edge().filter(|&(at, ..)| at <= through);
            match (update, edge) {
                (Some((at, channel)), edge) if edge.is_none_or(|(edge, ..)| at <= edge) => {
                    self.take_update(at, channel, apb);
                }
                (_, Some((at, channel, level))) => {
                    if level {
                        let pwm = self.pwm[channel].as_mut().expect("a PWM");
                        pwm.start = at;
                    }
                    self.drive(at, channel, level);
                }
                _ => break,
            }
        }
    }

    /// Channel `channel`'s pending update takes effect at `at`, an overflow of its timer.
    fn take_update(&mut self, at: Tick, channel: usize, apb: u64) {
        let update = self.pending[channel].take().expect("due");
        let (period, resolution) = self.period(update.timer, apb).expect("checked");
        let steps = 1 << resolution;
        self.duty[channel] = update.duty;
        let duty = match (update.out, update.idle) {
            (true, _) => update.duty,
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
            timer: update.timer,
            start: at,
            period,
            high: (u64::from(duty) * period + u64::from(steps >> 1)) >> resolution,
        });
        self.drive(at, channel, duty > 0);
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

    /// The updates that took effect since the last call, in the order they did.
    pub(crate) fn take_done(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.done)
    }

    /// The changes of the channels' signals since the last call, in time order.
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
                l.write(Tick(1), 0x0c, DUTY_START | 1 << 20 | 1 << 10 | 4, APB)?;
                l.write(Tick(1), 0x00, PARA_UP, APB)
            }),
            "fade"
        );
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
        assert_eq!(refused(&|l| duty(l, Tick(1), 17)), "duty_range");
        assert_eq!(
            refused(&|l| l.write(Tick(1), TIMER_BASE, 512 << 4 | 4, APB)),
            "timer_change_while_running"
        );
        assert_eq!(
            refused(&|l| l.write(Tick(1), TIMER_BASE, 256 << 4 | 4 | PAUSE, APB)),
            "pause"
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

    /// A reset of a timer that a channel's PWM counts on ends the run, since TRM 35.3.2.3 does
    /// not say what the output does while the counter is held. With no PWM on it, the reset
    /// goes through.
    #[test]
    fn a_reset_of_a_timer_under_a_pwm_is_refused() {
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 5).unwrap();
        ledc.settle(Tick(96), APB);
        let reset = 256 << 4 | 4 | RST;
        assert_eq!(
            ledc.write(Tick(100), TIMER_BASE, reset, APB),
            Err("timer_reset_while_running")
        );
        let mut ledc = Ledc::new();
        timer(&mut ledc, Tick(0), 256, 4);
        duty(&mut ledc, Tick(1), 16).unwrap();
        ledc.settle(Tick(96), APB);
        assert_eq!(ledc.write(Tick(100), TIMER_BASE, reset, APB), Ok(true));
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
}
