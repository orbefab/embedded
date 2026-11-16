//! What the core asks of its chip.
//!
//! The core owns the register file, `SREG`, the stack pointer, the program
//! counter and the decoded flash. It names no vendor register.
//!
//! Flash stays in the core. Every instruction reads the decoded image, `LPM`
//! reads that image, and `SPM` only logs, so a fetch on this trait would be a
//! call on the hottest path for work that is not the chip's. The chip loads an
//! image with [`Core::load_words`](crate::Core::load_words).
//!
//! Flag writes (`SEI`, `RETI`, the ALU) stay in the core. A data-space read or
//! write of the `SREG` address is the chip mapping that address onto the core.
//! `BREAK` sets the core's stopped flag and calls nothing here. The chip reads
//! [`Core::is_stopped`](crate::Core::is_stopped).
//!
//! [`Core::step`](crate::Core::step) and [`Core::run_free`](crate::Core::run_free)
//! are generic over [`Bus`], so the instruction loop monomorphizes in the chip
//! crate. No `dyn Bus` sits on that path.

use crate::core::Core;

/// Data-space address of I/O register 0. `IN` and `OUT` add this to the I/O address in the word.
pub const IO_BASE: u16 = 0x0020;

/// One implementation per chip.
pub trait Bus {
    /// The byte at data-space `addr`.
    ///
    /// Addresses `0x0000`–`0x001F` are the register file. The chip maps `SREG` and the stack
    /// pointer onto the core. SRAM and I/O are the chip's.
    fn load(&mut self, core: &Core, addr: u16) -> u8;

    /// Writes `value` at data-space `addr`. `cycles` is the length of the storing instruction,
    /// so a peripheral can stamp the edge that ends it.
    fn store(&mut self, core: &mut Core, addr: u16, value: u8, cycles: u64);

    /// What [`Bus::load`] would return, and it changes nothing.
    fn peek(&self, core: &Core, addr: u16) -> u8;

    /// A load of `addr` changes nothing, reads what [`Bus::peek`] reads, and that byte stays
    /// until the next event, a store, or a host call. `IN`, `LDS`, `SBIC` and `SBIS` of a
    /// steady address run inside a free run.
    fn steady(&self, addr: u16) -> bool;

    /// The byte a load of `addr` would return at the core's cycle, when that load changes nothing.
    ///
    /// A free run leaves the bus cycle behind the core. [`Bus::peek`] reads the bus cycle, so a
    /// byte that moves with the core's cycle is not a peek. `None` means the load writes, or the
    /// byte is not fixed by that cycle. `IN` of `Some` runs inside the free run.
    fn projected(&self, core: &Core, addr: u16) -> Option<u8> {
        let _ = (core, addr);
        None
    }

    /// A store of `value` at `addr` would change nothing the chip keeps. An idle `OUT` or `STS`
    /// runs inside a free run and writes nothing.
    fn store_idle(&self, core: &Core, addr: u16, value: u8) -> bool;

    /// Sets or clears bit `bit` at `addr`. The chip applies write-one-to-clear where a register
    /// has it. `SBI` and `CBI` are the callers.
    fn write_bit(&mut self, core: &mut Core, addr: u16, bit: u8, set: bool);

    /// `SLEEP` ran. Arms the chip's sleep mode when that mode is enabled, and
    /// reports whether the core should suspend.
    fn sleep(&mut self) -> bool;

    /// `WDR` ran. The chip restarts its watchdog timer.
    fn watchdog_reset_timer(&mut self);

    /// Stall cycles the instruction that just retired still has to add, then clears them.
    ///
    /// An ordinary instruction has none. The impl writes the clear only when the stall is
    /// non-zero, so that instruction does not store the stall byte.
    /// [`Core::step`](crate::Core::step) adds a non-zero result.
    /// A free run does not ask: the chip starts one only when the stall is zero.
    fn take_stall(&mut self) -> u8;
}
