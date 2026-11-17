//! What a microcontroller's chip crate implements so the engine can run it: [`Mcu`], what it
//! delivers through its USARTs and its TWI, the records it sends, and the faults and notes it
//! reports.
//!
//! Unstable, 0.x. [docs/engine/mcu.md](../../../docs/engine/mcu.md) says what a chip implements
//! and what the engine keeps.

mod fault;
mod sent;

use orbe_model::{Drive, Stop, Time};

pub use fault::{Fault, FlashRefusal, Note, Pc, ResetReason, Setting, Unsupported};
pub use sent::{Sent, SentKind, SpiPins};

/// A microcontroller on the circuit's clock: its CPUs, its memories and its peripherals behind
/// one image of output drives. `Clone` is the checkpoint, and the engine copies a chip only
/// onto a copy of the same chip, with `clone_from`.
///
/// The engine keeps the contract the run depends on:
/// - `run_until(limit)` runs from the last time it reached and stops at the first time in
///   `(reached, limit]` where its drives may change, it samples, or a record it sent ends. It
///   returns `None` once it has done everything up to `limit`. Two calls to `l1` then `l2`
///   leave the state one call to `l2` does. All of a time's effects are one stop.
/// - A clocked chip runs to cycle `floor(limit_ps * f / 1e12)` and puts a stop at cycle `c` at
///   `ceil(c * 1e12 / f)` ps, counting from its clock domain's last frequency change.
/// - Only `run_until` changes the drives. What `input` and `sample` cause comes at least 1 ps
///   later, at a stop.
/// - `input`, `deliver` and `take_sent` read nothing but the chip and their arguments, so they
///   take two equal chips to two equal chips.
pub trait Mcu: Clone + Send + std::fmt::Debug + 'static {
    /// The datasheet name of each USART it sends on. [`Frame::usart`] indexes this.
    const USARTS: &'static [&'static str];
    /// Whether it has a TWI master, whose phases [`deliver`](Self::deliver) hands over.
    const TWI: bool = false;
    /// Whether it sends records, which [`take_sent`](Self::take_sent) hands over.
    const SENDS: bool = false;

    /// The first time in `(reached, limit]` where the drives may change, it samples, or a record
    /// it sent ends. `None` once everything up to `limit` is done.
    fn run_until(&mut self, limit: Time) -> Option<Stop>;
    /// One drive per output, in the order the engine built its outputs.
    fn drive(&self) -> &[Drive];
    /// Its watches' levels from `at` on, in the order the engine built its watches.
    fn input(&mut self, at: Time, levels: &[bool]);
    /// The volts of each of its samples at its stop at `at`, which asked for them, in the order
    /// the engine built its samples. A chip that never asks takes none.
    fn sample(&mut self, _at: Time, _volts: &[f64]) {}
    /// Hands over each byte a USART sent whose stop bit ended at or before `through`, in the
    /// order they ended on each USART, and each TWI phase that ended by then, in the order they
    /// ended.
    fn deliver(&mut self, through: Time, sent: impl FnMut(Frame), saw: impl FnMut(Phase));
    /// Hands over each record it sent at the stop `run_until` last returned with `record` set,
    /// in the order it sent them. Each ends at that stop.
    fn take_sent(&mut self, _sent: impl FnMut(Sent)) {}
    /// The records it started by `at` and has not ended, where the run ends at `at` and every
    /// stop up to `at` has landed. `self` is the committed chip, and `ahead` its copy run on
    /// from there.
    fn in_flight(&self, _ahead: &Self, _at: Time) -> Vec<Sent> {
        Vec::new()
    }
    /// The fault that ended its run, and its time. The engine stops the run at the first.
    fn fault(&self) -> Option<(Time, Fault)> {
        None
    }
    /// The time the firmware first read output `output`'s pin as a digital input, as far as the
    /// chip has run. A chip that never says reads none, and the engine checks no input of it.
    fn read(&self, _output: usize) -> Option<Time> {
        None
    }
    /// What it noted at or before `through`, each at its time, in the order the engine reports
    /// them.
    fn notes(&self, _through: Time) -> Vec<(Time, Note)> {
        Vec::new()
    }
}

/// A byte one of a chip's USARTs sent, its start bit and the end of its stop bit on the run's
/// clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The index in its chip's [`Mcu::USARTS`].
    pub usart: u8,
    pub byte: u8,
    pub start: Time,
    pub end: Time,
}

/// A phase a chip's TWI master saw, its first bus change and its end on the run's clock, and
/// how many phases the chip delivered before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phase {
    pub phase: I2cPhase,
    pub start: Time,
    pub end: Time,
    pub count: u64,
}

/// What a phase was on the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I2cPhase {
    Start,
    RepeatedStart,
    /// SLA+R/W. `ack` is the target's acknowledge.
    Address {
        address: u8,
        read: bool,
        ack: bool,
    },
    /// A data byte. On a write `ack` is the target's acknowledge, on a read the master's.
    Data {
        byte: u8,
        read: bool,
        ack: bool,
    },
    Stop,
    ArbitrationLost,
    BusError,
}
