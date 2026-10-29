//! The LED PWM controller, LEDC, as far as firmware sets a channel up: its eight channels' and
//! four timers' configuration, its interrupt registers and `LEDC_CONF_REG` are kept from their
//! reset values, and nothing runs. No timer counts, no channel drives its signal, and no duty
//! fades, so a pad routed to a channel drives nothing. The self-clearing update bits read 0.
//! `LEDC_LSCHn_DUTY_R_REG` and `LEDC_LSTIMERn_VALUE_REG`, which a running controller fills, are
//! not kept.
//!
//! The registers come from IDF's `soc/esp32s3/register/soc/ledc_reg.h`, with the reset values
//! its field comments give.

const CHANNELS: u32 = 8;
const CHANNEL: u32 = 0x14;
const TIMERS: u32 = 0xa0;
const TIMER: u32 = 0x08;
const INT_RAW: u32 = 0xc0;
const INT_ST: u32 = 0xc4;
/// `LEDC_INT_ENA_REG`.
pub(crate) const INT_ENA: u32 = 0xc8;
const INT_CLR: u32 = 0xcc;
const CONF: u32 = 0xd0;
const DATE: u32 = 0xfc;
const WORDS: usize = (DATE / 4 + 1) as usize;

/// `LEDC_OVF_CNT_RESET_LSCHn` and `LEDC_PARA_UP_LSCHn` in `CONF0`.
const CHANNEL_STROBES: u32 = 1 << 16 | 1 << 4;
/// `LEDC_LSTIMERn_PARA_UP`.
const TIMER_STROBE: u32 = 1 << 25;
/// `LEDC_DUTY_INC_LSCHn` in `CONF1`, and `LEDC_LSTIMERn_RST`.
const CONF1_RESET: u32 = 1 << 30;
const TIMER_RESET: u32 = 1 << 23;
const DATE_RESET: u32 = 0x1904_0200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ledc {
    regs: [u32; WORDS],
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
        _ if offset < CHANNELS * CHANNEL => match offset % CHANNEL {
            0x00 => Kind::Strobed(CHANNEL_STROBES),
            0x04 | 0x08 | 0x0c => Kind::Kept,
            _ => return None,
        },
        TIMERS..INT_RAW => match offset % TIMER {
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
        for channel in 0..CHANNELS {
            regs[((channel * CHANNEL + 0x0c) / 4) as usize] = CONF1_RESET;
        }
        for timer in 0..4 {
            regs[((TIMERS + timer * TIMER) / 4) as usize] = TIMER_RESET;
        }
        regs[(DATE / 4) as usize] = DATE_RESET;
        Ledc { regs }
    }

    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        let word = self.regs[(offset / 4) as usize];
        Some(match kind(offset)? {
            Kind::Kept => word,
            Kind::Clear => 0,
            Kind::Strobed(strobes) => word & !strobes,
        })
    }

    pub(crate) fn write(&mut self, offset: u32, value: u32) -> bool {
        if kind(offset).is_none() {
            return false;
        }
        self.regs[(offset / 4) as usize] = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_and_a_timer_keep_their_setup_and_their_update_bits_read_0() {
        let mut ledc = Ledc::new();
        assert_eq!(ledc.read(0x0c), Some(CONF1_RESET));
        assert_eq!(ledc.read(TIMERS), Some(TIMER_RESET));
        assert_eq!(ledc.read(DATE), Some(DATE_RESET));
        assert!(ledc.write(0x14 + 0x08, 4095 << 4));
        assert!(ledc.write(0x14, 1 << 2 | 1 << 4));
        assert!(ledc.write(TIMERS + TIMER, TIMER_STROBE | 12));
        assert_eq!(ledc.read(0x14 + 0x08), Some(4095 << 4));
        assert_eq!(ledc.read(0x14), Some(1 << 2));
        assert_eq!(ledc.read(TIMERS + TIMER), Some(12));
        assert!(ledc.write(INT_CLR, u32::MAX));
        assert_eq!(ledc.read(INT_ST), Some(0));
        assert_eq!(ledc.read(INT_CLR), Some(0));
        assert_eq!(ledc.read(0x10), None, "DUTY_R");
        assert_eq!(ledc.read(TIMERS + 4), None, "VALUE");
        assert!(!ledc.write(0x10, 1));
    }
}
