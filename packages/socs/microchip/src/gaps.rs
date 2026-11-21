//! What the firmware reached that the model does not run.
//!
//! A [`Refusal`] is a setting or an instruction whose run would be wrong, so the chip stops
//! there and runs no further. A [`Noted`] is something the model keeps without running while
//! the run stays right, reported once per run at its first access. Each is stamped with the
//! oscillator cycle from power-on and the word address of the instruction, and an in-run reset
//! keeps both. The checks run once a host turns them on with [`crate::Cpu::watch_gaps`]: a
//! [`crate::Timeline`] does, because it hands the pads over as levels, and a test of a register
//! on a bare [`crate::Cpu`] does not. The checks themselves are in the bus, beside the registers they read, and in
//! [`crate::Cpu`] for the core's instructions. Each check gives its DS40002061B reason there.

/// Why the chip stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The core reached `op`. `unknown` is a word that decodes to no instruction of the AVRe+
    /// set.
    Instruction(&'static str),
    /// The firmware set `setting` on `peripheral`, which the model does not run. `aco_read` is
    /// a read of `ACSR` while the comparator is on.
    Setting {
        peripheral: &'static str,
        setting: &'static str,
    },
    /// The firmware enabled an interrupt of `peripheral` that the model never raises.
    Interrupt { peripheral: &'static str },
    /// `SLEEP` with `SE` set in a reserved sleep mode.
    Sleep,
}

/// The first thing the chip refused, which stopped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub what: Refused,
    /// The oscillator cycle from power-on of the instruction, or of the access that showed it
    /// for a comparator read.
    pub at: u64,
    /// The word address of the instruction.
    pub pc: u16,
}

/// A register the model keeps without running what it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unmodeled {
    /// `TWAR` holds an address the firmware wrote while `TWCR` has `TWEN` and `TWEA` set: the
    /// TWI would answer as a slave, and the model has no slave modes.
    TwiSlave,
    /// A START at a bit rate whose data hold time, `T/16` in the model, is longer than Table
    /// 29-14's 3.45 us maximum, below about 18 kHz.
    TwiHoldTime,
    /// `OSCCAL`, which retunes the RC oscillator that times EEPROM writes. The model keeps the
    /// typical 8 MHz.
    Osccal,
    /// `ADMUX` selects ADC6 or ADC7, which have no pad on the 28-pin package the bridge runs,
    /// so a conversion reads 0 V.
    AdcNoPad(u8),
}

impl Unmodeled {
    /// The register's name.
    pub fn register(self) -> &'static str {
        match self {
            Unmodeled::TwiSlave => "TWAR",
            Unmodeled::TwiHoldTime => "TWBR",
            Unmodeled::Osccal => "OSCCAL",
            Unmodeled::AdcNoPad(_) => "ADMUX",
        }
    }

    /// The register's data address.
    pub fn address(self) -> u16 {
        match self {
            Unmodeled::TwiSlave => 0x00BA,
            Unmodeled::TwiHoldTime => 0x00B8,
            Unmodeled::Osccal => 0x0066,
            Unmodeled::AdcNoPad(_) => 0x007C,
        }
    }

    /// One bit per item, so a run notes each once.
    pub(crate) fn bit(self) -> u8 {
        match self {
            Unmodeled::TwiSlave => 1 << 0,
            Unmodeled::TwiHoldTime => 1 << 1,
            Unmodeled::Osccal => 1 << 2,
            Unmodeled::AdcNoPad(6) => 1 << 3,
            Unmodeled::AdcNoPad(_) => 1 << 4,
        }
    }
}

/// The first access to an [`Unmodeled`] register in a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Noted {
    pub what: Unmodeled,
    /// The oscillator cycle from power-on.
    pub at: u64,
    /// The word address of the instruction.
    pub pc: u16,
    /// A store, not a load.
    pub write: bool,
}

/// What a run noted and the refusal that stopped it, which an in-run reset keeps, and the
/// state a check needs from one access to the next, which it does not.
#[derive(Clone, Debug, Default)]
pub(crate) struct Gaps {
    /// The host asked for the checks.
    pub on: bool,
    pub noted: Vec<Noted>,
    seen: u8,
    pub refusal: Option<Refusal>,
    /// A load of `ACSR` while the comparator was on, at this CPU cycle, that no later access
    /// has yet shown to be a use or a read-modify-write that turns it off, and its refusal.
    pub acsr_read: Option<(u64, Refusal)>,
    /// The firmware stored `TWAR` since the last reset.
    pub twar_set: bool,
}

impl Gaps {
    pub(crate) fn note(&mut self, noted: Noted) {
        let bit = noted.what.bit();
        if self.on && self.seen & bit == 0 {
            self.seen |= bit;
            self.noted.push(noted);
        }
    }

    pub(crate) fn refuse(&mut self, refusal: Refusal) {
        if self.on && self.refusal.is_none() {
            self.refusal = Some(refusal);
            self.acsr_read = None;
        }
    }

    /// What an in-run reset keeps: the notes and the refusal. The registers are back at their
    /// reset values, so the per-access state starts over.
    pub(crate) fn across_reset(&mut self) -> Gaps {
        Gaps {
            on: self.on,
            noted: std::mem::take(&mut self.noted),
            seen: self.seen,
            refusal: self.refusal,
            acsr_read: None,
            twar_set: false,
        }
    }
}
