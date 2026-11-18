//! The hardware RNG behind `WDEV_RND_REG`, `0x6003_507c`.
//!
//! IDF's `esp_random`, in `components/esp_hw_support/hw_random.c`, says the generator takes two
//! bits of entropy each APB clock and that a caller must wait 16 APB cycles before the word is
//! new. The function XORs every read it takes while it waits on `CCOUNT`, so two reads in one
//! APB cycle have to return the same word, or those XORs cancel.
//!
//! The model is a 32-bit shift register. Each whole APB cycle shifts two bits in at the bottom,
//! from a splitmix of a fixed seed and the cycle index. A read in the same APB cycle returns the
//! same word. The word, the tick it is settled through and the APB-cycle count are part of the
//! chip, so a rewind replays the stream. The seed is the constant [`SEED`]. Nothing in the
//! netlist and nothing on the host changes it. A board's register is truly random, so no capture
//! of its words can match this stream. The model promises a deterministic, non-zero, well-mixed
//! stream, and it returns that stream whether or not `SYSTEM_WIFI_CLK_RNG_EN` is set.

use super::clock::Tick;

/// `WDEV_RND_REG`, from IDF v5.5.5 `components/soc/esp32s3/include/soc/wdev_reg.h`.
pub(crate) const WDEV_RND_REG: u32 = 0x6003_507c;

/// The fixed seed. A rewind and a second run of the same netlist both start here.
const SEED: u64 = 0xE532_5A17_C0DE_0001;

/// The word at cycle 0, before any APB cycle has shifted bits in. It is not zero.
const RESET_WORD: u32 = 0xA5C3_3C5A;

/// Two new bits for APB cycle `index`, from a splitmix of [`SEED`] and the index.
fn pair(index: u64) -> u32 {
    let mut z = SEED.wrapping_add(index).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) as u32 & 3
}

/// The register after `n` whole APB cycles. The first 16 cycles shift [`RESET_WORD`] out. Later
/// cycles hold only the last 16 pairs.
fn word_at(n: u64) -> u32 {
    if n == 0 {
        return RESET_WORD;
    }
    let (mut word, from) = if n < 16 { (RESET_WORD, 0) } else { (0, n - 16) };
    for index in from..n {
        word = (word << 2) | pair(index);
    }
    word
}

/// The RNG. `word` is current at tick `at`, and `cycles` APB cycles have elapsed by then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rng {
    word: u32,
    at: Tick,
    cycles: u64,
    /// Ticks per APB cycle.
    apb: u64,
}

impl Rng {
    pub(crate) fn new(apb: u64) -> Rng {
        Rng {
            word: RESET_WORD,
            at: Tick(0),
            cycles: 0,
            apb,
        }
    }

    /// Brings `word` forward to the last whole APB cycle at or before `now`.
    fn settle(&mut self, now: Tick) {
        if now.0 <= self.at.0 {
            return;
        }
        let elapsed = (now.0 - self.at.0) / self.apb;
        if elapsed == 0 {
            return;
        }
        self.cycles += elapsed;
        self.word = word_at(self.cycles);
        self.at.0 += elapsed * self.apb;
    }

    /// The word a read at `now` returns. Two reads in one APB cycle return one word.
    pub(crate) fn read(&mut self, now: Tick) -> u32 {
        self.settle(now);
        self.word
    }

    /// The APB clock runs at `apb` ticks per cycle from `now` on. Cycles already counted stay.
    pub(crate) fn reclock(&mut self, now: Tick, apb: u64) {
        self.settle(now);
        self.apb = apb;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shifts `n` pairs into `start`, one step at a time.
    fn shifted(start: u32, from: u64, n: u64) -> u32 {
        let mut word = start;
        for index in from..from + n {
            word = (word << 2) | pair(index);
        }
        word
    }

    #[test]
    fn the_closed_form_matches_shifting_one_cycle_at_a_time() {
        assert_eq!(word_at(0), RESET_WORD);
        assert_ne!(RESET_WORD, 0);
        for n in 0..64 {
            let expect = if n < 16 {
                shifted(RESET_WORD, 0, n)
            } else {
                shifted(0, n - 16, 16)
            };
            assert_eq!(word_at(n), expect, "cycle {n}");
            assert_ne!(word_at(n), 0, "cycle {n}");
        }
        for n in [64, 100, 1778, 1 << 20] {
            assert_eq!(word_at(n), shifted(0, n - 16, 16), "cycle {n}");
            assert_ne!(word_at(n), 0, "cycle {n}");
        }
    }

    #[test]
    fn two_reads_in_one_apb_cycle_match_and_sixteen_cycles_replace_the_word() {
        let mut rng = Rng::new(12);
        assert_eq!(rng.read(Tick(0)), rng.read(Tick(11)));
        assert_ne!(rng.read(Tick(12)), RESET_WORD);
        let fresh = rng.read(Tick(16 * 12));
        assert_ne!(fresh, RESET_WORD);
        assert_eq!(fresh, word_at(16));
        assert_eq!(rng.read(Tick(16 * 12 + 11)), fresh);
    }

    #[test]
    fn a_reclock_keeps_the_cycles_already_counted() {
        let mut rng = Rng::new(12);
        assert_eq!(rng.read(Tick(36)), word_at(3));
        rng.reclock(Tick(40), 6);
        // Four ticks of the old period were left. At 6 ticks each, the next word is one new
        // cycle later, at tick 40 + 2.
        assert_eq!(rng.read(Tick(40)), word_at(3));
        assert_eq!(rng.read(Tick(46)), word_at(4));
    }
}
