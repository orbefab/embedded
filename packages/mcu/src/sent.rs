use orbe_model::{Duty, Pulses, Spi, Time};

/// A record a chip sent, and which of its outputs it went out on. An output is an index in the
/// chip's drives. The engine names each output's net, so a receiver on that net takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub start: Time,
    pub end: Time,
    /// The peripheral that sent the record, such as `RMT`, `SPI2` or `LEDC`.
    pub peripheral: &'static str,
    /// The chip's pin that names the link.
    pub pin: &'static str,
    pub kind: SentKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SentKind {
    Spi { spi: Spi, pins: SpiPins },
    Pulses { pulses: Pulses, output: usize },
    Duty { duty: Duty, output: usize },
}

/// The outputs of an SPI host's CS, SCLK and data pins. `None` for a signal on a pad that is
/// none of the chip's outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpiPins {
    pub cs: Option<usize>,
    pub sclk: Option<usize>,
    /// D0 to D3.
    pub data: [Option<usize>; 4],
}
