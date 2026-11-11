//! The ESP32-S3 behind the seam. Each GPIO with a net becomes an output and a watch around
//! `orbe_espressif::s3::Esp32s3`, its console goes to `serial.csv` as `USB_SERIAL_JTAG`, each SPI2
//! transfer is a `Spi` record, each LEDC update is a `Duty` record on each of its pins with a
//! net, each RMT transmission is a `Pulses` record on each of its pins with a net, and its
//! 480 MHz tick converts between picoseconds and its timeline.
//!
//! An LEDC update that moves its pad's drive moves it 1 ps after its record's end, a stop of its
//! own, so the record and the drive change never land together.
//!
//! A console packet has no line timing: every byte of one starts and ends at the cycle the
//! firmware flushed it.

use std::collections::BTreeMap;

use serde::Deserialize;

use orbe_model::{Cs, Drive, Lanes, Pulses, Spi, SpiPhase, Stop, Time};

use crate::active::{Chip, Frame, Output, Stage, Takes};
use crate::blob::{Blob, Blobs, Role, Sha256};
use crate::diagnostic::{
    BlobRule, ChipSetting, Diagnostic, FirmwareRule, FlashRefusal, ResetReason, ScheduleRule,
    Unsupported,
};
use crate::link::{Record, RecordKind, SpiNets};
use crate::mna::Node;
use crate::name::{Name, Ref};
use crate::netlist::Event;
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Threshold;
use crate::netlist::Value;
use crate::netlist::elf::{self, Elf};
use crate::netlist::flash::{self, Flash};
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, resolve_value};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};
use crate::watch::{Signal, Watch};
use orbe_espressif::s3::{
    self, CoreId, Duty, Fault, Halt, PadDrive, Pc, Refused, ResetCause, TICK_HZ, Tick, Touch,
    Train, Transfer,
};
use orbe_xtensa::Missing;

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
    /// A stop the chip reported whose time is past the last limit, which the first call
    /// through that time stops at.
    held: Option<(Time, Held)>,
    /// The latest time this copy has given the seam.
    served: Option<Time>,
}

