//! The AVR instruction set.
//!
//! `core` fetches, decodes and executes. `bus` is what it asks of a chip. A chip
//! crate implements [`Bus`] and the instruction loop monomorphizes there.

#![forbid(unsafe_code)]

mod bus;
mod core;

pub use bus::{Bus, IO_BASE};
pub use core::{Activity, Core, Program, SpmNote};
