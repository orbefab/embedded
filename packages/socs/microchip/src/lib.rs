//! Functional model of the ATmega328P.
//!
//! The instruction loop is `core`. Data-space access goes through `bus`:
//! the device list (indexes resolved when the chip is built), the next event,
//! and the byte-level test peers in `bus::hostside`. `chip` holds the chip
//! table. `irq` serves the lowest pending vector when `SREG.I` is set. `mux`
//! combines pin drivers. `analog` holds the voltages the ADC and the
//! comparator sample. `regs` holds the register ids and the masks in the chip
//! table. `cpu` is the core a host steps. `fuses` holds the three fuse bytes
//! and the start-up, `CLKPR`, watchdog, and brown-out values read out of them.
//! `MCUCR.IVSEL` does not move the vectors. `image` parses Intel HEX and ELF
//! into flash bytes. `timeline` runs the core on a host's clock, in oscillator
//! cycles, and stops it at each change of a pin's drive. `bridge` is the chip on
//! the `orbe-mcu` contract: its package pins, and the timeline in picoseconds.

/// `Clone` for a struct, field by field, with a `clone_from` that reuses each field's buffers.
/// The derived `clone_from` clones a fresh value and drops the old one. The patterns name every
/// field with no `..`, so a field missing from the list does not build.
macro_rules! clone_fields {
    ($ty:ident { $($(#[$attr:meta])* $field:ident),* $(,)? }) => {
        impl Clone for $ty {
            fn clone(&self) -> Self {
                let $ty { $($(#[$attr])* $field),* } = self;
                $ty { $($(#[$attr])* $field: Clone::clone($field)),* }
            }

            fn clone_from(&mut self, source: &Self) {
                let $ty { $($(#[$attr])* $field),* } = self;
                $($(#[$attr])* $field.clone_from(&source.$field);)*
            }
        }
    };
}

mod analog;
mod bridge;
mod bus;
mod chip;
mod cpu;
mod fuses;
mod image;
mod irq;
mod mux;
mod periph;
mod regs;
mod timeline;

pub use bridge::{Bridge, PACKAGE, PORT_PINS, Pad, port_index};
pub use cpu::{Channel, Cpu, LoadError, Parity, ResetKind, SerialFrame, SpiClock, SupplyChannel};
pub use fuses::FuseBytes;
pub use image::{IgnoredRegion, IgnoredSegment, Image, ImageError, parse_elf, parse_hex};
pub use periph::twi::{I2cNote, I2cStamp};
pub use periph::usart::Sent;
pub use periph::{IO_LOG_CAP, IoWrite};
pub use timeline::{Halt, Timeline};

/// Ports B, C, and D, eight pins each. [`Port::pin_id`] values are `0..PIN_COUNT`.
pub const PIN_COUNT: usize = 24;

/// Pin reports retained when the host does not drain the queue.
///
/// A 490 Hz PWM pin changes about 1000 times a second (two edges per cycle).
/// 8192 reports is about eight seconds of one such pin, or a bit over a second
/// of five of them, without a further allocation. Past that, the oldest report
/// is dropped, the newest is kept, and [`Cpu::pin_overflow`] counts the drop.
pub const PIN_QUEUE_CAP: usize = 8192;

/// GPIO port letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Port {
    B,
    C,
    D,
}

impl Port {
    pub const fn index(self) -> usize {
        match self {
            Self::B => 0,
            Self::C => 1,
            Self::D => 2,
        }
    }

    /// Chip pin id `(letter − 'B') × 8 + bit`.
    pub const fn pin_id(self, bit: u8) -> u32 {
        (self.index() as u32) * 8 + (bit & 7) as u32
    }

    /// Inverse of [`Self::pin_id`].
    pub fn from_pin(pin: u32) -> Option<(Self, u8)> {
        let port = match pin / 8 {
            0 => Self::B,
            1 => Self::C,
            2 => Self::D,
            _ => return None,
        };
        Some((port, (pin % 8) as u8))
    }
}

/// Level on a pin.
///
/// [`Level::Released`] is high-Z: the MCU is not driving the pin and the host
/// has released it, with the pull-up off. The wire is then undefined on the
/// die (DS40002061B Table 14-1, tri-state). A `PINx` read of that pin is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Low,
    High,
    Released,
}

/// What a pin's own stage does, apart from anything outside the chip.
///
/// [`PinDrive::High`] and [`PinDrive::Low`] are the output driver: `DDRxn` set, or a
/// peripheral that makes the pin an output. [`PinDrive::PullUp`] is an input with its pull-up
/// on, `PORTxn` set and `MCUCR.PUD` clear (DS40002061B Table 14-1), and `PC6` while it is
/// `RESET` (Table 14-7). [`PinDrive::Off`] is an input with no pull-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinDrive {
    High,
    Low,
    PullUp,
    Off,
}

/// Who drove a pin edge.
///
/// [`PinOrigin::Usart`] is the transmitter override (`TXD`). [`PinOrigin::Spi`]
/// is an SPI override (`MOSI`, `SCK`, or slave `MISO`). A GPIO write on that
/// pin, including one after the peripheral has released it, is
/// [`PinOrigin::Gpio`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinOrigin {
    #[default]
    Gpio,
    Usart,
    Spi,
}

/// A pin, the level the model drove, and the cycle of that edge.
///
/// `pin` is [`Port::pin_id`]. `PB0`–`PB7` are 0–7, `PC` is 8–15, and `PD` is
/// 16–23.
///
/// `cycle` is the cycle of the edge itself. A PWM edge between two instruction
/// boundaries keeps the timer's cycle, not the cycle after the instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinReport {
    pub pin: u32,
    pub level: Level,
    /// The pin's own stage from `cycle` on. A report goes out when this or `level` changes.
    pub drive: PinDrive,
    pub cycle: u64,
    pub origin: PinOrigin,
}

/// Level and drive of each pin id. [`PinLevels::get`] is `None` for an id this die does
/// not have. [`Level::Released`] is a floating input. A pull-up is [`Level::High`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinLevels {
    levels: [Option<(Level, PinDrive)>; PIN_COUNT],
}

impl PinLevels {
    pub(crate) fn from_array(levels: [Option<(Level, PinDrive)>; PIN_COUNT]) -> Self {
        Self { levels }
    }

    pub fn get(self, pin: u32) -> Option<Level> {
        self.both(pin).map(|(level, _)| level)
    }

    pub fn drive(self, pin: u32) -> Option<PinDrive> {
        self.both(pin).map(|(_, drive)| drive)
    }

    pub(crate) fn both(self, pin: u32) -> Option<(Level, PinDrive)> {
        self.levels.get(pin as usize).copied().flatten()
    }
}

/// Integration tests call [`Cpu`] hooks that are `pub(crate)`.
///
/// Not a host API.
#[doc(hidden)]
pub mod test_hooks {
    use crate::Cpu;
    use crate::cpu::ResetKind;

    pub fn set_io_clock_running(cpu: &mut Cpu, running: bool) {
        cpu.set_io_clock_running(running);
    }

    pub fn adc_noise_reduction(cpu: &mut Cpu) {
        cpu.adc_noise_reduction();
    }

    /// Power-on here restarts this `Cpu`'s cycle counter.
    pub fn reset_cpu(cpu: &mut Cpu, kind: ResetKind) {
        match kind {
            ResetKind::PowerOn => cpu.reset(),
            ResetKind::External => cpu.pulse_reset(),
            other => cpu.reset_as(other),
        }
    }
}
