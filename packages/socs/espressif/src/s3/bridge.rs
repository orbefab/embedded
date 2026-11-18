//! The ESP32-S3 on the `orbe-mcu` contract. Each GPIO the host wires becomes an output and a
//! watch around [`Esp32s3`], its console is the `USB_SERIAL_JTAG` USART and each UART byte is
//! `UART0`, `UART1` or `UART2`, each SPI2 transfer is a
//! `Spi` record, each LEDC update is a `Duty` record on each of its wired pads, each RMT
//! transmission is a `Pulses` record on each of its wired pads, and its 480 MHz tick converts
//! between picoseconds and its timeline.
//!
//! An LEDC update that moves its pad's drive moves it 1 ps after its record's end, a stop of its
//! own, so the record and the drive change never land together.
//!
//! A console packet has no line timing: every byte of one starts and ends at the cycle the
//! firmware flushed it.

use orbe_mcu::{self as mcu, Frame, Mcu, Note, Phase, Sent, SentKind, SpiPins};
use orbe_model::{Cs, Drive, Lanes, Pulses, Spi, SpiPhase, Stop, Time};
use orbe_xtensa::{CoreId, Missing};

use super::{
    Duty, Esp32s3, Fault, FlashRefusal, Halt, PadDrive, Pc, Refused, ResetCause, TICK_HZ, Tick,
    Touch, Train, Transfer,
};

/// The supply pins, then the GPIOs a board can wire. GPIO22 to GPIO25 do not exist, and GPIO33
/// to GPIO37 belong to the in-package PSRAM.
pub const PINS: [&str; 42] = [
    "VCC", "GND", "GPIO0", "GPIO1", "GPIO2", "GPIO3", "GPIO4", "GPIO5", "GPIO6", "GPIO7", "GPIO8",
    "GPIO9", "GPIO10", "GPIO11", "GPIO12", "GPIO13", "GPIO14", "GPIO15", "GPIO16", "GPIO17",
    "GPIO18", "GPIO19", "GPIO20", "GPIO21", "GPIO26", "GPIO27", "GPIO28", "GPIO29", "GPIO30",
    "GPIO31", "GPIO32", "GPIO38", "GPIO39", "GPIO40", "GPIO41", "GPIO42", "GPIO43", "GPIO44",
    "GPIO45", "GPIO46", "GPIO47", "GPIO48",
];

/// An ESP32-S3 and the GPIOs the host wired.
#[derive(Clone, Debug)]
pub struct Bridge {
    chip: Esp32s3,
    /// The GPIO of each output and watch.
    gpios: Vec<u8>,
    image: Vec<Drive>,
    /// Whether each output pulls down, beside its drive in `image`.
    pulled_down: Vec<bool>,
    /// The levels it last took, in watch order. Empty before the power-on input.
    levels: Vec<bool>,
    /// The fault that ended its run, once a stop reported it.
    fault: Option<(Time, Fault)>,
    /// A stop the chip reported whose time is past the last limit, which the first call
    /// through that time stops at.
    held: Option<(Time, Held)>,
    /// The latest time this copy has given the host.
    served: Option<Time>,
}

/// What a held stop does.
#[derive(Clone, Copy, Debug)]
enum Held {
    /// The drives the chip moved land, 1 ps after its `Moved` halt.
    Moved,
    /// A fault the chip reported at or before a time this copy had already given the host ends
    /// the run, 1 ps after that time.
    Fault(Fault),
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

/// `pc` with its CPU as an index, the PRO CPU first.
fn pc(pc: Pc) -> mcu::Pc {
    mcu::Pc {
        pc: pc.pc,
        core: u8::from(pc.core == CoreId::App),
    }
}

/// The pin of the part on `pad`.
fn pin(pad: u8) -> Option<&'static str> {
    PINS.into_iter()
        .find(|name| name.strip_prefix("GPIO") == Some(&pad.to_string()))
}