/// What a held stop does.
#[derive(Clone, Copy, Debug)]
enum Held {
    /// The drives the chip moved land, 1 ps after its `Moved` halt.
    Moved,
    /// A fault the chip reported at or before a time this copy had already given the seam ends
    /// the run, 1 ps after that time.
    Fault(Fault),
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
        held: None,
        served: None,
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
        core: crate::active::Core::Esp32s3(Box::new(core)),
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
        if self.held.is_some() {
            return self.take_held(limit);
        }
        let Some(halt) = self.chip.run_until(tick(limit)) else {
            self.served = Some(limit);
            return None;
        };
        let at = time(halt.at());
        let mut record = false;
        let at = match halt {
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
                    let trains: Vec<Train> = self.chip.take_trains().collect();
                    for train in trains {
                        let record = self.pulses(train);
                        self.records.push(record);
                    }
                    record = true;
                }
                at
            }
            Halt::Moved(_) => {
                self.held = Some((Time::from_ps(at.as_ps() + 1), Held::Moved));
                return self.take_held(limit);
            }
            Halt::Fault(_, fault) => {
                // A Moved landing of this tick is already 1 ps later, and the input there can
                // refuse a pulse. The refusal stays on the tick, so its stop is the next
                // picosecond after whatever this copy has already given the seam.
                let at = match self.served {
                    Some(served) if at <= served => Time::from_ps(served.as_ps() + 1),
                    _ => at,
                };
                self.held = Some((at, Held::Fault(fault)));
                return self.take_held(limit);
            }
        };
        self.served = Some(at);
        Some(Stop {
            at,
            sample: false,
            record,
        })
    }

    /// The held stop, once `limit` reaches it. A fault ends the run there, and moved drives
    /// land there.
    fn take_held(&mut self, limit: Time) -> Option<Stop> {
        let (at, held) = self.held.take_if(|&mut (at, _)| at <= limit)?;
        match held {
            Held::Moved => self.refresh(),
            Held::Fault(fault) => self.fault = Some((at, fault)),
        }
        self.served = Some(at);
        Some(Stop {
            at,
            sample: false,
            record: false,
        })
    }

    /// The pin of the part on `pad`.
    fn pin(pad: u8) -> Option<&'static str> {
        Esp32s3::PINS
            .into_iter()
            .find(|name| name.strip_prefix("GPIO") == Some(&pad.to_string()))
    }

    /// `train` as its `Pulses` record, named by its pad's pin.
    fn pulses(&mut self, train: Train) -> Record {
        let k = self
            .gpios
            .iter()
            .position(|&gpio| gpio == train.pad)
            .expect("a train's pad has a net");
        let seq = self.made;
        self.made += 1;
        Record {
            seq,
            start: time(train.start),
            end: time(train.end),
            pin: Self::pin(train.pad).expect("a pad with a net has a pin"),
            kind: RecordKind::Pulses {
                pulses: Pulses {
                    idle: train.idle,
                    pulses: std::sync::Arc::from(train.pulses),
                },
                net: self.nets[k],
            },
        }
    }

    /// The copy stands at `at`, and no live or unreported transmission covers `at`.
    /// A held record ends at or before `served`, so standing on `at` already excludes it.
    /// A copy that ran past `at` is not the state at the cut.
    pub(crate) fn no_train_through(&self, at: Time) -> bool {
        debug_assert!(
            self.served != Some(at)
                || self.records.iter().all(|record| {
                    !matches!(record.kind, RecordKind::Pulses { .. }) || record.end <= at
                }),
            "a held record ends after the cut the copy stands on"
        );
        self.served == Some(at) && !self.chip.transmission_open(tick(at))
    }

    /// The part of a transmission still running at `at`, one record per netted pad. Each record
    /// ends one picosecond after `at`, so a chain cut at `at` takes the pairs read by then.
    pub(crate) fn in_flight(&self, at: Time) -> Vec<Record> {
        let Some(flight) = self.chip.in_flight(tick(at)) else {
            return Vec::new();
        };
        let end = Time::from_ps(at.as_ps().saturating_add(1));
        flight
            .pads
            .into_iter()
            .filter_map(|pad| {
                let k = self.gpios.iter().position(|&gpio| gpio == pad)?;
                Some(Record {
                    seq: 0,
                    start: time(flight.start),
                    end,
                    pin: Self::pin(pad)?,
                    kind: RecordKind::Pulses {
                        pulses: Pulses {
                            idle: flight.idle,
                            pulses: std::sync::Arc::from(flight.pulses.clone()),
                        },
                        net: self.nets[k],
                    },
                })
            })
            .collect()
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
            kind: RecordKind::Duty {
                duty: orbe_model::Duty {
                    duty: duty.duty,
                    resolution: duty.resolution,
                    hz: duty.hz,
                },
                net: self.nets[k],
            },
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
            kind: RecordKind::Spi {
                spi: Spi {
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
                },
                nets: SpiNets {
                    cs: net(pads.cs),
                    sclk: net(pads.sclk),
                    data: pads.data.map(net),
                },
            },
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
                    R::Cs(command) => (Some(command), FlashRefusal::Cs),
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
        use orbe_xtensa::Config;

        let [code, _] = Esp32s3::RAM;
        assert!(code.code);
        assert_eq!(
            [code.start - 1, code.start, code.end - 1, code.end]
                .map(orbe_xtensa::Esp32s3::instruction_bus),
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

    /// A chip on GPIO9 as SDA and GPIO10 as SCL that starts writing `0xA5` at 400 kHz with the
    /// filter at 7 cycles, then spins.
    fn i2c_core() -> Core {
        const BASE: u32 = 0x4037_0000;
        const I2C0: u32 = 0x6001_3000;
        const GPIO: u32 = 0x6000_4000;
        const IO_MUX: u32 = 0x6000_9000;
        let mut stores = vec![(0x600c_0018, 0xf9c1_e06f | 1 << 7)];
        for (pad, signal) in [(9u32, 90u32), (10, 89)] {
            stores.extend([
                (IO_MUX + 0x4 + 4 * pad, 1 << 12 | 1 << 9 | 1 << 8),
                (GPIO + 0x74 + 4 * pad, 1 << 2),
                (GPIO + 0x554 + 4 * pad, signal),
                (GPIO + 0x154 + 4 * signal, 1 << 7 | pad),
            ]);
        }
        let mode = 1 | 1 << 1 | 1 << 4;
        stores.extend(
            [
                (0x00, 49),
                (0x30, 11),
                (0x34, 24),
                (0x38, 23 << 9 | 27),
                (0x40, 49),
                (0x44, 49),
                (0x48, 49),
                (0x4c, 49),
                (0x0c, 10 | 1 << 5),
                (0x54, 1 << 21),
                (0x50, 7 | 7 << 4 | 1 << 8 | 1 << 9),
                (0x04, mode | 1 << 11),
                (0x1c, 0xa5),
                (0x58, 6 << 11),
                (0x5c, 1 << 11 | 1 << 8 | 1),
                (0x60, 2 << 11),
                (0x04, mode | 1 << 11 | 1 << 5),
            ]
            .map(|(offset, value)| (I2C0 + offset, value)),
        );
        let mut image: Vec<u8> = stores
            .iter()
            .flat_map(|&(addr, value)| [addr, value])
            .flat_map(u32::to_le_bytes)
            .collect();
        let entry = BASE + image.len() as u32;
        for k in 0..stores.len() as u32 {
            for (t, literal) in [(2u8, 2 * k), (3, 2 * k + 1)] {
                let pc = BASE + image.len() as u32;
                let imm = ((BASE + 4 * literal).wrapping_sub((pc + 3) & !3) as i32 >> 2) as u16;
                image.extend([t << 4 | 1, imm as u8, (imm >> 8) as u8]);
            }
            image.extend([0x32, 0x62, 0x00]);
        }
        image.extend([0x06, 0xff, 0xff]);
        let gpios = vec![9, 10];
        let mut core = Core {
            chip: s3::Esp32s3::new(&gpios, entry, [(BASE, image.as_slice())], None),
            image: vec![Drive::Off; gpios.len()],
            gpios,
            nets: vec![NetId::GROUND; 2],
            records: Vec::new(),
            made: 0,
            levels: Vec::new(),
            fault: None,
            held: None,
            served: None,
        };
        core.refresh();
        core
    }

    /// An input at a landing the copy has served refuses a pulse at that landing's tick. The
    /// refusal stops the next picosecond and never past a call's limit.
    #[test]
    fn a_refusal_at_a_served_landing_stops_the_next_picosecond() {
        let mut core = i2c_core();
        core.input(Time::ZERO, &[true, true]);
        let levels = |core: &Core| core.drive().iter().map(|d| *d != Drive::Low).collect();
        let released = loop {
            let stop = core
                .run_until(Time::from_ps(10_000_000))
                .expect("the bus moves");
            let now: Vec<bool> = levels(&core);
            core.input(stop.at, &now);
            if now == [true, false] {
                break stop.at;
            }
        };
        let landing = Time::from_ps(released.as_ps() + 25_000);
        assert_eq!(core.run_until(landing), None);
        core.input(landing, &[false, false]);
        assert_eq!(core.run_until(landing), None, "a stop past the limit");
        assert!(core.fault.is_none(), "a held refusal is not yet recorded");
        let next = Time::from_ps(landing.as_ps() + 1);
        assert_eq!(
            core.run_until(next).map(|stop| stop.at),
            Some(next),
            "the refusal"
        );
        assert!(
            matches!(
                core.fault,
                Some((at, Fault::Refused(Refused::Register("I2C0", "glitch"), _))) if at == next
            ),
            "{:?}",
            core.fault
        );
    }

    #[test]
    fn a_crystal_cycle_is_25_ns() {
        assert_eq!(time(Tick(3 * 12)), Time::from_ps(75_000));
        assert_eq!(tick(Time::from_ps(74_999)).0 / 12, 2);
        assert_eq!(tick(Time::from_ps(75_000)), Tick(3 * 12));
    }

    /// GPIO0 on ground and GPIO1 on ground. The train is four entries of 8 and 8 at divider 1,
    /// so it starts at 875,000 ps and ends at 2,475,000 ps.
    fn rmt_core(reroute: bool) -> Core {
        const BASE: u32 = 0x4037_0000;
        const RMT: u32 = 0x6001_6000;
        const GPIO: u32 = 0x6000_4000;
        const IO_MUX: u32 = 0x6000_9000;
        const SYSTEM: u32 = 0x600c_0000;
        let symbol = 8u32 | 1 << 15 | 8 << 16;
        let mut stores = vec![
            (SYSTEM + 0x18, 0xf9c1_e06f | 1 << 9),
            (SYSTEM + 0x20, 1 << 9),
            (SYSTEM + 0x20, 0),
            (IO_MUX + 4, 1 << 12),
            (GPIO + 0x554, 81),
            (RMT + 0xc0, 1 | 1 << 24 | 1 << 26),
        ];
        for index in 0..4u32 {
            stores.push((RMT + 0x800 + 4 * index, symbol));
        }
        stores.push((RMT + 0x810, 0));
        stores.push((RMT + 0x20, 1 | 1 << 4 | 1 << 6 | 1 << 8 | 1 << 16));
        if reroute {
            stores.extend([
                (GPIO + 0x554, 0x100),
                (IO_MUX + 8, 1 << 12),
                (GPIO + 0x558, 81),
            ]);
        }
        let mut image: Vec<u8> = stores
            .iter()
            .flat_map(|&(addr, value)| [addr, value])
            .flat_map(u32::to_le_bytes)
            .collect();
        let entry = BASE + image.len() as u32;
        for k in 0..stores.len() as u32 {
            for (t, literal) in [(2u8, 2 * k), (3, 2 * k + 1)] {
                let pc = BASE + image.len() as u32;
                let imm = ((BASE + 4 * literal).wrapping_sub((pc + 3) & !3) as i32 >> 2) as u16;
                image.extend([t << 4 | 1, imm as u8, (imm >> 8) as u8]);
            }
            image.extend([0x32, 0x62, 0x00]);
        }
        image.extend([0x06, 0xff, 0xff]);
        let gpios = vec![0, 1];
        let mut core = Core {
            chip: s3::Esp32s3::new(&gpios, entry, [(BASE, image.as_slice())], None),
            image: vec![Drive::Off; gpios.len()],
            gpios,
            nets: vec![NetId::GROUND, NetId::GROUND],
            records: Vec::new(),
            made: 0,
            levels: Vec::new(),
            fault: None,
            held: None,
            served: None,
        };
        core.refresh();
        core
    }

    fn pulses_of(record: &Record) -> (&Pulses, NetId) {
        match &record.kind {
            RecordKind::Pulses { pulses, net } => (pulses, *net),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_train_record_names_the_pad_it_started_on() {
        let mut core = rmt_core(false);
        let record = loop {
            let stop = core
                .run_until(Time::from_ps(5_000_000))
                .expect("the train ends");
            if stop.record {
                break core.records.pop().expect("the record");
            }
        };
        assert!(core.records.is_empty());
        assert_eq!(record.pin, "GPIO0");
        assert_eq!(record.seq, 0);
        assert_eq!(record.start, Time::from_ps(875_000));
        assert_eq!(record.end, Time::from_ps(2_475_000));
        let (pulses, net) = pulses_of(&record);
        assert!(!pulses.idle);
        assert_eq!(net, NetId::GROUND);
        assert_eq!(net, core.nets[0]);
        let pair = [(true, 200_000u64), (false, 200_000)];
        assert_eq!(pulses.pulses.len(), 8);
        for (index, pulse) in pulses.pulses.iter().enumerate() {
            assert_eq!(*pulse, pair[index % 2]);
        }
    }

    #[test]
    fn a_reroute_during_the_train_keeps_the_start_pad() {
        let mut core = rmt_core(true);
        let mut images = Vec::new();
        let record = loop {
            let stop = core
                .run_until(Time::from_ps(5_000_000))
                .expect("the train ends");
            if stop.record {
                break core.records.pop().expect("the record");
            }
            images.push(stop.at);
        };
        assert!(
            images
                .iter()
                .any(|at| *at > record.start && *at < record.end),
            "the pad moves while the train runs: {images:?}"
        );
        assert!(core.records.is_empty(), "gpio1 sends no second record");
        assert_eq!(record.pin, "GPIO0");
        assert_eq!(pulses_of(&record).1, core.nets[0]);
        assert_eq!(pulses_of(&record).0.pulses.len(), 8);
    }

    #[test]
    fn a_cut_inside_an_entry_returns_the_pairs_read_by_then() {
        let mut core = rmt_core(false);
        let opened = Time::from_ps(400_000);
        while let Some(stop) = core.run_until(opened) {
            assert!(!stop.record);
            assert_eq!(core.drive()[0], Drive::Low);
        }
        let mid = Time::from_ps(1_868_750);
        let flying = core.in_flight(mid);
        assert_eq!(flying.len(), 1);
        assert_eq!(flying[0].pin, "GPIO0");
        assert_eq!(flying[0].start, Time::from_ps(875_000));
        assert_eq!(flying[0].end, Time::from_ps(mid.as_ps() + 1));
        let (pulses, net) = pulses_of(&flying[0]);
        assert_eq!(net, core.nets[0]);
        assert_eq!(
            pulses.pulses.as_ref(),
            &[
                (true, 200_000),
                (false, 200_000),
                (true, 200_000),
                (false, 200_000),
                (true, 200_000),
            ]
        );
    }

    #[test]
    fn a_copy_short_of_the_cut_does_not_count_as_idle() {
        let mut core = rmt_core(false);
        let early = Time::from_ps(400_000);
        let mid = Time::from_ps(1_868_750);
        while core.run_until(early).is_some() {}
        assert!(core.no_train_through(early));
        assert!(!core.no_train_through(mid));
    }

    #[test]
    fn a_fault_cut_inside_a_train_the_copy_already_finished_keeps_the_pairs() {
        let mut ahead = rmt_core(false);
        let mid = Time::from_ps(1_868_750);
        let mut finished = false;
        while let Some(stop) = ahead.run_until(Time::from_ps(5_000_000)) {
            if stop.record {
                finished = true;
                break;
            }
        }
        assert!(finished, "the copy ran to the train's end");
        assert!(ahead.served.is_some_and(|served| served > mid));
        assert!(
            !ahead.no_train_through(mid),
            "the finished train still covers the cut"
        );
        let mut committed = rmt_core(false);
        while committed.run_until(mid).is_some() {}
        let flying = committed.in_flight(mid);
        assert_eq!(flying.len(), 1);
        assert_eq!(flying[0].end, Time::from_ps(mid.as_ps() + 1));
        assert_eq!(pulses_of(&flying[0]).0.pulses.len(), 5);
    }

    #[test]
    fn a_copy_past_a_quiet_cut_does_not_count_as_standing_there() {
        let mut core = rmt_core(false);
        let early = Time::from_ps(400_000);
        let later = Time::from_ps(500_000);
        while core.run_until(later).is_some() {}
        assert_eq!(core.served, Some(later));
        assert!(core.records.is_empty());
        assert!(!core.no_train_through(early));
    }

    #[test]
    fn a_copy_that_reached_a_cut_inside_a_train_still_has_one() {
        let mut core = rmt_core(false);
        let mid = Time::from_ps(1_868_750);
        while core.run_until(mid).is_some() {}
        assert!(!core.no_train_through(mid));
        let flying = core.in_flight(mid);
        assert_eq!(flying.len(), 1);
        assert_eq!(
            pulses_of(&flying[0]).0.pulses.as_ref(),
            &[
                (true, 200_000),
                (false, 200_000),
                (true, 200_000),
                (false, 200_000),
                (true, 200_000),
            ]
        );
    }
}

/// An ESP32-S3 running `firmware`: a bare-metal program whose core starts at its entry at
/// power-on, with every segment loaded, or the app on its flash, from where the bootloader
/// leaves it.
///
/// `VCC` and `GND` need nets, and every GPIO is optional. A GPIO with no net is neither stamped
/// nor watched. One with a net drives through `output` from `VCC` and into `GND`, pulls up
/// through `pull_up_resistance`, or is off, as the firmware sets it, leaks to `VCC` and to `GND`
/// through `leakage_resistance`, and reads its level through `input_threshold`. `console` is
/// what the host sends its console: each event's bytes, in order, after t = 0 and after the
/// previous event. Only parse builds one.
#[derive(Clone, Debug, PartialEq)]
pub struct Esp32s3 {
    /// `VCC`, `GND`, then each GPIO with a net, in `PINS` order.
    pub pins: Vec<NetId>,
    /// The name of each of `pins`.
    pub names: Vec<&'static str>,
    pub firmware: Esp32s3Firmware,
    pub output: OutputStage,
    /// Ohm. Every GPIO's pull-up.
    pub pull_up_resistance: Value,
    /// Ohm. Every GPIO's to `VCC` and again to `GND`, in every drive state.
    pub leakage_resistance: Value,
    pub input_threshold: Threshold,
    pub console: Vec<Event<Vec<u8>>>,
}

/// What an `esp32s3` runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Esp32s3Firmware {
    /// A bare-metal program in SRAM.
    Elf(elf::Elf),
    /// The app on the flash, booted with the mask ROM.
    Flash { rom: Blob, flash: flash::Flash },
}

impl Esp32s3Firmware {
    /// The blobs it names: the ELF, or the ROM and then each flash region.
    pub fn blobs(&self) -> Vec<&Blob> {
        match self {
            Esp32s3Firmware::Elf(elf) => vec![elf.blob()],
            Esp32s3Firmware::Flash { rom, flash } => {
                std::iter::once(rom).chain(flash.regions()).collect()
            }
        }
    }

    /// The symbol a diagnostic names `pc` by. A flash's app carries none.
    pub fn symbolize(&self, pc: u32) -> Option<String> {
        match self {
            Esp32s3Firmware::Elf(elf) => elf.symbolize(pc),
            Esp32s3Firmware::Flash { .. } => None,
        }
    }
}

impl Esp32s3 {
    /// The mask ROM the model runs, revision 0, as its SHA-256 and length.
    pub const ROM: (&'static str, u64) = (
        "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd",
        949_552,
    );

    /// The supply pins, then the GPIOs a board can wire. GPIO22 to GPIO25 do not exist, and
    /// GPIO33 to GPIO37 belong to the in-package PSRAM.
    pub const PINS: [&'static str; 42] = [
        "VCC", "GND", "GPIO0", "GPIO1", "GPIO2", "GPIO3", "GPIO4", "GPIO5", "GPIO6", "GPIO7",
        "GPIO8", "GPIO9", "GPIO10", "GPIO11", "GPIO12", "GPIO13", "GPIO14", "GPIO15", "GPIO16",
        "GPIO17", "GPIO18", "GPIO19", "GPIO20", "GPIO21", "GPIO26", "GPIO27", "GPIO28", "GPIO29",
        "GPIO30", "GPIO31", "GPIO32", "GPIO38", "GPIO39", "GPIO40", "GPIO41", "GPIO42", "GPIO43",
        "GPIO44", "GPIO45", "GPIO46", "GPIO47", "GPIO48",
    ];

    /// Where a bare-metal program may load: the 512 KB of internal SRAM, through its window on
    /// the instruction bus and through its window on the data bus. Each window's `offset` is
    /// its first byte's place in the SRAM, so code at `0x4037_8000` and data at `0x3fc8_8000`
    /// are one byte.
    pub const RAM: [elf::Window; 2] = [
        elf::Window {
            start: 0x4037_0000,
            end: 0x403e_0000,
            offset: 0,
            code: true,
        },
        elf::Window {
            start: 0x3fc8_8000,
            end: 0x3fd0_0000,
            offset: 0x8000,
            code: false,
        },
    ];

    /// The GPIO number of each of `pins` past `VCC` and `GND`.
    pub fn gpios(&self) -> impl Iterator<Item = u8> + '_ {
        self.names[2..].iter().map(|name| {
            name.strip_prefix("GPIO")
                .and_then(|number| number.parse().ok())
                .expect("a GPIO pin name")
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireEsp32s3 {
    pub(crate) name: String,
    pins: PinMap,
    #[serde(default)]
    rom: Option<WireBlob>,
    firmware: WireFirmware,
    output: WireOutputStage,
    pull_up_resistance: WireValue,
    leakage_resistance: WireValue,
    input_threshold: WireThreshold,
    #[serde(default)]
    console: Vec<WireSend>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSend {
    t_ps: u64,
    bytes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum WireFirmware {
    Elf(WireBlob),
    Flash(Vec<WireRegion>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRegion {
    offset: u32,
    #[serde(deserialize_with = "sha256")]
    sha256: Sha256,
    bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBlob {
    #[serde(deserialize_with = "sha256")]
    sha256: Sha256,
    bytes: u64,
}

fn sha256<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Sha256, D::Error> {
    let text = String::deserialize(deserializer)?;
    Sha256::parse(&text)
        .ok_or_else(|| serde::de::Error::custom(format!("`{text}` is not 64 lowercase hex digits")))
}

impl WireEsp32s3 {
    /// The SHA-256 of each blob it names: the ROM, then its firmware's.
    pub(crate) fn blob_names(&self) -> Vec<Sha256> {
        let firmware = match &self.firmware {
            WireFirmware::Elf(elf) => vec![elf.sha256],
            WireFirmware::Flash(regions) => regions.iter().map(|region| region.sha256).collect(),
        };
        self.rom
            .iter()
            .map(|rom| rom.sha256)
            .chain(firmware)
            .collect()
    }
}

pub(crate) fn resolve_esp32s3(
    name: &Name,
    part_ref: &Ref,
    wire: WireEsp32s3,
    nets: &BTreeMap<String, NetId>,
    blobs: &Blobs,
    out: &mut Vec<Diagnostic>,
) -> Option<Esp32s3> {
    let ohm = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part_ref, field, "ohm", true, value, out)
    };
    let pins = resolve_pins(name, &wire.pins.0, nets, out);
    let firmware = resolve_firmware(part_ref, wire.rom, wire.firmware, blobs, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let pull_up = ohm("pull_up_resistance", wire.pull_up_resistance, out);
    let leakage = ohm("leakage_resistance", wire.leakage_resistance, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let console = resolve_console(part_ref, wire.console, out);
    let (pins, names) = pins?;
    Some(Esp32s3 {
        pins,
        names,
        firmware: firmware?,
        output: output?,
        pull_up_resistance: pull_up?,
        leakage_resistance: leakage?,
        input_threshold: input_threshold?,
        console: console?,
    })
}

/// The console's sends, each after t = 0, after the previous one, and with a byte.
fn resolve_console(
    part: &Ref,
    wires: Vec<WireSend>,
    out: &mut Vec<Diagnostic>,
) -> Option<Vec<Event<Vec<u8>>>> {
    let mut ok = true;
    let mut previous = None;
    let mut events = Vec::with_capacity(wires.len());
    for (index, wire) in wires.into_iter().enumerate() {
        let rule = if previous.is_some_and(|previous| wire.t_ps <= previous) {
            Some(ScheduleRule::NotIncreasing)
        } else if wire.t_ps == 0 {
            Some(ScheduleRule::AtZero)
        } else if wire.bytes.is_empty() {
            Some(ScheduleRule::SameState)
        } else {
            None
        };
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                field: "console",
                event: index,
                rule,
                end_ps: 0,
            });
        }
        previous = Some(wire.t_ps);
        events.push(Event {
            at: Time::from_ps(wire.t_ps),
            to: wire.bytes,
        });
    }
    ok.then_some(events)
}

/// The firmware `wire` names, with `rom` beside a flash and absent beside an ELF.
fn resolve_firmware(
    part: &Ref,
    rom: Option<WireBlob>,
    wire: WireFirmware,
    blobs: &Blobs,
    out: &mut Vec<Diagnostic>,
) -> Option<Esp32s3Firmware> {
    let fail = |rule, out: &mut Vec<Diagnostic>| {
        out.push(Diagnostic::NetlistFirmware {
            part: part.clone(),
            line: None,
            rule,
        });
        None
    };
    match (rom, wire) {
        (None, WireFirmware::Elf(elf)) => {
            let blob = find(part, Role::Elf, &elf, blobs, out)?;
            match Elf::parse(blob, &Esp32s3::RAM) {
                Ok(elf) => Some(Esp32s3Firmware::Elf(elf)),
                Err(rule) => fail(rule, out),
            }
        }
        (Some(rom), WireFirmware::Flash(wire_regions)) => {
            let (sha256, bytes) = Esp32s3::ROM;
            if rom.sha256.to_string() != sha256 || rom.bytes != bytes {
                return fail(FirmwareRule::Rom, out);
            }
            let rom = find(part, Role::Rom, &rom, blobs, out);
            let mut regions = Vec::new();
            for region in wire_regions {
                let wire = WireBlob {
                    sha256: region.sha256,
                    bytes: region.bytes,
                };
                regions.push(find(part, Role::Flash(region.offset), &wire, blobs, out));
            }
            let regions: Option<Vec<_>> = regions.into_iter().collect();
            let (rom, regions) = (rom?, regions?);
            match Flash::parse(regions) {
                Ok(flash) => Some(Esp32s3Firmware::Flash { rom, flash }),
                Err(rule) => fail(rule, out),
            }
        }
        _ => fail(FirmwareRule::Rom, out),
    }
}

/// `VCC` and `GND`, then each GPIO the part has a net on, in [`Esp32s3::PINS`] order.
fn resolve_pins(
    part: &Name,
    pins: &BTreeMap<String, String>,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<(Vec<NetId>, Vec<&'static str>)> {
    let (mut ids, mut names) = (Vec::new(), Vec::new());
    let mut resolved = true;
    for &pin in &Esp32s3::PINS {
        let Some(net) = pins.get(pin) else {
            continue;
        };
        match nets.get(net) {
            Some(&id) => {
                ids.push(id);
                names.push(pin);
            }
            None => {
                resolved = false;
                out.push(Diagnostic::NetlistUnknownNet {
                    pin: Ref::Pin(part.clone(), Name::new(pin).expect("kind pin name")),
                    net: net.clone(),
                });
            }
        }
    }
    let missing: Vec<&'static str> = Esp32s3::PINS[..2]
        .iter()
        .copied()
        .filter(|pin| !pins.contains_key(*pin))
        .collect();
    let mut unexpected = Vec::new();
    for (key, net) in pins {
        if Esp32s3::PINS.contains(&key.as_str()) {
            continue;
        }
        unexpected.push(key.clone());
        if let Ok(pin) = Name::new(key)
            && !nets.contains_key(net)
        {
            out.push(Diagnostic::NetlistUnknownNet {
                pin: Ref::Pin(part.clone(), pin),
                net: net.clone(),
            });
        }
    }
    if !missing.is_empty() || !unexpected.is_empty() {
        out.push(Diagnostic::NetlistPins {
            part: Ref::Part(part.clone()),
            missing,
            unexpected,
        });
        resolved = false;
    }
    resolved.then_some((ids, names))
}

/// The blob `wire` names, when `blobs` has it under that name, it hashes to that name, and its
/// length is the netlist's.
fn find(
    part: &Ref,
    role: Role,
    wire: &WireBlob,
    blobs: &Blobs,
    out: &mut Vec<Diagnostic>,
) -> Option<Blob> {
    let mut fail = |rule, found| {
        out.push(Diagnostic::NetlistBlob {
            part: part.clone(),
            role,
            sha256: wire.sha256,
            rule,
            bytes: wire.bytes,
            found,
            source: blobs.source(),
        });
        None
    };
    let Some(bytes) = blobs.get(&wire.sha256) else {
        return fail(BlobRule::Missing, None);
    };
    let found = Some(bytes.len() as u64);
    if Sha256::of(bytes) != wire.sha256 {
        return fail(BlobRule::Hash, found);
    }
    if found != Some(wire.bytes) {
        return fail(BlobRule::Length, found);
    }
    Some(Blob {
        role,
        sha256: wire.sha256,
        bytes: bytes.clone(),
    })
}
