//! The ESP32-S3 behind the seam. Each GPIO with a net becomes an output and a watch around
//! `sim_esp32::s3::Esp32s3`, its console goes to `serial.csv` as `USB_SERIAL_JTAG`, each SPI2
//! transfer is a `Spi` record, and its 480 MHz tick converts between picoseconds and its timeline.
//!
//! A console packet has no line timing: every byte of one starts and ends at the cycle the
//! firmware flushed it.

use sim_core::diagnostic::{ChipSetting, Diagnostic, FlashRefusal, ResetReason, Unsupported};
use sim_core::link::{Cs, Lanes, Record, RecordKind, Spi, SpiNets, SpiPhase};
use sim_core::name::Ref;
use sim_core::netlist::{Esp32s3, Esp32s3Firmware, NetId, PartId, Value};
use sim_core::time::Time;
use sim_esp32::s3::{
    self, Fault, Halt, PadDrive, Refused, ResetCause, TICK_HZ, Tick, Touch, Transfer,
};
use sim_xtensa::Missing;

use super::{Chip, Drive, Frame, Output, Stage, Stop, Takes};
use crate::mna::Node;
use crate::watch::{Signal, Watch};

/// The pin pairs the chip joins at DC, for `net.floating`, by pin index: each GPIO to `VCC`
/// and to `GND` through its leakage. A part's GPIOs with nets take the first `2 * n`.
pub(crate) const DC_PATHS: [(usize, usize); 80] = {
    let mut paths = [(0, 0); 80];
    let mut k = 0;
    while k < 40 {
        paths[2 * k] = (2 + k, 0);
        paths[2 * k + 1] = (2 + k, 1);
        k += 1;
    }
    paths
};
const _: () = assert!(Esp32s3::PINS.len() == 2 + DC_PATHS.len() / 2);

pub(crate) fn dc_paths(mcu: &Esp32s3) -> &'static [(usize, usize)] {
    &DC_PATHS[..2 * (mcu.pins.len() - 2)]
}

#[derive(Clone, Debug)]
pub(crate) struct Core {
    chip: s3::Esp32s3,
    /// The GPIO of each output and watch.
    gpios: Vec<u8>,
    /// The net of each of `gpios`.
    nets: Vec<NetId>,
    /// The records it sent and still holds, in the order it sent them.
    records: Vec<Record>,
    /// How many records it has made.
    made: u64,
    image: Vec<Drive>,
    /// The levels it last took, in watch order. Empty before the power-on input.
    levels: Vec<bool>,
    /// The fault that ended its run, once a stop reported it.
    fault: Option<(Time, Fault)>,
}

