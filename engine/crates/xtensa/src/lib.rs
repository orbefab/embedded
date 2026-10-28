//! The Xtensa LX6 and LX7 core of the ESP32 and the ESP32-S3.
//!
//! One core serves both chips through a [`Config`] type per chip, [`Esp32`] and [`Esp32s3`].
//! `decode` holds both generated opcode tables and one [`Op`] enum over them. `core` runs the
//! integer ops, loads and stores, branches, `call0`, the windowed calls and returns that need no
//! spill, the special registers, exceptions and the `ICOUNT` debug exception. Every other op ends
//! the run with [`Stop::Unimplemented`]. `bus` is what the core asks of its SoC, and `paged` is
//! the copy-on-write RAM a SoC keeps its memory in.
//!
//! Both chips' `core-isa.h` select the ISA's hardware alignment option, so the hardware does most
//! unaligned loads and stores at their own address. The ISA still lets a chip raise
//! `LoadStoreAlignmentCause` when a part lies in instruction RAM, ROM or XLMI, has the Isolate
//! attribute, or the parts differ in memory type or cache attribute, and does not say what
//! `EXCVADDR` holds for a fault on a later part. So the core splits an unaligned access into
//! aligned bus accesses and stops as [`Missing::Unaligned`] when any part faults. The board probe
//! S5b settles what each region does.
//!
//! Neither chip has an MMU or an MPU, so the ISA's current ring is always 0, and `rfi`, `rsil` and
//! the special-register ops never raise the privileged exception. `PS.UM` only picks the user
//! vector.

#![forbid(unsafe_code)]

mod bus;
mod config;
mod core;
mod decode;
mod paged;
mod special;

pub use bus::{Bus, Cause, Cycle, Exit, Width};
pub use config::{Config, Esp32, Esp32s3};
pub use core::{Core, Missing, Stop, Unimplemented};
pub use decode::{Decoded, Insn, Op};
pub use paged::{PAGE, PagedRam};
pub use special::Sr;
