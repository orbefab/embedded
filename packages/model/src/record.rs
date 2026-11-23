//! The records chips send each other: an SPI transaction, a pulse train and a PWM duty. A record
//! says what was sent. The host says which pins it went out on.

use std::sync::Arc;

/// One SPI transaction as the host's registers set it.
#[derive(Clone, Debug, PartialEq)]
pub struct Spi {
    /// The phases before the data, such as a command and an address, in the order they ran.
    pub phases: Vec<SpiPhase>,
    /// The data's lanes.
    pub lanes: Lanes,
    pub data: Arc<[u8]>,
    pub clock_hz: u32,
    /// The chip select at the transaction's end.
    pub cs: Cs,
    /// What the host reads back on MISO, D1.
    pub miso: Miso,
}

/// What an SPI host reads on MISO, D1, in one transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Miso {
    /// Nothing.
    None,
    /// One byte for each byte of data, as it sends that byte, on one lane.
    Duplex,
    /// This many bytes on one lane after the data, while it sends nothing.
    Read(u32),
}

impl Spi {
    /// How many bytes the host reads.
    pub fn reads(&self) -> usize {
        match self.miso {
            Miso::None => 0,
            Miso::Duplex => self.data.len(),
            Miso::Read(n) => n as usize,
        }
    }
}

/// One phase before the data: `bits` bits of `value`, most significant first, on `lanes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpiPhase {
    /// From 1 to 32.
    pub bits: u8,
    pub lanes: Lanes,
    /// Below `2^bits`.
    pub value: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lanes {
    One,
    Two,
    Four,
}

impl Lanes {
    pub fn count(self) -> u8 {
        match self {
            Lanes::One => 1,
            Lanes::Two => 2,
            Lanes::Four => 4,
        }
    }
}

/// Whether a transaction leaves its chip select asserted. A data-only transaction after one
/// that ends `Held` continues it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cs {
    Held,
    Released,
}

/// A pulse train, as an RMT channel sends it: the idle level, then each pulse's level and
/// duration in picoseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Pulses {
    pub idle: bool,
    pub pulses: Arc<[(bool, u64)]>,
}

/// A PWM duty as an LEDC channel sets it: `duty` of `2^resolution` steps at `hz`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duty {
    pub duty: u32,
    /// Below 32.
    pub resolution: u8,
    pub hz: u32,
}
