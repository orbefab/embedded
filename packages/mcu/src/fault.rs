/// Where a chip's CPU was: the address of the instruction it ran, and which CPU ran it, 0 for
/// the first. On the ESP32-S3, 0 is the PRO CPU and 1 the APP CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pc {
    pub pc: u32,
    pub core: u8,
}

/// Why a chip's run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A CPU reached `op` at `pc`, which the model does not run there.
    UnsupportedInstruction {
        op: &'static str,
        reason: Unsupported,
        pc: Pc,
    },
    /// A CPU fetched code where the model maps no memory.
    UnmappedFetch(Pc),
    /// A CPU reached a jump to itself that nothing can take it out of.
    Halted(Pc),
    /// The store at `pc` started a `peripheral` transfer with `setting`, or set `setting` on
    /// `peripheral`, which the model does not run.
    UnsupportedSetting {
        peripheral: &'static str,
        setting: Setting,
        pc: Pc,
    },
    /// A stage of a watchdog ran out and reset `reset`.
    Watchdog {
        watchdog: &'static str,
        stage: u8,
        reset: &'static str,
    },
    /// The store at this PC started an eFuse program command, which would burn eFuses.
    EfuseProgram(Pc),
    /// The store at `pc` sent the flash `command`, which the model refuses for `reason`.
    /// `command` is `None` when no one command byte went.
    FlashCommand {
        command: Option<u8>,
        reason: FlashRefusal,
        pc: Pc,
    },
    /// The store at this PC picked a CPU clock the model does not run.
    UnsupportedClock(Pc),
    /// The store at `pc` reset the chip after the firmware recorded `reason`.
    FaultRestart { reason: ResetReason, pc: Pc },
    /// The load at `pc` read `address` in a cache window where no flash is mapped.
    UnmappedLoad { address: u32, pc: Pc },
}

/// Something a chip did that the run goes on past.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// The firmware first read or wrote `register` at `address`, which the model keeps no state
    /// for, from `pc`.
    UnmodeledRegister {
        address: u32,
        register: String,
        pc: Pc,
        write: bool,
        /// The register keeps the write, and the model does not run what it does.
        kept: bool,
    },
    /// The store at this PC reset the chip, which stays in reset to the end of the run.
    Restart(Pc),
}

/// What the model lacks to run an instruction a CPU reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// The op itself.
    Op,
    /// The op at an address that is not a multiple of its size.
    Unaligned,
    /// `l32r` while `LITBASE` is enabled.
    Litbase,
    /// The op as the last of a loop that loops back.
    LoopEnd,
}

impl Unsupported {
    pub fn name(self) -> &'static str {
        match self {
            Unsupported::Op => "op",
            Unsupported::Unaligned => "unaligned",
            Unsupported::Litbase => "litbase",
            Unsupported::LoopEnd => "loop_end",
        }
    }
}

/// What a chip's firmware set that the model does not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// A transfer setting, by name, that the store starting the transfer found.
    Transfer(&'static str),
    /// An interrupt of the peripheral that the model never raises.
    Interrupt,
    /// A register setting, by name, that the model does not run.
    Register(&'static str),
    /// A register of a peripheral the model does not simulate at all. The words say what the
    /// peripheral is, as a maker knows it, such as `the pulse counter`.
    Unsimulated(&'static str),
    /// Sleep.
    Sleep,
}

impl Setting {
    pub fn name(self) -> &'static str {
        match self {
            Setting::Transfer(name) => name,
            Setting::Register(name) => name,
            Setting::Interrupt => "interrupt",
            Setting::Unsimulated(_) => "unsimulated",
            Setting::Sleep => "sleep",
        }
    }
}

/// Why a chip's flash command ends the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashRefusal {
    /// A command the model does not know.
    Unsupported,
    /// A page program or an erase, which the model does not simulate.
    Write,
    /// A write of the flash's status registers, which can lock them for good.
    StatusWrite,
    /// A user command with no command phase.
    NoCommandPhase,
    /// One `SPI_MEM_CMD_REG` write that starts more than one command.
    StartBits,
    /// A read that reaches past the flash's end.
    PastEnd,
    /// An address phase longer than 32 bits.
    AddressBits,
    /// A command started on SPI0, the controller the cache reads the flash through.
    Spi0,
    /// A write that would take one of the flash's pads from SPI0 and SPI1.
    Pads,
    /// A user command while `CS0_DIS` is set or `CS1_DIS` is clear.
    Cs,
}

impl FlashRefusal {
    pub fn name(self) -> &'static str {
        match self {
            FlashRefusal::Unsupported => "unsupported",
            FlashRefusal::Write => "write",
            FlashRefusal::StatusWrite => "status_write",
            FlashRefusal::NoCommandPhase => "no_command_phase",
            FlashRefusal::StartBits => "start_bits",
            FlashRefusal::PastEnd => "past_end",
            FlashRefusal::AddressBits => "address_bits",
            FlashRefusal::Spi0 => "spi0",
            FlashRefusal::Pads => "pads",
            FlashRefusal::Cs => "cs",
        }
    }
}

/// Why firmware reset its chip, as ESP-IDF records it before the reset, when that ends the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetReason {
    Panic,
    InterruptWatchdog,
    TaskWatchdog,
    Watchdog,
    Brownout,
    /// The firmware recorded no reason the model can read.
    Unreadable,
}

impl ResetReason {
    pub fn name(self) -> &'static str {
        match self {
            ResetReason::Panic => "panic",
            ResetReason::InterruptWatchdog => "interrupt_watchdog",
            ResetReason::TaskWatchdog => "task_watchdog",
            ResetReason::Watchdog => "watchdog",
            ResetReason::Brownout => "brownout",
            ResetReason::Unreadable => "unreadable",
        }
    }
}
