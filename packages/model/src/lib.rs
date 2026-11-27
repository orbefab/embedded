//! What a device model is written against: time, pins, the records chips send each other, the
//! frames radios put on the air, and the `Device` trait with its role traits.
//!
//! Unstable, 0.x. [docs/engine/model.md](../../../docs/engine/model.md) says what a device
//! implements and what is still to come.

/// The air: its own module, since its [`air::Ack`] is not an I2C [`Ack`].
pub mod air;
mod device;
mod pages;
mod pins;
mod record;
mod sha256;
mod time;
mod world;

pub use device::{
    Ack, Cx, Device, Dir, Draw, Drawn, DutyTarget, Error, Event, Fields, I2cTarget, Idle, Panel,
    Params, PulseTarget, Settings, Shows, SpiTarget, State, Value, View,
};
pub use pages::Pages;
pub use pins::{Drive, Drives, Levels, Stop};
pub use record::{Cs, Duty, Lanes, Miso, Pulses, Spi, SpiPhase};
pub use sha256::Sha256;
pub use time::Time;
pub use world::{Quantity, Readings, Row};
