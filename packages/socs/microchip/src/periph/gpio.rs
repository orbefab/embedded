//! One GPIO port: `PINx`, `DDRx`, and `PORTx`.
//!
//! A write of one to `PINx` toggles the matching `PORTx` bit. The chip-table
//! strobe mask makes `SBI` write a one in only that bit. A read of `PINx` is
//! the synchronizer's view of the pad: the compare override while `DDR` is
//! set, otherwise the host level or the pull-up (DS40002061B §14.2.4, Table
//! 14-1). Clearing `DDR` reports the level the pin settles to.

use crate::Port;
use crate::mux::PinMux;
use crate::regs::{GPIO_DDR, GPIO_PIN, GPIO_PORT};

use super::Ctx;

#[derive(Debug, Clone)]
pub(crate) struct GpioPort {
    port: Port,
}

impl GpioPort {
    pub(crate) fn new(port: Port) -> Self {
        Self { port }
    }

    pub(crate) fn read(&mut self, reg: u8, mux: &PinMux, cycle: u64) -> u8 {
        self.peek(reg, mux, cycle)
    }

    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        at: u64,
    ) {
        let (ddr, port) = mux.ddr_port(self.port);
        match reg {
            GPIO_PIN => mux.toggle_gpio(self.port, value, ctx, at),
            GPIO_DDR => mux.store_gpio(self.port, value, port, ctx, at),
            GPIO_PORT => mux.store_gpio(self.port, ddr, value, ctx, at),
            _ => {}
        }
    }

    pub(crate) fn peek(&self, reg: u8, mux: &PinMux, cycle: u64) -> u8 {
        let (ddr, port) = mux.ddr_port(self.port);
        match reg {
            GPIO_PIN => mux.pin_bits(self.port, cycle),
            GPIO_DDR => ddr,
            GPIO_PORT => port,
            _ => 0,
        }
    }

    /// `DDRx` and `PORTx` change only at a store. `PINx` is steady once no pad change waits out the
    /// synchronizer, because `pin_bits` reads the cycle until then.
    pub(crate) fn steady(&self, reg: u8, mux: &PinMux, cycle: u64) -> bool {
        match reg {
            GPIO_DDR | GPIO_PORT => true,
            GPIO_PIN => mux.settled(self.port, cycle),
            _ => false,
        }
    }

    pub(crate) fn store_idle(&self, reg: u8, value: u8, mux: &PinMux) -> bool {
        let (ddr, port) = mux.ddr_port(self.port);
        match reg {
            GPIO_DDR => mux.store_skips(self.port, value, port),
            GPIO_PORT => mux.store_skips(self.port, ddr, value),
            _ => false,
        }
    }

    pub(crate) fn poke(&mut self, reg: u8, value: u8, mux: &mut PinMux) {
        let (ddr, port) = mux.ddr_port(self.port);
        match reg {
            GPIO_DDR => mux.poke_gpio(self.port, value, port),
            GPIO_PORT => mux.poke_gpio(self.port, ddr, value),
            _ => {}
        }
    }
}
