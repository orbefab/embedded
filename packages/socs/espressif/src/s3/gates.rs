//! SYSTEM's peripheral clock gates and resets: `SYSTEM_PERIP_CLK_EN0_REG`,
//! `SYSTEM_PERIP_CLK_EN1_REG`, `SYSTEM_PERIP_RST_EN0_REG` and `SYSTEM_PERIP_RST_EN1_REG`.
//!
//! SPI2, LEDC, I2C0, GDMA, the SHA accelerator, RMT and the three UARTs run their gates. Each runs while its clock bit is set
//! and its reset bits are clear. A write that leaves one of its reset bits set puts it back to
//! its power-on state, and it stays there until they clear. The SHA accelerator is held by the
//! digital signature and HMAC reset bits too, as IDF's `sha_ll_reset_register` says.
//!
//! The clock bits of TIMG0, TIMG1, SYSTIMER and `SYSTEM_WDG` stay reported. Their reset bits do
//! not. The general timers run the timer-group reset. `esp_perip_clk_init` sets `SYSTEM_WDG_RST`
//! by copying the inverted clock word into the reset word after a CPU reset, and no modeled
//! watchdog reads it. `SYSTIMER_RST` is pulsed over a count the capture already holds at 0, and
//! the firmware rewrites the timer after the pulse. While a timer-group reset bit is set the
//! general timers sit at their reset values, and while its clock bit is clear they keep their
//! count and do not advance. A load or a store of a general timer is not refused while the group
//! is gated. The watchdog keeps counting through both.
//!
//! The clock and reset bits of SPI0 and SPI1, the eFuse controller, the USB Serial/JTAG unit, and
//! the bits IDF names `SYSTEM_TIMERS_*` are kept and not run, and every write that changes one is
//! reported.
//! Every other bit gates or resets a peripheral the model does not keep, so it has no effect,
//! apart from `SYSTEM_UART_MEM_CLK_EN`, bit 24 of `CLK_EN0`. That bit stays stored here. A UART
//! FIFO access or a frame while it is clear ends the run, as the UART model says. The write of
//! the bit itself does not.
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
    Uart0,
    Uart1,
    Uart2,
}

impl Block {
    pub(crate) const ALL: [Block; 9] = [
        Block::Spi2,
        Block::Ledc,
        Block::Gdma,
        Block::Sha,
        Block::I2c0,
        Block::Rmt,
        Block::Uart0,
        Block::Uart1,
        Block::Uart2,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Block::Spi2 => "SPI2",
            Block::Ledc => "LEDC",
            Block::Gdma => "GDMA",
            Block::Sha => "SHA",
            Block::I2c0 => "I2C0",
            Block::Rmt => "RMT",
            Block::Uart0 => "UART0",
            Block::Uart1 => "UART1",
            Block::Uart2 => "UART2",
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
            Block::Uart0 => (0, 1 << 2, 1 << 2),
            Block::Uart1 => (0, 1 << 5, 1 << 5),
            Block::Uart2 => (1, 1 << 9, 1 << 9),
        }
    }
}

/// In each clock word, the bits a change of which is reported. The reset word uses
/// [`REPORTED_RESET`], which leaves out `SYSTIMER_RST`, `TIMERGROUP_RST`, `TIMERGROUP1_RST` and
/// `SYSTEM_WDG_RST`.
const REPORTED_CLOCK: [u32; 2] = [
    1 << 29 | 1 << 15 | 1 << 14 | 1 << 13 | 1 << 3 | 1 << 1 | 1 << 0,
    1 << 10,
];

/// In each reset word, the bits a change of which is reported: the eFuse controller, SPI0 and
/// SPI1, `SYSTEM_TIMERS_RST`, and the USB Serial/JTAG unit in the second word.
const REPORTED_RESET: [u32; 2] = [1 << 14 | 1 << 1 | 1 << 0, 1 << 10];

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
        let mask = if k < 2 {
            REPORTED_CLOCK
        } else {
            REPORTED_RESET
        };
        (self.words[k] ^ value) & mask[k % 2] != 0
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

    /// Timer group `group`'s clock bit and reset bit: bit 13 for group 0, bit 15 for group 1.
    /// The general timers run them. The group's watchdog does not, and the reset write is not listed.
    pub(crate) fn timg(&self, group: usize) -> (bool, bool) {
        let bit = 1 << if group == 0 { 13 } else { 15 };
        (self.words[0] & bit != 0, self.words[2] & bit != 0)
    }

    /// `SYSTEM_UART_MEM_CLK_EN`. One clock for the RAM of UART0, UART1 and UART2.
    pub(crate) fn uart_mem_clock(&self) -> bool {
        self.words[0] & (1 << 24) != 0
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
        let mut gates = Gates::new();
        assert!(!gates.reports(0x20, 1 << 29), "SYSTIMER_RST");
        assert!(!gates.reports(0x20, 1 << 15 | 1 << 13 | 1 << 3));
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
        assert!(gates.uart_mem_clock());
        assert!(!gates.reports(0x18, CLOCK_RESET[0] & !(1 << 24)));
        gates
            .write(0x18, CLOCK_RESET[0] & !(1 << 24))
            .for_each(drop);
        assert!(!gates.uart_mem_clock());
    }
}
