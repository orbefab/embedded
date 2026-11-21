//! The code version the cores' decode tables run on.
//!
//! A core keeps each instruction it decoded with the version [`Code::version`] read then, and
//! runs it again without a fetch while the version holds. So the version moves whenever a word a
//! fetch returned may read differently: a store to a 64-byte line of SRAM or RTC fast memory that
//! a fetch read since the last move, a loader's write, or a change to the MMU table that places
//! the flash. The mask ROM never changes. A move forgets every line, since no kept decode holds
//! from before it.
//!
//! A fetch marks its line in SRAM by its SRAM offset, so a store through the data-bus window
//! finds the line a fetch through the instruction-bus window marked. The marks are derived from
//! the fetches, and a clone keeps them with the version, which only costs a move that no table
//! needed.

/// One bit per line.
const LINE: u32 = 64;
const SRAM_LINES: usize = super::soc::SRAM / LINE as usize;
const RTC_LINES: usize =
    (super::soc::RTC_FAST.end - super::soc::RTC_FAST.start) as usize / LINE as usize;

#[derive(Clone, Debug)]
pub(crate) struct Code {
    version: u64,
    sram: [u64; SRAM_LINES / 64],
    rtc_fast: [u64; RTC_LINES / 64],
}

impl Default for Code {
    fn default() -> Self {
        Code {
            version: 0,
            sram: [0; SRAM_LINES / 64],
            rtc_fast: [0; RTC_LINES / 64],
        }
    }
}

fn bit(offset: u32) -> (usize, u64) {
    let line = (offset / LINE) as usize;
    (line / 64, 1 << (line % 64))
}

impl Code {
    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    /// A fetch read the word at SRAM offset `offset`.
    pub(crate) fn fetched_sram(&mut self, offset: u32) {
        let (word, mask) = bit(offset);
        self.sram[word] |= mask;
    }

    /// A fetch read the word at `offset` from the start of RTC fast memory.
    pub(crate) fn fetched_rtc_fast(&mut self, offset: u32) {
        let (word, mask) = bit(offset);
        self.rtc_fast[word] |= mask;
    }

    /// A store wrote SRAM offset `offset`, inside one word.
    #[inline]
    pub(crate) fn stored_sram(&mut self, offset: u32) {
        let (word, mask) = bit(offset);
        if self.sram[word] & mask != 0 {
            self.moved();
        }
    }

    /// A store wrote `offset` from the start of RTC fast memory, inside one word.
    pub(crate) fn stored_rtc_fast(&mut self, offset: u32) {
        let (word, mask) = bit(offset);
        if self.rtc_fast[word] & mask != 0 {
            self.moved();
        }
    }

    /// Something other than a core's store changed what a fetch can read.
    pub(crate) fn moved(&mut self) {
        self.version += 1;
        self.sram = [0; SRAM_LINES / 64];
        self.rtc_fast = [0; RTC_LINES / 64];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_moves_the_version_only_on_a_fetched_line() {
        let mut code = Code::default();
        code.fetched_sram(0x100);
        code.stored_sram(0x140);
        assert_eq!(code.version(), 0, "the next line");
        code.stored_sram(0x13c);
        assert_eq!(code.version(), 1, "the fetched line");
        code.stored_sram(0x100);
        assert_eq!(code.version(), 1, "a move forgets every line");
        code.fetched_rtc_fast(0x1ffc);
        code.stored_sram(0x1ffc);
        assert_eq!(code.version(), 1, "SRAM and RTC fast memory are apart");
        code.stored_rtc_fast(0x1fc0);
        assert_eq!(code.version(), 2);
    }
}
