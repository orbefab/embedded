//! The Xtensa LX6 and LX7 core of the ESP32 and the ESP32-S3.
//!
//! One core serves both chips through a [`Config`] type per chip, [`Esp32`] and [`Esp32s3`].
//! `decode` holds both generated opcode tables and one [`Op`] enum over them. `core` runs the
//! integer ops, loads and stores, branches, `call0`, the register windows with their overflow and
//! underflow exceptions, zero-overhead loops, the special registers, exceptions, interrupts and
//! their levels, the `CCOMPARE` timers, `WAITI` and the `ICOUNT` debug exception. Every other op
//! ends the run with [`Stop::Unimplemented`]. `bus` is what the core asks of its SoC, and `paged`
//! is the copy-on-write RAM a SoC keeps its memory in.
//!
//! The window overflow check is the ISA's: each op tests the quarters of the window its address
//! registers name, so a frame spills on the first reference to it. The decoder gives each op its
//! [`Insn::reach`].
//!
//! `WAITI` suspends the core until it takes an interrupt. While suspended it skips to the bus's
//! next event, its next timer match or the run's limit, so an idle core costs no instructions.
//! The ISA lets a chip leave `WAITI` at any time (8.3.341, p 643). This core leaves only to take
//! an interrupt.
//!
//! Both chips' `core-isa.h` select the ISA's hardware alignment option, so the hardware does most
//! unaligned loads and stores at their own address. The ISA still lets a chip raise
//! `LoadStoreAlignmentCause` when a part lies in instruction RAM, ROM or XLMI, has the Isolate
//! attribute, or the parts differ in memory type or cache attribute, and does not say what
//! `EXCVADDR` holds for a fault on a later part. So the core splits an unaligned access into
//! aligned bus accesses and stops as [`Missing::Unaligned`] when any part faults. A board probe
//! settles what each region does.
//!
//! Neither chip has an MMU or an MPU, so the ISA's current ring is always 0, and the privileged
//! ops, such as `rfi`, `rsil`, `waiti`, `rotw`, `rfwo`, `l32e` and the special-register ops, never
//! raise the privileged exception. `PS.UM` only picks the user vector.

#![forbid(unsafe_code)]

mod bus;
mod config;
mod core;
mod decode;
mod paged;
mod special;

pub use bus::{Bus, Cause, Cycle, Exit, Width};
pub use config::{Config, Esp32, Esp32s3, Interrupts};
pub use core::{Core, Missing, Stop, Unimplemented};
pub use decode::{Decoded, Insn, Op};
pub use paged::{PAGE, PagedRam};
pub use special::Sr;
