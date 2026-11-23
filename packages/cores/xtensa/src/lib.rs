//! The Xtensa LX6 and LX7 core of the ESP32 and the ESP32-S3.
//!
//! One core serves both chips through a [`Config`] type per chip, [`Esp32`] and [`Esp32s3`].
//! `decode` holds both generated opcode tables and one [`Op`] enum over them. `core` runs the
//! integer ops, loads and stores, branches, `call0`, the register windows with their overflow and
//! underflow exceptions, zero-overhead loops, the special registers, exceptions, interrupts and
//! their levels, the `CCOMPARE` timers, `WAITI` and the `ICOUNT` debug exception. It also runs
//! the single-precision FPU, MAC16, the Boolean registers and `s32c1i`, and on the S3 the `q`
//! registers and `cop_ai` user registers as plain state: `ld.qr`, `st.qr`, `mv.qr`, `rur` and
//! `wur`. `PRID` reads what the chip set in the core's [`Reset`], with its reset PC and `VECBASE`.
//! The region protection ops `witlb` and `wdtlb` keep each 512 MB region's attribute for `ritlb1` and
//! `rdtlb1` to read back, and no access is checked against it. `iitlb` and `idtlb` do nothing,
//! and the external registers read 0 through `rer` and drop `wer`. `DBREAKA0`, `DBREAKA1`,
//! `DBREAKC0` and `DBREAKC1` read back what was written. A load or a store that hits one stops
//! the core with [`Stop::Watchpoint`] before the bus access. Every other op ends the run
//! with [`Stop::Unimplemented`], among them the classic `f64*` ops and the S3's `ee.*` vector ops.
//! `bus` is what the core asks of its SoC, and `paged` is the copy-on-write RAM a SoC keeps its
//! memory in.
//!
//! `fpu` holds the floating-point rules, in integer arithmetic, so a result is the same on every
//! host and in wasm. An op of a coprocessor whose `CPENABLE` bit is clear raises
//! [`Cause::Coprocessor0Disabled`] for the FPU or [`Cause::Coprocessor3Disabled`] for `cop_ai`,
//! and the core keeps the op and its PC as [`Core::coprocessor_disabled`]. A handler that sets
//! the bit runs the op again, and an `ee.*` op then stops the run, with the record naming the op
//! that found the coprocessor off.
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
//! On a bus with a code version the core runs straight runs. After a step that takes no
//! interrupt, does not wait or count `ICOUNT`, and runs an op that writes none of `PS`,
//! `INTENABLE`, the pending interrupts, `ICOUNT`, `ICOUNTLEVEL`, `CCOUNT` and the `CCOMPAREn`,
//! it runs the decodes it keeps at that version back to back. Before each op it only marks the
//! step's start and compares the cycle with the run's limit, the next timer match and the cycle
//! the lines can next move at. The run ends at an op that writes one of those, at a trap, at an
//! access that can move the lines, at a PC it does not keep, or at that horizon, and the next
//! step makes every check. Every op, interrupt, exception and timer match lands on the
//! instruction and cycle that a full step before each op gives.
//!
//! Both chips select the hardware alignment option, so an unaligned load or store in data RAM
//! completes at its own address. Where [`Bus::instruction_bus`] says an address is on the
//! instruction bus, the core raises [`Cause::LoadStoreAlignment`] when the address is not aligned
//! for the access, and [`Cause::LoadStoreError`] for a sub-word access or an aligned `l32ai` or
//! `s32ri` there, before the bus sees it. An unaligned `l32ai` or `s32ri` anywhere on the S3 raises
//! `LoadStoreAlignment` and writes nothing, and `s32c1i`, which the ISA lists with them (4.6,
//! p 168), does the same. The classic core still stops as [`Missing::Unaligned`] for those three
//! ops, and both chips stop that way when a later part of some other unaligned
//! access faults. The region is the start address.
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
mod fpu;
mod paged;
mod special;

pub use bus::{Bus, Cause, Cycle, Exit, Reset, Width};
pub use config::{Config, Esp32, Esp32s3, Interrupts};
pub use core::{CoprocessorDisabled, Core, Missing, Stop, Unimplemented};
pub use decode::{Decoded, Insn, Op};
pub use paged::{PAGE, PagedRam};
pub use special::Sr;
