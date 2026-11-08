//! SYSTEM's peripheral clock gates and resets: `SYSTEM_PERIP_CLK_EN0_REG`,
//! `SYSTEM_PERIP_CLK_EN1_REG`, `SYSTEM_PERIP_RST_EN0_REG` and `SYSTEM_PERIP_RST_EN1_REG`.
//!
//! SPI2, LEDC, I2C0, GDMA, the SHA accelerator and RMT run their gates. Each runs while its clock bit is set
//! and its reset bits are clear. A write that leaves one of its reset bits set puts it back to
//! its power-on state, and it stays there until they clear. The SHA accelerator is held by the
//! digital signature and HMAC reset bits too, as IDF's `sha_ll_reset_register` says.
//!
//! The clock and reset bits of SYSTIMER, TIMG0, TIMG1, SPI0 and SPI1, the eFuse controller, the
//! USB Serial/JTAG unit, and the bits IDF names `SYSTEM_TIMERS_*` and `SYSTEM_WDG_*` are kept and
//! not run, and every write that changes one is reported.
//! Every other bit gates or resets a peripheral the model does not keep, so it has no effect.
//!
//! The reset values come from IDF's `soc/esp32s3/register/soc/system_reg.h`.

use std::ops::RangeInclusive;

/// The offsets in SYSTEM of the four words.
pub(crate) const OFFSETS: RangeInclusive<u32> = 0x18..=0x24;

/// A peripheral that runs its gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Spi2,
    Ledc,
    Gdma,
    Sha,
    I2c0,
    Rmt,
}

impl Block {
    pub(crate) const ALL: [Block; 6] = [
        Block::Spi2,
        Block::Ledc,
        Block::Gdma,
        Block::Sha,
        Block::I2c0,
        Block::Rmt,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Block::Spi2 => "SPI2",
            Block::Ledc => "LEDC",
            Block::Gdma => "GDMA",
            Block::Sha => "SHA",
            Block::I2c0 => "I2C0",
            Block::Rmt => "RMT",
        }
    }

    /// Which of the two words of each kind holds its bits, its clock bit, and its reset bits.
    fn bits(self) -> (usize, u32, u32) {
        match self {
            Block::Spi2 => (0, 1 << 6, 1 << 6),
            Block::Ledc => (0, 1 << 11, 1 << 11),
            Block::Gdma => (1, 1 << 6, 1 << 6),
            Block::Sha => (1, 1 << 2, 1 << 2 | 1 << 4 | 1 << 5),
            Block::I2c0 => (0, 1 << 7, 1 << 7),
            Block::Rmt => (0, 1 << 9, 1 << 9),
        }
    }
}

/// In each word of each kind, the bits the model keeps without running: SYSTIMER, TIMG0, TIMG1,
/// SPI0 and SPI1, the eFuse controller, and `SYSTEM_TIMERS_*` and `SYSTEM_WDG_*` in the first,
/// the USB Serial/JTAG unit in the second. IDF's headers leave the timers and watchdog bits
/// undescribed and no IDF driver gates a block with them, so a change is reported in case they
/// gate a timer or a watchdog the model runs.
const REPORTED: [u32; 2] = [
    1 << 29 | 1 << 15 | 1 << 14 | 1 << 13 | 1 << 3 | 1 << 1 | 1 << 0,
    1 << 10,
];

const CLOCK_RESET: [u32; 2] = [0xf9c1_e06f, 0x0000_0600];
const RESET_RESET: [u32; 2] = [0, 0x0000_01fe];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Gates {
    /// `CLK_EN0` and `CLK_EN1`, then `RST_EN0` and `RST_EN1`.
    words: [u32; 4],
}

impl Gates {
    pub(crate) fn new() -> Gates {
        Gates {
            words: [
                CLOCK_RESET[0],
                CLOCK_RESET[1],
                RESET_RESET[0],
                RESET_RESET[1],
            ],
        }
    }

    fn index(offset: u32) -> usize {
        ((offset - OFFSETS.start()) / 4) as usize
    }

    pub(crate) fn read(&self, offset: u32) -> u32 {
        self.words[Self::index(offset)]
    }

    /// Whether writing `value` at `offset` changes a bit the model keeps without running.
    pub(crate) fn reports(&self, offset: u32, value: u32) -> bool {
        let k = Self::index(offset);
        (self.words[k] ^ value) & REPORTED[k % 2] != 0
    }

    /// Writes `value` at `offset`, and returns each block a reset bit now holds.
    pub(crate) fn write(&mut self, offset: u32, value: u32) -> impl Iterator<Item = Block> + '_ {
        self.words[Self::index(offset)] = value;
        Block::ALL.into_iter().filter(|&block| self.held(block))
    }

    pub(crate) fn held(&self, block: Block) -> bool {
        let (k, _, reset) = block.bits();
        self.words[2 + k] & reset != 0
    }

    pub(crate) fn clocked(&self, block: Block) -> bool {
        let (k, clock, _) = block.bits();
        self.words[k] & clock != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_runs_while_its_clock_is_set_and_its_resets_are_clear() {
        let mut gates = Gates::new();
        assert!(gates.clocked(Block::Spi2) && !gates.held(Block::Spi2));
        assert!(!gates.clocked(Block::Gdma) && gates.held(Block::Gdma));
        assert!(!gates.clocked(Block::Ledc) && !gates.held(Block::Ledc));
        assert!(!gates.clocked(Block::I2c0) && !gates.held(Block::I2c0));
        let held: Vec<Block> = gates.write(0x20, 1 << 6).collect();
        assert_eq!(held, [Block::Spi2, Block::Gdma, Block::Sha]);
        assert!(gates.write(0x24, 1 << 4).any(|block| block == Block::Sha));
        assert!(!gates.write(0x24, 0).any(|block| block == Block::Sha));
        assert_eq!(gates.read(0x24), 0);
        gates.write(0x1c, 1 << 2).for_each(drop);
        assert!(gates.clocked(Block::Sha) && !gates.clocked(Block::Gdma));
    }

    #[test]
    fn a_change_of_a_kept_bit_is_reported_and_other_bits_are_not() {
        let gates = Gates::new();
        assert!(gates.reports(0x20, 1 << 29));
        assert!(gates.reports(0x18, CLOCK_RESET[0] & !(1 << 13)));
        assert!(gates.reports(0x24, RESET_RESET[1] | 1 << 10));
        assert!(gates.reports(0x20, 1 << 0), "SYSTEM_TIMERS_RST");
        assert!(
            gates.reports(0x18, CLOCK_RESET[0] & !(1 << 3)),
            "SYSTEM_WDG_CLK_EN"
        );
        assert!(!gates.reports(0x20, 1 << 6 | 1 << 2));
        assert!(!gates.reports(0x1c, CLOCK_RESET[1] | 1 << 2));
        assert!(!gates.reports(0x18, CLOCK_RESET[0]));
    }
}
