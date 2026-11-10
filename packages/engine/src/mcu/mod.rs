pub(crate) mod avr;
pub(crate) mod esp32s3;

pub use avr::{Atmega328p, Firmware, Fuses};
pub use esp32s3::{Esp32s3, Esp32s3Firmware};