impl Bridge {
    /// `chip` at power-on, wired on `gpios` in output order, with each of `console`'s bytes
    /// sent to its console at the first tick at or after its time.
    pub fn new<'a>(
        mut chip: Esp32s3,
        gpios: Vec<u8>,
        console: impl IntoIterator<Item = (Time, &'a [u8])>,
    ) -> Bridge {
        for (at, bytes) in console {
            chip.receive(tick_at_or_after(at), bytes);
        }
        let mut bridge = Bridge {
            chip,
            image: vec![Drive::Off; gpios.len()],
            pulled_down: vec![false; gpios.len()],
            gpios,
            levels: Vec::new(),
            fault: None,
            held: None,
            served: None,
        };
        bridge.refresh();
        bridge
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

    /// The output on `pad`, when the host wired it.
    fn output(&self, pad: u8) -> Option<usize> {
        self.gpios.iter().position(|&gpio| gpio == pad)
    }

    /// `train` as its `Pulses` record, named by its pad's pin.
    fn pulses(&self, train: Train) -> Sent {
        Sent {
            start: time(train.start),
            end: time(train.end),
            pin: pin(train.pad).expect("a pad with a net has a pin"),
            kind: SentKind::Pulses {
                pulses: Pulses {
                    idle: train.idle,
                    pulses: std::sync::Arc::from(train.pulses),
                },
                output: self.output(train.pad).expect("a train's pad has a net"),
            },
        }
    }

    /// The copy stands at `at`, and no live or unreported transmission covers `at`. A copy
    /// that ran past `at` is not the state at the cut.
    fn no_train_through(&self, at: Time) -> bool {
        self.served == Some(at) && !self.chip.transmission_open(tick(at))
    }

    /// The part of a transmission still running at `at`, one record per wired pad. Each record
    /// ends one picosecond after `at`, so a chain cut at `at` takes the pairs read by then.
    fn flying(&self, at: Time) -> Vec<Sent> {
        let Some(flight) = self.chip.in_flight(tick(at)) else {
            return Vec::new();
        };
        let end = Time::from_ps(at.as_ps().saturating_add(1));
        flight
            .pads
            .into_iter()
            .filter_map(|pad| {
                let output = self.output(pad)?;
                Some(Sent {
                    start: time(flight.start),
                    end,
                    pin: pin(pad)?,
                    kind: SentKind::Pulses {
                        pulses: Pulses {
                            idle: flight.idle,
                            pulses: std::sync::Arc::from(flight.pulses.clone()),
                        },
                        output,
                    },
                })
            })
            .collect()
    }

    /// `duty` as its `Duty` record, named by its pad's pin.
    fn duty(&self, duty: Duty) -> Sent {
        Sent {
            start: time(duty.at),
            end: time(duty.at),
            pin: pin(duty.pad).expect("a pad with a net has a pin"),
            kind: SentKind::Duty {
                duty: orbe_model::Duty {
                    duty: duty.duty,
                    resolution: duty.resolution,
                    hz: duty.hz,
                },
                output: self.output(duty.pad).expect("a duty's pad has a net"),
            },
        }
    }

    /// `transfer` as its `Spi` record, named by the pin of its CS pad, or `SPI2` when no pin of
    /// the part carries CS. A signal whose pad has no net is on no output.
    fn spi(&self, transfer: Transfer) -> Sent {
        let lanes = |count: u8| match count {
            1 => Lanes::One,
            2 => Lanes::Two,
            4 => Lanes::Four,
            _ => unreachable!("SPI2 sends on 1, 2 or 4 lanes"),
        };
        let output = |pad: Option<u8>| pad.and_then(|pad| self.output(pad));
        let pads = transfer.pads;
        Sent {
            start: time(transfer.start),
            end: time(transfer.end),
            pin: pads.cs.and_then(pin).unwrap_or("SPI2"),
            kind: SentKind::Spi {
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
                pins: SpiPins {
                    cs: output(pads.cs),
                    sclk: output(pads.sclk),
                    data: pads.data.map(output),
                },
            },
        }
    }

    fn refresh(&mut self) {
        let outputs = self.image.iter_mut().zip(&mut self.pulled_down);
        for ((drive, pulled_down), &gpio) in outputs.zip(&self.gpios) {
            (*drive, *pulled_down) = match self.chip.drive(gpio) {
                PadDrive::High => (Drive::High, false),
                PadDrive::Low => (Drive::Low, false),
                PadDrive::PullUp => (Drive::PullUp, false),
                PadDrive::PullDown => (Drive::Off, true),
                PadDrive::PullBoth => (Drive::PullUp, true),
                PadDrive::Off => (Drive::Off, false),
            };
        }
    }
}

impl Mcu for Bridge {
    const USARTS: &'static [&'static str] = &["USB_SERIAL_JTAG", "UART0", "UART1", "UART2"];
    const SENDS: bool = true;

