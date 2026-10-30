//! The ESP32-S3 behind the seam. Each GPIO with a net becomes an output and a watch around
//! `sim_esp32::s3::Esp32s3`, its console goes to `serial.csv` as `USB_SERIAL_JTAG`, each SPI2
//! transfer is a `Spi` record, each LEDC update is a `Duty` record on each of its pins with a
//! net, and its 480 MHz tick converts between picoseconds and its timeline.
//!
//! An LEDC update that moves its pad's drive moves it 1 ps after its record's end, a stop of its
//! own, so the record and the drive change never land together.
//!
//! A console packet has no line timing: every byte of one starts and ends at the cycle the
//! firmware flushed it.

use sim_core::diagnostic::{ChipSetting, Diagnostic, FlashRefusal, ResetReason, Unsupported};
use sim_core::link::{self, Cs, Lanes, Record, RecordKind, Spi, SpiNets, SpiPhase};
use sim_core::name::Ref;
use sim_core::netlist::{Esp32s3, Esp32s3Firmware, NetId, PartId, Value};
use sim_core::time::Time;
use sim_esp32::s3::{
    self, CoreId, Duty, Fault, Halt, PadDrive, Pc, Refused, ResetCause, TICK_HZ, Tick, Touch,
    Transfer,
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
    /// The time of an LEDC drive move the chip reported past the last limit, which the next
    /// call stops at.
    moved: Option<Time>,
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

    let mut chip = match &mcu.firmware {
        Esp32s3Firmware::Elf(elf) => {
            let segments = elf
                .segments()
                .iter()
                .map(|segment| (segment.addr, elf.data(segment)));
            s3::Esp32s3::new(&gpios, elf.entry(), segments, None)
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
    for send in &mcu.console {
        chip.receive(tick_at_or_after(send.at), &send.to);
    }
    let mut core = Core {
        chip,
        image: vec![Drive::Off; gpios.len()],
        gpios,
        nets: pins.to_vec(),
        records: Vec::new(),
        made: 0,
        levels: Vec::new(),
        fault: None,
        moved: None,
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

/// Whether the APP CPU ran the code at `pc`.
fn on_app(pc: Pc) -> bool {
    pc.core == CoreId::App
}

/// The last tick at or before `at`.
fn tick(at: Time) -> Tick {
    let ticks = u128::from(at.as_ps()) * u128::from(TICK_HZ) / 1_000_000_000_000;
    Tick(u64::try_from(ticks).expect("a tick count fits in u64"))
}

/// The first tick at or after `at`.
fn tick_at_or_after(at: Time) -> Tick {
    let ticks = (u128::from(at.as_ps()) * u128::from(TICK_HZ)).div_ceil(1_000_000_000_000);
    Tick(u64::try_from(ticks).expect("a tick count fits in u64"))
}

/// The first picosecond at or after `tick`.
fn time(tick: Tick) -> Time {
    let at = (u128::from(tick.0) * 1_000_000_000_000).div_ceil(u128::from(TICK_HZ));
    Time::from_ps(u64::try_from(at).expect("a tick inside the run"))
}

impl Core {
    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        if let Some(at) = self.moved {
            return (at <= limit).then(|| self.move_drive(at));
        }
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
                    let duties: Vec<Duty> = self.chip.take_duties().collect();
                    for duty in duties {
                        let record = self.duty(duty);
                        self.records.push(record);
                    }
                    record = true;
                }
            }
            Halt::Moved(_) => {
                let at = Time::from_ps(at.as_ps() + 1);
                self.moved = Some(at);
                return (at <= limit).then(|| self.move_drive(at));
            }
            Halt::Fault(_, fault) => self.fault = Some((at, fault)),
        }
        Some(Stop {
            at,
            sample: false,
            record,
        })
    }

    /// The stop at `at` where the drives the chip moved after an LEDC record land.
    fn move_drive(&mut self, at: Time) -> Stop {
        self.moved = None;
        self.refresh();
        Stop {
            at,
            sample: false,
            record: false,
        }
    }

    /// The pin of the part on `pad`.
    fn pin(pad: u8) -> Option<&'static str> {
        Esp32s3::PINS
            .into_iter()
            .find(|name| name.strip_prefix("GPIO") == Some(&pad.to_string()))
    }

    /// `duty` as its `Duty` record, named by its pad's pin.
    fn duty(&mut self, duty: Duty) -> Record {
        let k = self
            .gpios
            .iter()
            .position(|&gpio| gpio == duty.pad)
            .expect("a duty's pad has a net");
        let seq = self.made;
        self.made += 1;
        Record {
            seq,
            start: time(duty.at),
            end: time(duty.at),
            pin: Self::pin(duty.pad).expect("a pad with a net has a pin"),
            kind: RecordKind::Duty(link::Duty {
                duty: duty.duty,
                resolution: duty.resolution,
                hz: duty.hz,
                net: self.nets[k],
            }),
        }
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
        let pin = pads.cs.and_then(Self::pin).unwrap_or("SPI2");
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
    pub(crate) fn input(&mut self, at: Time, levels: &[bool]) {
        let changed = levels
            .iter()
            .enumerate()
            .filter(|&(k, level)| self.levels.get(k) != Some(level))
            .map(|(k, &level)| (self.gpios[k], level));
        self.chip.input(tick(at), changed);
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
        let symbol = |pc: Pc| mcu.firmware.symbolize(pc.pc);
        let diagnostic = match fault {
            Fault::Unsupported(core, unimplemented) => {
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
                    app: core == CoreId::App,
                    symbol: mcu.firmware.symbolize(unimplemented.pc),
                }
            }
            Fault::UnmappedFetch(pc) => Diagnostic::ChipUnmappedFetch {
                part,
                at,
                pc: pc.pc,
                app: on_app(pc),
                symbol: symbol(pc),
            },
            Fault::Halted(pc) => Diagnostic::ChipHalted {
                part,
                at,
                pc: pc.pc,
                app: on_app(pc),
                symbol: symbol(pc),
            },
            Fault::Setting(setting, pc) => Diagnostic::ChipUnsupportedSetting {
                part,
                at,
                peripheral: "SPI2",
                setting: ChipSetting::Transfer(setting.name()),
                pc: pc.pc,
                app: on_app(pc),
                symbol: symbol(pc),
            },
            Fault::Refused(refused, pc) => {
                let (peripheral, setting) = match refused {
                    Refused::Interrupt(peripheral) => (peripheral, ChipSetting::Interrupt),
                    Refused::Register(peripheral, setting) => {
                        (peripheral, ChipSetting::Register(setting))
                    }
                    Refused::Sleep => ("RTC_CNTL", ChipSetting::Sleep),
                };
                Diagnostic::ChipUnsupportedSetting {
                    part,
                    at,
                    peripheral,
                    setting,
                    pc: pc.pc,
                    app: on_app(pc),
                    symbol: symbol(pc),
                }
            }
            Fault::Watchdog {
                watchdog,
                stage,
                reset,
            } => Diagnostic::ChipWatchdog {
                part,
                at,
                watchdog: watchdog.name(),
                stage,
                reset: reset.name(),
            },
            Fault::EfuseProgram(pc) => Diagnostic::ChipEfuseProgram {
                part,
                at,
                pc: pc.pc,
                app: on_app(pc),
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
                    pc: pc.pc,
                    app: on_app(pc),
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
                pc: pc.pc,
                app: on_app(pc),
                symbol: symbol(pc),
            },
            Fault::UnmappedLoad { address, pc } => Diagnostic::ChipUnmappedLoad {
                part,
                at,
                address,
                pc: pc.pc,
                app: on_app(pc),
                symbol: symbol(pc),
            },
            Fault::Clock(pc) => Diagnostic::ChipUnsupportedClock {
                part,
                at,
                pc: pc.pc,
                app: on_app(pc),
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
                pc: touch.pc.pc,
                app: on_app(touch.pc),
                symbol: mcu.firmware.symbolize(touch.pc.pc),
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
            pc: pc.pc,
            app: on_app(pc),
            symbol: mcu.firmware.symbolize(pc.pc),
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

    /// A console byte reaches the chip at the first tick at or after its time, never before.
    #[test]
    fn a_send_lands_on_the_first_tick_at_or_after_its_time() {
        assert_eq!(tick_at_or_after(Time::from_ps(75_000)), Tick(36));
        assert_eq!(tick_at_or_after(Time::from_ps(75_001)), Tick(37));
        assert_eq!(tick_at_or_after(Time::from_ps(1)), Tick(1));
        for ps in [1u64, 2083, 2084, 1_500_000_001] {
            let Tick(at) = tick_at_or_after(Time::from_ps(ps));
            let scaled = u128::from(ps) * u128::from(TICK_HZ);
            assert!(u128::from(at) * 1_000_000_000_000 >= scaled, "{ps}");
            assert!(u128::from(at - 1) * 1_000_000_000_000 < scaled, "{ps}");
        }
    }

    #[test]
    fn a_crystal_cycle_is_25_ns() {
        assert_eq!(time(Tick(3 * 12)), Time::from_ps(75_000));
        assert_eq!(tick(Time::from_ps(74_999)).0 / 12, 2);
        assert_eq!(tick(Time::from_ps(75_000)), Tick(3 * 12));
    }
}
