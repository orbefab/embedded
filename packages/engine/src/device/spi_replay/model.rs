//! A replay of SPI transactions as a device. It holds its pins at SPI mode 0's idle levels for
//! the whole run: `CS` high, `SCLK` and `D0` to `D3` low. The host's replay engine sends its
//! transactions as `Spi` records, each at its end.

use orbe_model::{Cx, Device, Drive, Drives, Error, Params, State, Stop, Time};

/// It drives `CS`, `SCLK`, then `D0` to `D3`.
pub(crate) const OUTPUTS: usize = 6;

#[derive(Clone, Copy, Debug)]
pub struct SpiReplay;

impl State for SpiReplay {}

impl Device for SpiReplay {
    fn new(_params: &Params<'_>) -> Result<SpiReplay, Error> {
        Ok(SpiReplay)
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::new([
            Drive::High,
            Drive::Low,
            Drive::Low,
            Drive::Low,
            Drive::Low,
            Drive::Low,
        ])
    }
}