    #[inline]
    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        if self.held.is_some() {
            return self.take_held(limit);
        }
        let Some(halt) = self.chip.run_until(tick(limit)) else {
            self.served = Some(limit);
            return None;
        };
        let at = time(halt.at());
        let (at, record) = match halt {
            Halt::Change { image, record, .. } => {
                if image {
                    self.refresh();
                }
                (at, record)
            }
            Halt::Moved(_) => {
                self.held = Some((Time::from_ps(at.as_ps() + 1), Held::Moved));
                return self.take_held(limit);
            }
            Halt::Fault(_, fault) => {
                // A Moved landing of this tick is already 1 ps later, and the input there can
                // refuse a pulse. The refusal stays on the tick, so its stop is the next
                // picosecond after whatever this copy has already given the host.
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

    #[inline]
    fn drive(&self) -> &[Drive] {
        &self.image
    }

    #[inline]
    fn pulled_down(&self) -> &[bool] {
        &self.pulled_down
    }

    /// Its GPIOs' levels, in watch order.
    fn input(&mut self, at: Time, levels: &[bool]) {
        let changed = levels
            .iter()
            .enumerate()
            .filter(|&(k, level)| self.levels.get(k) != Some(level))
            .map(|(k, &level)| (self.gpios[k], level));
        self.chip.input(tick(at), changed);
        self.levels = levels.to_vec();
    }

    /// Takes the console bytes whose packet went on a tick at or before `through`'s last.
    fn deliver(&mut self, through: Time, mut sent: impl FnMut(Frame), _saw: impl FnMut(Phase)) {
        while let Some((at, byte)) = self.chip.take_sent(tick(through)) {
            let at = time(at);
            sent(Frame {
                usart: 0,
                byte,
                start: at,
                end: at,
            });
        }
        let through = tick(through);
        while let Some(byte) = self.chip.take_uart(through) {
            sent(Frame {
                usart: byte.port + 1,
                byte: byte.byte,
                start: time(byte.start),
                end: time(byte.end),
            });
        }
    }

    /// The SPI2 transfers, then the LEDC updates, then the RMT transmissions.
    fn take_sent(&mut self, mut sent: impl FnMut(Sent)) {
        let transfers: Vec<Transfer> = self.chip.take_transfers().collect();
        for transfer in transfers {
            sent(self.spi(transfer));
        }
        let duties: Vec<Duty> = self.chip.take_duties().collect();
        for duty in duties {
            sent(self.duty(duty));
        }
        let trains: Vec<Train> = self.chip.take_trains().collect();
        for train in trains {
            sent(self.pulses(train));
        }
    }

    fn in_flight(&self, ahead: &Self, at: Time) -> Vec<Sent> {
        if ahead.no_train_through(at) {
            return Vec::new();
        }
        self.flying(at)
    }

    fn fault(&self) -> Option<(Time, mcu::Fault)> {
        let (at, fault) = self.fault?;
        let fault = match fault {
            Fault::Unsupported(core, unimplemented) => {
                let (op, reason) = match unimplemented.missing {
                    Missing::Op(op) => (op.name(), mcu::Unsupported::Op),
                    Missing::Unaligned(op) => (op.name(), mcu::Unsupported::Unaligned),
                    Missing::Litbase => ("l32r", mcu::Unsupported::Litbase),
                    Missing::LoopEnd(op) => (op.name(), mcu::Unsupported::LoopEnd),
                };
                mcu::Fault::UnsupportedInstruction {
                    op,
                    reason,
                    pc: pc(Pc {
                        core,
                        pc: unimplemented.pc,
                    }),
                }
            }
            Fault::UnmappedFetch(p) => mcu::Fault::UnmappedFetch(pc(p)),
            Fault::Halted(p) => mcu::Fault::Halted(pc(p)),
            Fault::Setting(setting, p) => mcu::Fault::UnsupportedSetting {
                peripheral: "SPI2",
                setting: mcu::Setting::Transfer(setting.name()),
                pc: pc(p),
            },
            Fault::Refused(refused, p) => {
                let (peripheral, setting) = match refused {
                    Refused::Interrupt(peripheral) => (peripheral, mcu::Setting::Interrupt),
                    Refused::Register(peripheral, setting) => {
                        (peripheral, mcu::Setting::Register(setting))
                    }
                    Refused::Sleep => ("RTC_CNTL", mcu::Setting::Sleep),
                };
                mcu::Fault::UnsupportedSetting {
                    peripheral,
                    setting,
                    pc: pc(p),
                }
            }
            Fault::Watchdog {
                watchdog,
                stage,
                reset,
            } => mcu::Fault::Watchdog {
                watchdog: watchdog.name(),
                stage,
                reset: reset.name(),
            },
            Fault::EfuseProgram(p) => mcu::Fault::EfuseProgram(pc(p)),
            Fault::FlashCommand(refusal, p) => {
                use mcu::FlashRefusal as To;
                let (command, reason) = match refusal {
                    FlashRefusal::Command(command) => (Some(command), To::Unsupported),
                    FlashRefusal::StatusWrite(command) => (Some(command), To::StatusWrite),
                    FlashRefusal::NoCommandPhase => (None, To::NoCommandPhase),
                    FlashRefusal::StartBits => (None, To::StartBits),
                    FlashRefusal::PastEnd(command) => (Some(command), To::PastEnd),
                    FlashRefusal::AddressBits(command) => (Some(command), To::AddressBits),
                    FlashRefusal::Spi0 => (None, To::Spi0),
                    FlashRefusal::Pads => (None, To::Pads),
                    FlashRefusal::Cs(command) => (Some(command), To::Cs),
                };
                mcu::Fault::FlashCommand {
                    command,
                    reason,
                    pc: pc(p),
                }
            }
            Fault::FaultRestart(cause, p) => mcu::Fault::FaultRestart {
                reason: match cause {
                    ResetCause::Panic => mcu::ResetReason::Panic,
                    ResetCause::InterruptWatchdog => mcu::ResetReason::InterruptWatchdog,
                    ResetCause::TaskWatchdog => mcu::ResetReason::TaskWatchdog,
                    ResetCause::Watchdog => mcu::ResetReason::Watchdog,
                    ResetCause::Brownout => mcu::ResetReason::Brownout,
                    ResetCause::Unreadable => mcu::ResetReason::Unreadable,
                },
                pc: pc(p),
            },
            Fault::UnmappedLoad { address, pc: p } => {
                mcu::Fault::UnmappedLoad { address, pc: pc(p) }
            }
            Fault::Clock(p) => mcu::Fault::UnsupportedClock(pc(p)),
        };
        Some((at, fault))
    }

    /// Each unmodeled register first touched at or before `through`, then the restart, when
    /// the firmware reset the chip by then.
    fn notes(&self, through: Time) -> Vec<(Time, Note)> {
        let unmodeled = self
            .chip
            .unmodeled()
            .filter(|(_, touch)| time(touch.at) <= through)
            .map(|(address, touch)| {
                let note = Note::UnmodeledRegister {
                    address,
                    register: Touch::register(address),
                    pc: pc(touch.pc),
                    write: touch.write,
                    kept: touch.kept,
                };
                (time(touch.at), note)
            });
        let restart = self
            .chip
            .restart()
            .filter(|&(at, _)| time(at) <= through)
            .map(|(at, at_pc)| (time(at), Note::Restart(pc(at_pc))));
        unmodeled.chain(restart).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn i2c_bridge() -> Bridge {
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
        bridge(&[9, 10], &stores)
    }

    /// A bridge on `gpios` that runs `stores`, each an `l32r` of its address and value and an
    /// `s32i`, then spins.
    fn bridge(gpios: &[u8], stores: &[(u32, u32)]) -> Bridge {
        const BASE: u32 = 0x4037_0000;
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
        let chip = Esp32s3::new(gpios, entry, [(BASE, image.as_slice())], None);
        Bridge::new(chip, gpios.to_vec(), [])
    }

    /// An input at a landing the copy has served refuses a pulse at that landing's tick. The
    /// refusal stops the next picosecond and never past a call's limit.
    #[test]
    fn a_refusal_at_a_served_landing_stops_the_next_picosecond() {
        let mut core = i2c_bridge();
        core.input(Time::ZERO, &[true, true]);
        let levels = |core: &Bridge| core.drive().iter().map(|d| *d != Drive::Low).collect();
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

    /// GPIO0 and GPIO1 wired. The train is four entries of 8 and 8 at divider 1, so it starts
    /// at 875,000 ps and ends at 2,475,000 ps.
    fn rmt_bridge(reroute: bool) -> Bridge {
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
        bridge(&[0, 1], &stores)
    }

    fn pulses_of(sent: &Sent) -> (&Pulses, usize) {
        match &sent.kind {
            SentKind::Pulses { pulses, output } => (pulses, *output),
            other => panic!("{other:?}"),
        }
    }

    /// Runs to the first stop that sends, and returns what it sent, and the times of the stops
    /// before it.
    fn first_sent(core: &mut Bridge) -> (Vec<Sent>, Vec<Time>) {
        let mut stops = Vec::new();
        loop {
            let stop = core
                .run_until(Time::from_ps(5_000_000))
                .expect("the train ends");
            if stop.record {
                let mut sent = Vec::new();
                core.take_sent(|record| sent.push(record));
                return (sent, stops);
            }
            stops.push(stop.at);
        }
    }

    #[test]
    fn a_train_record_names_the_pad_it_started_on() {
        let mut core = rmt_bridge(false);
        let (sent, _) = first_sent(&mut core);
        let [record] = &sent[..] else {
            panic!("one record: {sent:?}");
        };
        assert_eq!(record.pin, "GPIO0");
        assert_eq!(record.start, Time::from_ps(875_000));
        assert_eq!(record.end, Time::from_ps(2_475_000));
        let (pulses, output) = pulses_of(record);
        assert!(!pulses.idle);
        assert_eq!(output, 0);
        let pair = [(true, 200_000u64), (false, 200_000)];
        assert_eq!(pulses.pulses.len(), 8);
        for (index, pulse) in pulses.pulses.iter().enumerate() {
            assert_eq!(*pulse, pair[index % 2]);
        }
    }

    #[test]
    fn a_reroute_during_the_train_keeps_the_start_pad() {
        let mut core = rmt_bridge(true);
        let (sent, images) = first_sent(&mut core);
        let [record] = &sent[..] else {
            panic!("gpio1 sends no second record: {sent:?}");
        };
        assert!(
            images
                .iter()
                .any(|at| *at > record.start && *at < record.end),
            "the pad moves while the train runs: {images:?}"
        );
        assert_eq!(record.pin, "GPIO0");
        assert_eq!(pulses_of(record).1, 0);
        assert_eq!(pulses_of(record).0.pulses.len(), 8);
    }

    #[test]
    fn a_cut_inside_an_entry_returns_the_pairs_read_by_then() {
        let mut core = rmt_bridge(false);
        let opened = Time::from_ps(400_000);
        while let Some(stop) = core.run_until(opened) {
            assert!(!stop.record);
            assert_eq!(core.drive()[0], Drive::Low);
        }
        let mid = Time::from_ps(1_868_750);
        let flying = core.flying(mid);
        assert_eq!(flying.len(), 1);
        assert_eq!(flying[0].pin, "GPIO0");
        assert_eq!(flying[0].start, Time::from_ps(875_000));
        assert_eq!(flying[0].end, Time::from_ps(mid.as_ps() + 1));
        let (pulses, output) = pulses_of(&flying[0]);
        assert_eq!(output, 0);
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
        let mut core = rmt_bridge(false);
        let early = Time::from_ps(400_000);
        let mid = Time::from_ps(1_868_750);
        while core.run_until(early).is_some() {}
        assert!(core.no_train_through(early));
        assert!(!core.no_train_through(mid));
    }

    #[test]
    fn a_fault_cut_inside_a_train_the_copy_already_finished_keeps_the_pairs() {
        let mut ahead = rmt_bridge(false);
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
        let mut committed = rmt_bridge(false);
        while committed.run_until(mid).is_some() {}
        let flying = committed.in_flight(&ahead, mid);
        assert_eq!(flying.len(), 1);
        assert_eq!(flying[0].end, Time::from_ps(mid.as_ps() + 1));
        assert_eq!(pulses_of(&flying[0]).0.pulses.len(), 5);
    }

    #[test]
    fn a_copy_past_a_quiet_cut_does_not_count_as_standing_there() {
        let mut core = rmt_bridge(false);
        let early = Time::from_ps(400_000);
        let later = Time::from_ps(500_000);
        while let Some(stop) = core.run_until(later) {
            assert!(!stop.record);
        }
        assert_eq!(core.served, Some(later));
        assert!(!core.no_train_through(early));
    }

    #[test]
    fn a_copy_that_reached_a_cut_inside_a_train_still_has_one() {
        let mut core = rmt_bridge(false);
        let mid = Time::from_ps(1_868_750);
        while core.run_until(mid).is_some() {}
        assert!(!core.no_train_through(mid));
        let flying = core.in_flight(&core, mid);
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
