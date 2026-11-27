//! What a microcontroller's chip crate implements so the engine can run it: [`Mcu`], what it
//! delivers through its USARTs and its TWI, the records it sends, and the faults and notes it
//! reports.
//!
//! Unstable, 0.x. [docs/engine/mcu.md](../../../docs/engine/mcu.md) says what a chip implements
//! and what the engine keeps.

mod fault;
mod sent;

use orbe_model::air::{Ack, AirFrame, Channel, Signal};
use orbe_model::{Drive, Quantity, Readings, Stop, Time};

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
/// - Only `run_until` changes the drives, the pull-downs and [`held`](Mcu::held). What `input`,
///   `sample` and `hear` cause comes at least 1 ps later, at a stop.
/// - `input`, `deliver`, `take_sent` and `take_aired` read nothing but the chip and their
///   arguments, so they take two equal chips to two equal chips.
pub trait Mcu: Clone + Send + std::fmt::Debug + 'static {
    /// The datasheet name of each USART it sends on. [`Frame::usart`] indexes this.
    const USARTS: &'static [&'static str];
    /// Whether it has a TWI master, whose phases [`deliver`](Self::deliver) hands over.
    const TWI: bool = false;
    /// Whether it sends records, which [`take_sent`](Self::take_sent) hands over.
    const SENDS: bool = false;
    /// The world quantities it reads at a sample, each when the scene declares it. None, unless
    /// it says otherwise.
    const WORLD: &'static [Quantity] = &[];

    /// The first time in `(reached, limit]` where the drives may change, it samples, or a record
    /// it sent ends. `None` once everything up to `limit` is done.
    fn run_until(&mut self, limit: Time) -> Option<Stop>;
    /// One drive per output, in the order the engine built its outputs.
    fn drive(&self) -> &[Drive];
    /// Whether each output pulls down to the chip's own GND pin, in the order the engine built
    /// its outputs. Only an output whose drive is [`Drive::PullUp`] or [`Drive::Off`] pulls
    /// down, and one with [`Drive::PullUp`] then pulls to both rails. Like the drives, it
    /// changes only at a stop. A chip that never pulls down returns the default, which is empty.
    fn pulled_down(&self) -> &[bool] {
        &[]
    }
    /// Whether each output whose drive is [`Drive::PullUp`] uses its alternate (strong) pull-up
    /// resistor instead of the pad's weak one, in the order the engine built its outputs (such as
    /// the ESP32-S3 USB PHY's D+ pull-up on GPIO20). Like the drives, it changes only at a stop.
    /// A chip with only one pull-up per output keeps the default, which is empty.
    fn strong_pull_up(&self) -> &[bool] {
        &[]
    }
    /// Whether a reset holds it, which changes its own supply current. Like the drives, it
    /// changes only at a stop. A chip that never says is never held.
    fn held(&self) -> bool {
        false
    }
    /// Its watches' levels from `at` on, in the order the engine built its watches.
    fn input(&mut self, at: Time, levels: &[bool]);
    /// The volts of each of its samples at its stop at `at`, which asked for them, in the order
    /// the engine built its samples, and each of its [`WORLD`](Self::WORLD) quantities the scene
    /// declares, as it stands just before `at`. A chip that never asks takes none.
    fn sample(&mut self, _at: Time, _volts: &[f64], _world: &Readings) {}
    /// Hands over each byte a USART sent whose stop bit ended at or before `through`, in the
    /// order they ended on each USART, and each TWI phase that ended by then, in the order they
    /// ended.
    fn deliver(&mut self, through: Time, sent: impl FnMut(Frame), saw: impl FnMut(Phase));
    /// Hands over each record it sent at the stop `run_until` last returned with `record` set,
    /// in the order it sent them. Each ends at that stop.
    fn take_sent(&mut self, _sent: impl FnMut(Sent)) {}
    /// The bytes the targets answered on MISO for the SPI record it sent that ended at `at`,
    /// one for each byte the record reads. A chip sends at most one record that reads per stop,
    /// and the engine hands this over at that stop before the chip runs on. A byte no target
    /// drives is `0xFF`. A chip whose records never read keeps the default, which does nothing.
    fn answered(&mut self, _at: Time, _miso: &[u8]) {}
    /// The records it started by `at` and has not ended, where the run ends at `at` and every
    /// stop up to `at` has landed. `self` is the committed chip, and `ahead` its copy run on
    /// from there.
    fn in_flight(&self, _ahead: &Self, _at: Time) -> Vec<Sent> {
        Vec::new()
    }
    /// The channel its radio listens on now, or `None` while its RF is off. A chip with no
    /// radio keeps the default, which never listens.
    fn tuned(&self) -> Option<Channel> {
        None
    }
    /// Hands over the frame it put on the air that ended at the stop `run_until` last returned
    /// with `record` set, if one did. A radio sends one frame at a time, so at most one ends at a
    /// stop. A chip with no radio keeps the default, which sends none.
    fn take_aired(&mut self, _aired: impl FnMut(AirFrame)) {}
    /// Another station's frame on the channel it is [`tuned`](Self::tuned) to, which ended at
    /// `at`, heard at `signal`. It returns whether it acknowledges it. What the frame causes
    /// comes at least 1 ps later, at a stop. A chip with no radio keeps the default, which hears
    /// nothing and acknowledges nothing.
    fn hear(&mut self, _at: Time, _frame: &AirFrame, _signal: Signal) -> Ack {
        Ack::None
    }
    /// Whether any station acknowledged the frame it put on the air that ended at `at`. The
    /// engine hands this over at that stop before the chip runs on, as it does
    /// [`answered`](Self::answered). A chip with no radio keeps the default, which does nothing.
    fn acked(&mut self, _at: Time, _ack: Ack) {}
    /// The fault that ended its run, and its time. The engine stops the run at the first.
    fn fault(&self) -> Option<(Time, Fault)> {
        None
    }
    /// The time the firmware first read output `output`'s pin as a digital input since the
    /// pin's drive last turned off, as far as the chip has run. A read while the drive is not
    /// off reads a held level, so it does not count. Once the drive is no longer off, it is
    /// still the read of the stretch that just ended. A chip that never says reads none, and
    /// the engine checks no input of it.
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
