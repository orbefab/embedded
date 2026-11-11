use crate::Time;

/// What one output pin does. The drives of a chip's outputs are all the circuit's stamp reads,
/// and the only thing its restart test compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    /// Through the high stage from the chip's own VCC pin.
    High,
    /// Through the low stage into the chip's own GND pin.
    Low,
    /// Through the output's pull-up from the chip's own VCC pin.
    PullUp,
    /// No stage, only the leakage every drive state stamps.
    Off,
}

impl Drive {
    const fn bits(self) -> u64 {
        match self {
            Drive::Off => 0,
            Drive::Low => 1,
            Drive::High => 2,
            Drive::PullUp => 3,
        }
    }

    const fn from_bits(bits: u64) -> Drive {
        match bits & 3 {
            0 => Drive::Off,
            1 => Drive::Low,
            2 => Drive::High,
            _ => Drive::PullUp,
        }
    }
}

/// One drive per output a device declares, in declaration order, two bits each. An output past
/// the ones set reads [`Drive::Off`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drives(u64);

impl Drives {
    /// The most outputs a device drives.
    pub const MAX: usize = 32;

    /// Every output off.
    pub const OFF: Drives = Drives(0);

    /// Panics when `N` is over [`Drives::MAX`].
    pub const fn new<const N: usize>(drives: [Drive; N]) -> Drives {
        assert!(N <= Drives::MAX, "a device drives at most 32 outputs");
        let mut bits = 0;
        let mut k = 0;
        while k < N {
            bits |= drives[k].bits() << (2 * k);
            k += 1;
        }
        Drives(bits)
    }

    /// The drive of output `output`, which is below [`Drives::MAX`].
    pub const fn get(self, output: usize) -> Drive {
        assert!(output < Drives::MAX, "a device drives at most 32 outputs");
        Drive::from_bits(self.0 >> (2 * output))
    }
}

/// The levels of a device's watches, in declaration order. Watch `k` reads high when bit `k` is
/// set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Levels(u64);

impl Levels {
    /// The most watches a device reads.
    pub const MAX: usize = 64;

    /// Panics when `levels` is longer than [`Levels::MAX`].
    pub fn from_slice(levels: &[bool]) -> Levels {
        assert!(
            levels.len() <= Levels::MAX,
            "a device reads at most 64 watches"
        );
        Levels(
            levels
                .iter()
                .enumerate()
                .fold(0, |bits, (k, &high)| bits | (u64::from(high) << k)),
        )
    }

    /// Whether watch `watch`, below [`Levels::MAX`], reads high.
    pub const fn high(self, watch: usize) -> bool {
        assert!(watch < Levels::MAX, "a device reads at most 64 watches");
        (self.0 >> watch) & 1 == 1
    }
}

/// Where a run stopped before its limit: at `at` its drives may change, it samples, or a record
/// it sent ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    pub at: Time,
    /// Whether it samples at `at`. It reads every one of its samples on the step's solution
    /// there, before any change at `at`.
    pub sample: bool,
    /// Whether a record it sent ends at `at`. The drives do not change at that stop for the
    /// record's sake.
    pub record: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_drive_reads_back_from_its_output() {
        let all = [
            Drive::High,
            Drive::Low,
            Drive::PullUp,
            Drive::Off,
            Drive::Low,
        ];
        let drives = Drives::new(all);
        for (k, drive) in all.into_iter().enumerate() {
            assert_eq!(drives.get(k), drive, "output {k}");
        }
        assert_eq!(drives.get(Drives::MAX - 1), Drive::Off);
        assert_eq!(Drives::OFF.get(0), Drive::Off);
    }

    #[test]
    fn each_level_reads_back_from_its_watch() {
        let levels = Levels::from_slice(&[true, false, false, true]);
        assert_eq!(
            [0, 1, 2, 3, 63].map(|k| levels.high(k)),
            [true, false, false, true, false]
        );
    }
}
