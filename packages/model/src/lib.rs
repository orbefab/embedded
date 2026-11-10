//! What a device model is written against: time, pins, the records chips send each other, and
//! the `Device` trait with its role traits.
//!
//! Unstable, 0.x. [docs/engine/model.md](../../../docs/engine/model.md) says what a device
//! implements and what is still to come.

mod pins;
mod record;
mod time;

pub use pins::{Drive, Drives, Levels, Stop};
pub use record::{Cs, Duty, Lanes, Pulses, Spi, SpiPhase};
pub use time::Time;
