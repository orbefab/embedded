//! What the core asks of its SoC. The core knows no SoC and owns no memory.

/// A core's cycle on its own clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cycle(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Byte,
    Half,
    Word,
}

impl Width {
    pub const fn bytes(self) -> u32 {
        match self {
            Width::Byte => 1,
            Width::Half => 2,
            Width::Word => 4,
        }
    }
}

/// The XEA2 exception causes, as `EXCCAUSE` holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Cause {
    IllegalInstruction = 0,
    Syscall = 1,
    InstructionFetchError = 2,
    LoadStoreError = 3,
    Level1Interrupt = 4,
    Alloca = 5,
    IntegerDivideByZero = 6,
    LoadStoreAlignment = 9,
    InstFetchProhibited = 20,
    LoadProhibited = 28,
    StoreProhibited = 29,
}

/// Why an access did not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The access did not happen. The core stops before the instruction that made it, with that
    /// instruction's PC, and runs it again when it next runs. For a later part of an unaligned
    /// access, the earlier parts did happen, and the rerun makes them again. That matters only
    /// where an access has a side effect, such as a peripheral register.
    Stop,
    /// The access faulted. The core raises this exception with the access's address in
    /// `EXCVADDR`. An unaligned access whose part faults stops as `Missing::Unaligned` on the
    /// classic core, and also on the S3 when the address is not on its instruction bus.
    Fault(Cause),
}

/// One implementation per SoC. `Core::run_until` is generic over it, so no `dyn Bus` sits on the
/// instruction path.
pub trait Bus {
    /// The little-endian word at `addr`, a multiple of 4. The core fetches a second word only
    /// when the instruction the first byte names runs into it, so a two-byte op just before an
    /// unmapped word does not fault.
    fn fetch(&mut self, now: Cycle, addr: u32) -> Result<u32, Exit>;
    /// The value at `addr`, a multiple of `width`'s size, zero-extended. The core splits an
    /// unaligned access into aligned ones.
    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit>;
    /// Writes the low `width` bytes of `value` at `addr`, a multiple of `width`'s size.
    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit>;
    /// The levels of the interrupt lines at `now`, one bit per interrupt number. The core reads
    /// the bits of the external level, edge and NMI interrupts, once before each instruction.
    /// The lines may change at an access or at [`Bus::next_event`], so that a run split at any
    /// cycle sees the same lines as one run. The core compares each call with the last one, so an
    /// edge or NMI line must read low on one call and high on a later one.
    fn interrupts(&mut self, now: Cycle) -> u32;
    /// The first cycle after `now` at which the lines can change with no access, or `None`. A
    /// core in `WAITI` skips to the earliest of this, its own timer match and the run's limit.
    fn next_event(&mut self, now: Cycle) -> Option<Cycle>;
}
