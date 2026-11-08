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
//! The channel's signal is high while its duty is above 0 and low at 0. While
//! `LEDC_SIG_OUT_EN_LSCHn` is clear the channel sends its idle level, a duty of 0 or of every
//! step. At power-on every signal is low.
//!
//! The interrupt registers read 0, and the self-clearing update bits read 0.
//! `LEDC_LSCHn_DUTY_R_REG` and `LEDC_LSTIMERn_VALUE_REG` are not kept. The registers come from
//! IDF's `soc/esp32s3/register/soc/ledc_reg.h`, with the reset values its field comments give.

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
    /// A word with bits that clear themselves.
    Strobed(u32),
}

fn kind(offset: u32) -> Option<Kind> {
    Some(match offset {
        _ if offset < CHANNELS as u32 * CHANNEL => match offset % CHANNEL {
            0x00 => Kind::Strobed(CHANNEL_STROBES),
            0x04 | 0x08 | 0x0c => Kind::Kept,
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

    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        let word = self.reg(offset);
        Some(match kind(offset)? {
            Kind::Kept => word,
            Kind::Clear => 0,
            Kind::Strobed(strobes) => word & !strobes,
        })
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
        if kind(offset).is_none() {
            return Ok(false);
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

    /// The tick of the next update to take effect.
    pub(crate) fn next(&self, apb: u64) -> Option<Tick> {
        (0..CHANNELS).filter_map(|k| self.due(k, apb)).min()
    }

    /// Lets every update due at or before `through` take effect, in time order.
    pub(crate) fn settle(&mut self, through: Tick, apb: u64) {
        while let Some((at, channel)) = (0..CHANNELS)
            .filter_map(|k| Some((self.due(k, apb)?, k)))
            .min()
            .filter(|&(at, _)| at <= through)
        {
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
        assert_eq!(ledc.read(0x0c), Some(CONF1_RESET));
        assert_eq!(ledc.read(TIMER_BASE), Some(TIMER_RESET));
        assert_eq!(ledc.read(DATE), Some(DATE_RESET));
        assert_eq!(ledc.write(Tick(0), 0x14 + 0x08, 4095 << 4, APB), Ok(true));
        assert_eq!(ledc.write(Tick(0), 0x14, 1 << 2 | 1 << 4, APB), Ok(true));
        let conf = TIMER_STROBE | RST | 12;
        assert_eq!(ledc.write(Tick(0), TIMER_BASE + TIMER, conf, APB), Ok(true));
        assert_eq!(ledc.read(0x14 + 0x08), Some(4095 << 4));
        assert_eq!(ledc.read(0x14), Some(1 << 2));
        assert_eq!(ledc.read(TIMER_BASE + TIMER), Some(RST | 12));
        assert_eq!(ledc.write(Tick(0), INT_CLR, u32::MAX, APB), Ok(true));
        assert_eq!(ledc.read(INT_ST), Some(0));
        assert_eq!(ledc.read(INT_CLR), Some(0));
        assert_eq!(ledc.read(0x10), None, "DUTY_R");
        assert_eq!(ledc.read(TIMER_BASE + 4), None, "VALUE");
        assert_eq!(ledc.write(Tick(0), 0x10, 1, APB), Ok(false));
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
        assert_eq!(ledc.next(APB), None);
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
