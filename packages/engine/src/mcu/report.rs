//! A chip's faults and notes as the diagnostics `diagnostics.json` lists, each PC named by the
//! symbol its part's firmware gives it.

use orbe_mcu::{Fault, Note, Pc};
use orbe_model::Time;

use crate::diagnostic::Diagnostic;
use crate::name::Ref;
use crate::netlist::Chip;

/// The symbol a diagnostic names `pc` by, from the firmware of `chip`. Only an `esp32s3`'s
/// ELF carries symbols.
pub(crate) fn symbolize(chip: &Chip, pc: u32) -> Option<String> {
    match chip {
        Chip::Esp32s3(mcu) => mcu.firmware.symbolize(pc),
        Chip::Ne555(_)
        | Chip::Atmega328p(_)
        | Chip::SpiReplay(_)
        | Chip::St77916(_)
        | Chip::Ssd1306(_)
        | Chip::Hd44780I2c(_)
        | Chip::Hd44780(_)
        | Chip::Cst816(_)
        | Chip::Dht22(_)
        | Chip::Bmp280(_)
        | Chip::Hc595(_)
        | Chip::Max7219(_)
        | Chip::Ws2812(_)
        | Chip::PulsesReplay(_)
        | Chip::Sg90(_) => None,
    }
}

/// Whether the second CPU ran the code at `pc`, the ESP32-S3's APP CPU.
fn app(pc: Pc) -> bool {
    pc.core == 1
}

/// The diagnostic of `fault` at `at`, which ends the run, on `part`.
pub(crate) fn fault(
    part: Ref,
    at: Time,
    fault: Fault,
    symbol: impl Fn(u32) -> Option<String>,
) -> Diagnostic {
    match fault {
        Fault::UnsupportedInstruction { op, reason, pc } => {
            Diagnostic::ChipUnsupportedInstruction {
                part,
                at,
                op,
                reason,
                pc: pc.pc,
                app: app(pc),
                symbol: symbol(pc.pc),
            }
        }
        Fault::UnmappedFetch(pc) => Diagnostic::ChipUnmappedFetch {
            part,
            at,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
        Fault::Halted(pc) => Diagnostic::ChipHalted {
            part,
            at,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
        Fault::UnsupportedSetting {
            peripheral,
            setting,
            pc,
        } => Diagnostic::ChipUnsupportedSetting {
            part,
            at,
            peripheral,
            setting,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
        Fault::Watchdog {
            watchdog,
            stage,
            reset,
        } => Diagnostic::ChipWatchdog {
            part,
            at,
            watchdog,
            stage,
            reset,
        },
        Fault::EfuseProgram(pc) => Diagnostic::ChipEfuseProgram {
            part,
            at,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
        Fault::FlashCommand {
            command,
            reason,
            pc,
        } => Diagnostic::ChipFlashCommand {
            part,
            at,
            command,
            reason,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
        Fault::UnsupportedClock(pc) => Diagnostic::ChipUnsupportedClock {
            part,
            at,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
        Fault::FaultRestart { reason, pc } => Diagnostic::ChipFaultRestart {
            part,
            at,
            reason,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
        Fault::PowerOnRestart => Diagnostic::ChipPowerOnRestart { part, at },
        Fault::UnmappedLoad { address, pc } => Diagnostic::ChipUnmappedLoad {
            part,
            at,
            address,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
    }
}

/// The warning of `note` at `at` on `part`.
pub(crate) fn note(
    part: Ref,
    at: Time,
    note: Note,
    symbol: impl Fn(u32) -> Option<String>,
) -> Diagnostic {
    match note {
        Note::UnmodeledRegister {
            address,
            register,
            pc,
            write,
            kept,
        } => Diagnostic::ChipUnmodeledRegister {
            part,
            at,
            address,
            register,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
            write,
            kept,
        },
        Note::Restart(pc) => Diagnostic::ChipRestart {
            part,
            at,
            pc: pc.pc,
            app: app(pc),
            symbol: symbol(pc.pc),
        },
    }
}
