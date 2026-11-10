//! What a device model is written against: time, pins, the records chips send each other, and
//! the `Device` trait with its role traits.
//!
//! Unstable, 0.x. [docs/engine/model.md](../../../docs/engine/model.md) says what a device
//! implements and what is still to come.

mod device;
mod pins;
mod record;
mod time;

pub use device::{Ack, Cx, Device, Dir, Error, Event, Fields, I2cTarget, Params, State, Value};
pub use pins::{Drive, Drives, Levels, Stop};
pub use record::{Cs, Duty, Lanes, Pulses, Spi, SpiPhase};
pub use time::Time;
