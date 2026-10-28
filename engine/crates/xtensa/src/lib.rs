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
//! Both chips select the hardware alignment option, so an unaligned load or store in data RAM
//! completes at its own address. On the S3, instruction RAM (`0x40370000..0x403e0000`) and mask
//! ROM (`0x40000000..0x40060000`) raise [`Cause::LoadStoreAlignment`] when the address is not
//! aligned for the access, and [`Cause::LoadStoreError`] for a sub-word access or an aligned
//! `l32ai` or `s32ri` there. The board probed `0x40378dc0` and `0x40000400`. The rest of each
//! window is the memory map. An unaligned `l32ai` or `s32ri` anywhere on the S3 raises
//! `LoadStoreAlignment` and writes nothing. The classic core still stops as [`Missing::Unaligned`]
//! for those two ops, and both chips stop that way when a later part of some other unaligned
//! access faults. The region is the start address. Cached instruction ROM and RTC instruction RAM
//! are not classified.
//!
//! A vector costs 4 cycles on both chips. No capture isolates that entry. `rsr` and `wsr` cost
//! one cycle, and the S3 board spends more than that on `epc1` inside an exception handler.
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
