//! What the core asks of its SoC. The core knows no SoC and owns no memory.

/// Which of a chip's two cores, by the `PRID` it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoreId {
    Pro,
    App,
}

impl CoreId {
    /// `0xCDCD` on the PRO CPU and `0xABAB` on the APP CPU. IDF v5.5.5,
    /// `components/xtensa/include/xtensa_context.h`.
    pub const fn prid(self) -> u32 {
        match self {
            CoreId::Pro => 0xcdcd,
            CoreId::App => 0xabab,
        }
    }
}

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
    /// An FPU op with `CPENABLE` bit 0 clear.
    Coprocessor0Disabled = 32,
    /// On the S3, a `cop_ai` op with `CPENABLE` bit 3 clear.
    Coprocessor3Disabled = 35,
}

impl Cause {
    /// `CoprocessornDisabled` for coprocessor `n`, of the two a chip here has.
    pub const fn coprocessor_disabled(n: u8) -> Option<Cause> {
        match n {
            0 => Some(Cause::Coprocessor0Disabled),
            3 => Some(Cause::Coprocessor3Disabled),
            _ => None,
        }
    }
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
    /// Whether a word store at `addr`, a multiple of 4, would complete, without making it.
    /// `s32c1i` asks when its compare fails, since it needs store permission whether or not it
    /// stores. The default allows every address, which is right for a bus whose `load` refuses
    /// every address its `store` refuses, since `s32c1i` loads first.
    fn check_store(&mut self, _now: Cycle, _addr: u32) -> Result<(), Exit> {
        Ok(())
    }
    /// The levels of the interrupt lines at `now`, one bit per interrupt number. The core reads
    /// the bits of the external level, edge and NMI interrupts before an instruction when this
    /// run has not read them yet, when the cycle has reached [`Bus::next_event`], or when the
    /// previous instruction stored to an address [`Bus::access_moves_lines`] names or loaded
    /// from one [`Bus::load_moves_lines`] names. Between those reads the lines stay as the last
    /// read left them. The lines may change at an access or at [`Bus::next_event`], so that a run
    /// split at any cycle sees the same lines as one run. The core compares each call with the
    /// last one, so an edge or NMI line must read low on one call and high on a later one.
    fn interrupts(&mut self, now: Cycle) -> u32;
    /// The first cycle after `now` at which the lines can change with no access, or `None`. A
    /// poll asks at the cycle of the read. A one-step read does not ask. A core in `WAITI` asks
    /// from `idle` and skips to the earliest of this, its own timer match and the run's limit.
    fn next_event(&mut self, now: Cycle) -> Option<Cycle>;
    /// Whether the core returns [`Stop::Yield`](crate::Stop::Yield) before its next step. The
    /// core does not ask between the ops of a straight run, which ends at an access that
    /// [`Bus::access_moves_lines`] or [`Bus::load_moves_lines`] names. So a bus turns this on
    /// only in such an access, or between runs.
    fn yields(&self) -> bool {
        false
    }

    /// Records the start of the instruction at `now` when the core does not read the lines,
    /// which includes each op of a straight run. A bus that timestamps a store from that cycle
    /// records it here. The default does nothing.
    fn begin_step(&mut self, _now: Cycle) {}

    /// Whether a data access at `addr` can change what [`Bus::interrupts`] returns. The default
    /// is every access. A memory that cannot move the lines answers `false`, and the core keeps
    /// the lines it last read until the next event.
    fn access_moves_lines(&self, _addr: u32) -> bool {
        true
    }

    /// Whether a load from `addr` can change what [`Bus::interrupts`] returns. The default is
    /// [`Bus::access_moves_lines`]. A register whose load changes nothing a poll reads answers
    /// `false` here even when a store to it can move the lines.
    fn load_moves_lines(&self, addr: u32) -> bool {
        self.access_moves_lines(addr)
    }

    /// A count that moves whenever a word [`Bus::fetch`] returned may read differently: a store
    /// or a loader wrote its bytes, or the map that placed it changed. While the count holds, a
    /// fetch of an address that returned a word returns that word again, unless
    /// [`Bus::fetch_held`] is set. The core then runs an instruction it decoded at that count
    /// without fetching it, and runs straight runs of them. `None`, the default, keeps no
    /// count, and the core fetches every instruction, compares its bytes and runs no straight
    /// run.
    fn code_version(&self) -> Option<u64> {
        None
    }

    /// Whether a fetch now returns [`Exit::Stop`] whatever its address. The core asks before an
    /// instruction it runs without fetching at the start of a step, and stops there as the fetch
    /// would. It does not ask between the ops of a straight run, so a bus sets this only in an
    /// access that [`Bus::access_moves_lines`] or [`Bus::load_moves_lines`] names, or between
    /// runs. The default is never.
    fn fetch_held(&self) -> bool {
        false
    }
}