pub(crate) fn chip(part: PartId, mcu: &Esp32s3) -> Chip {
    let (vcc, gnd) = (mcu.pins[0], mcu.pins[1]);
    let gpios: Vec<u8> = mcu.gpios().collect();
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let high = stage(&mcu.output.high_drop, &mcu.output.high_resistance);
    let low = stage(&mcu.output.low_drop, &mcu.output.low_resistance);
    let pins = &mcu.pins[2..];
    let outputs = pins
        .iter()
        .map(|&pin| Output {
            pin,
            vcc,
            gnd,
            high,
            low,
            pull_up: Some(mcu.pull_up_resistance.si.get()),
            leakage: Some(mcu.leakage_resistance.si.get()),
        })
        .collect();

    // V(pin) - f * V(VCC) - (1 - f) * V(GND): the pin against a fraction of its own supply.
    let against = |pin, fraction: f64| Signal {
        terms: vec![
            (Node::from(pin), 1.0),
            (Node::from(vcc), -fraction),
            (Node::from(gnd), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let threshold = &mcu.input_threshold;
    let watches = pins
        .iter()
        .map(|&pin| Watch {
            rise: against(pin, threshold.high.si.get()),
            fall: against(pin, threshold.low.si.get()),
        })
        .collect();
    let mut signals: Vec<(&'static str, usize)> = mcu.names[2..]
        .iter()
        .enumerate()
        .map(|(n, &name)| (name, n))
        .collect();
    signals.sort_unstable();

    let chip = match &mcu.firmware {
        Esp32s3Firmware::Elf(elf) => {
            let segments = elf
                .segments()
                .iter()
                .map(|segment| (segment.addr, elf.data(segment)));
            s3::Esp32s3::new(&gpios, elf.entry(), segments)
        }
        Esp32s3Firmware::Flash { rom, flash } => {
            let segments: Vec<s3::Segment> = flash
                .segments()
                .iter()
                .map(|segment| s3::Segment {
                    addr: segment.addr,
                    offset: segment.offset,
                    len: segment.len,
                })
                .collect();
            let boot = s3::Boot {
                rom: &rom.bytes,
                flash: flash.bytes().clone(),
                entry: flash.entry(),
                segments: &segments,
            };
            s3::Esp32s3::boot(&gpios, boot)
        }
    };
    let mut core = Core {
        chip,
        image: vec![Drive::Off; gpios.len()],
        gpios,
        nets: pins.to_vec(),
        records: Vec::new(),
        made: 0,
        levels: Vec::new(),
        fault: None,
    };
    core.refresh();
    Chip {
        part: Some(part),
        outputs,
        watches,
        signals,
        samples: Vec::new(),
        usarts: &["USB_SERIAL_JTAG"],
        twi: false,
        sends: true,
        takes: Takes::default(),
        core: super::Core::Esp32s3(Box::new(core)),
    }
}

/// The last tick at or before `at`.
fn tick(at: Time) -> Tick {
    let ticks = u128::from(at.as_ps()) * u128::from(TICK_HZ) / 1_000_000_000_000;
    Tick(u64::try_from(ticks).expect("a tick count fits in u64"))
}

/// The first picosecond at or after `tick`.
fn time(tick: Tick) -> Time {
    let at = (u128::from(tick.0) * 1_000_000_000_000).div_ceil(u128::from(TICK_HZ));
    Time::from_ps(u64::try_from(at).expect("a tick inside the run"))
}

impl Core {
    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let halt = self.chip.run_until(tick(limit))?;
        let at = time(halt.at());
        let mut record = false;
        match halt {
            Halt::Change {
                image,
                record: sent,
                ..
            } => {
                if image {
                    self.refresh();
                }
                if sent {
                    let transfers: Vec<Transfer> = self.chip.take_transfers().collect();
                    for transfer in transfers {
                        let record = self.record(transfer);
                        self.records.push(record);
                    }
                    record = true;
                }
            }
            Halt::Fault(_, fault) => self.fault = Some((at, fault)),
        }
        Some(Stop {
            at,
            sample: false,
            record,
        })
    }

    /// `transfer` as its `Spi` record, named by the pin of its CS pad, or `SPI2` when no pin of
    /// the part carries CS. A signal whose pad has no net has no net in the record.
    fn record(&mut self, transfer: Transfer) -> Record {
        let net = |pad: Option<u8>| {
            let k = self.gpios.iter().position(|&gpio| Some(gpio) == pad)?;
            Some(self.nets[k])
        };
        let lanes = |count: u8| match count {
            1 => Lanes::One,
            2 => Lanes::Two,
            4 => Lanes::Four,
            _ => unreachable!("SPI2 sends on 1, 2 or 4 lanes"),
        };
        let pads = transfer.pads;
        let pin = pads
            .cs
            .and_then(|pad| {
                Esp32s3::PINS
                    .into_iter()
                    .find(|name| name.strip_prefix("GPIO") == Some(&pad.to_string()))
            })
            .unwrap_or("SPI2");
        let seq = self.made;
        self.made += 1;
        Record {
            seq,
            start: time(transfer.start),
            end: time(transfer.end),
            pin,
            kind: RecordKind::Spi(Spi {
                phases: transfer
                    .phases
                    .iter()
                    .map(|phase| SpiPhase {
                        bits: phase.bits,
                        lanes: lanes(phase.lanes),
                        value: phase.value,
                    })
                    .collect(),
                lanes: lanes(transfer.lanes),
                data: transfer.data.into(),
                clock_hz: transfer.clock_hz,
                cs: if transfer.held {
                    Cs::Held
                } else {
                    Cs::Released
                },
                nets: SpiNets {
                    cs: net(pads.cs),
                    sclk: net(pads.sclk),
                    data: pads.data.map(net),
                },
            }),
        }
    }

    pub(crate) fn records(&self) -> &[Record] {
        &self.records
    }

    pub(crate) fn drain_records(&mut self, through: Time, sent: impl FnMut(Record)) {
        let ended = self.records.partition_point(|record| record.end <= through);
        self.records.drain(..ended).for_each(sent);
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        &self.image
    }

    /// Its GPIOs' levels, in watch order.
    pub(crate) fn input(&mut self, _at: Time, levels: &[bool]) {
        for (k, &level) in levels.iter().enumerate() {
            if self.levels.get(k) != Some(&level) {
                self.chip.set_pad(self.gpios[k], level);
            }
        }
        self.levels = levels.to_vec();
    }

    /// Takes the console bytes whose packet went on a tick at or before `through`'s last.
    pub(crate) fn deliver(&mut self, through: Time, mut sent: impl FnMut(Frame)) {
        while let Some((at, byte)) = self.chip.take_sent(tick(through)) {
            let at = time(at);
            sent(Frame {
                usart: 0,
                byte,
                start: at,
                end: at,
            });
        }
    }

    /// The fault that ended its run, as the diagnostic `part` raises, symbolized on `mcu`'s
    /// firmware.
    pub(crate) fn fault(&self, part: &Ref, mcu: &Esp32s3) -> Option<(Time, Diagnostic)> {
        let (at, fault) = self.fault?;
        let part = part.clone();
        let symbol = |pc| mcu.firmware.symbolize(pc);
        let diagnostic = match fault {
            Fault::Unsupported(unimplemented) => {
                let (op, reason) = match unimplemented.missing {
                    Missing::Op(op) => (op.name(), Unsupported::Op),
                    Missing::Unaligned(op) => (op.name(), Unsupported::Unaligned),
                    Missing::Litbase => ("l32r", Unsupported::Litbase),
                    Missing::LoopEnd(op) => (op.name(), Unsupported::LoopEnd),
                };
                Diagnostic::ChipUnsupportedInstruction {
                    part,
                    at,
                    op,
                    reason,
                    pc: unimplemented.pc,
                    symbol: symbol(unimplemented.pc),
                }
            }
            Fault::UnmappedFetch(pc) => Diagnostic::ChipUnmappedFetch {
                part,
                at,
                pc,
                symbol: symbol(pc),
            },
            Fault::Setting(setting, pc) => Diagnostic::ChipUnsupportedSetting {
                part,
                at,
                peripheral: "SPI2",
                setting: ChipSetting::Transfer(setting.name()),
                pc,
                symbol: symbol(pc),
            },
            Fault::Refused(refused, pc) => {
                let (peripheral, setting) = match refused {
                    Refused::Interrupt(peripheral) => (peripheral, ChipSetting::Interrupt),
                    Refused::Sleep => ("RTC_CNTL", ChipSetting::Sleep),
                };
                Diagnostic::ChipUnsupportedSetting {
                    part,
                    at,
                    peripheral,
                    setting,
                    pc,
                    symbol: symbol(pc),
                }
            }
            Fault::Watchdog(watchdog) => Diagnostic::ChipWatchdog {
                part,
                at,
                watchdog: watchdog.name(),
            },
            Fault::EfuseProgram(pc) => Diagnostic::ChipEfuseProgram {
                part,
                at,
                pc,
                symbol: symbol(pc),
            },
            Fault::FlashCommand(refusal, pc) => {
                use s3::FlashRefusal as R;
                let (command, reason) = match refusal {
                    R::Command(command) => (Some(command), FlashRefusal::Unsupported),
                    R::StatusWrite(command) => (Some(command), FlashRefusal::StatusWrite),
                    R::NoCommandPhase => (None, FlashRefusal::NoCommandPhase),
                    R::StartBits => (None, FlashRefusal::StartBits),
                    R::PastEnd(command) => (Some(command), FlashRefusal::PastEnd),
                    R::AddressBits(command) => (Some(command), FlashRefusal::AddressBits),
                    R::Spi0 => (None, FlashRefusal::Spi0),
                    R::Pads => (None, FlashRefusal::Pads),
                };
                Diagnostic::ChipFlashCommand {
                    part,
                    at,
                    command,
                    reason,
                    pc,
                    symbol: symbol(pc),
                }
            }
            Fault::FaultRestart(cause, pc) => Diagnostic::ChipFaultRestart {
                part,
                at,
                reason: match cause {
                    ResetCause::Panic => ResetReason::Panic,
                    ResetCause::InterruptWatchdog => ResetReason::InterruptWatchdog,
                    ResetCause::TaskWatchdog => ResetReason::TaskWatchdog,
                    ResetCause::Watchdog => ResetReason::Watchdog,
                    ResetCause::Brownout => ResetReason::Brownout,
                    ResetCause::Unreadable => ResetReason::Unreadable,
                },
                pc,
                symbol: symbol(pc),
            },
            Fault::UnmappedLoad { address, pc } => Diagnostic::ChipUnmappedLoad {
                part,
                at,
                address,
                pc,
                symbol: symbol(pc),
            },
            Fault::Clock(pc) => Diagnostic::ChipUnsupportedClock {
                part,
                at,
                pc,
                symbol: symbol(pc),
            },
        };
        Some((at, diagnostic))
    }

    /// A `chip.unmodeled_register` warning for each unmodeled register first touched at or
    /// before `through`.
    pub(crate) fn unmodeled(&self, part: &Ref, mcu: &Esp32s3, through: Time) -> Vec<Diagnostic> {
        self.chip
            .unmodeled()
            .filter(|(_, touch)| time(touch.at) <= through)
            .map(|(address, touch)| Diagnostic::ChipUnmodeledRegister {
                part: part.clone(),
                at: time(touch.at),
                address,
                register: Touch::register(address),
                pc: touch.pc,
                symbol: mcu.firmware.symbolize(touch.pc),
                write: touch.write,
                kept: touch.kept,
            })
            .collect()
    }

    /// The `chip.restart` warning, when the firmware reset the chip at or before `through`.
    pub(crate) fn restart(&self, part: &Ref, mcu: &Esp32s3, through: Time) -> Option<Diagnostic> {
        let (at, pc) = self.chip.restart()?;
        (time(at) <= through).then(|| Diagnostic::ChipRestart {
            part: part.clone(),
            at: time(at),
            pc,
            symbol: mcu.firmware.symbolize(pc),
        })
    }

    fn refresh(&mut self) {
        for (drive, &gpio) in self.image.iter_mut().zip(&self.gpios) {
            *drive = match self.chip.drive(gpio) {
                PadDrive::High => Drive::High,
                PadDrive::Low => Drive::Low,
                PadDrive::PullUp => Drive::PullUp,
                PadDrive::Off => Drive::Off,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ram_windows_are_the_netlists() {
        let windows: Vec<(u32, u32, u32, bool)> = Esp32s3::RAM
            .iter()
            .map(|w| (w.start, w.end, w.offset, w.code))
            .collect();
        let soc: Vec<(u32, u32, u32, bool)> = s3::RAM
            .iter()
            .map(|w| (w.start, w.end, w.offset, w.code))
            .collect();
        assert_eq!(windows, soc);
    }

    #[test]
    fn the_code_window_is_the_cores_instruction_ram() {
        use sim_xtensa::Config;

        let [code, _] = Esp32s3::RAM;
        assert!(code.code);
        assert_eq!(
            [code.start - 1, code.start, code.end - 1, code.end]
                .map(sim_xtensa::Esp32s3::instruction_bus),
            [false, true, true, false]
        );
    }

    #[test]
    fn a_crystal_cycle_is_25_ns() {
        assert_eq!(time(Tick(3 * 12)), Time::from_ps(75_000));
        assert_eq!(tick(Time::from_ps(74_999)).0 / 12, 2);
        assert_eq!(tick(Time::from_ps(75_000)), Tick(3 * 12));
    }
}
