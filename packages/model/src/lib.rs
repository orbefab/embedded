//! What a device model is written against: time, pins, the records chips send each other, and
//! the `Device` trait with its role traits.
//!
//! Unstable, 0.x. [docs/engine/model.md](../../../docs/engine/model.md) says what a device
//! implements and what is still to come.

mod device;
mod pages;
mod pins;
mod record;
mod sha256;
mod time;

pub use device::{
    Ack, Cx, Device, Dir, Drawn, DutyTarget, Error, Event, Fields, I2cTarget, Idle, Panel, Params,
    PulseTarget, Shows, SpiTarget, State, Value, View,
};
pub use pages::Pages;
pub use pins::{Drive, Drives, Levels, Stop};
pub use record::{Cs, Duty, Lanes, Pulses, Spi, SpiPhase};
pub use sha256::Sha256;
pub use time::Time;
