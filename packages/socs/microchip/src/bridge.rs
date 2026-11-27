//! The ATmega328P on the `orbe-mcu` contract. Its port pins become the image and the watches
//! around [`Timeline`], in package order, and its oscillator converts between picoseconds and
//! cycles. What the firmware reached that the model does not run becomes the contract's fault
//! and notes.

use orbe_mcu::{Fault, Frame, I2cPhase, Mcu, Note, Pc, Phase, Setting, Unsupported};
use orbe_model::{Drive, Quantity, Readings, Stop, Time};

use crate::{Bootloader, Channel, Cpu, FuseBytes, I2cNote, PinDrive, Port, Refused, Timeline};

/// What a pin of the kind is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pad {
    /// `P<port><bit>`, which the chip drives and reads.
    Port(Port, u8),
    /// `ADC6` or `ADC7`, an input of the ADC alone, with no port behind it (§1.1.9).
    Adc(u8),
    /// `PB6` or `PB7`, which the crystal holds.
    Xtal,
    Vcc,
    /// Pin 8, the digital ground.
    Gnd,
    Avcc,
    Aref,
    /// Pin 22, the analog ground.
    Agnd,
}

/// What each pin of the kind is: the 28-pin DIP's, pin 1 first, then `ADC6` and `ADC7`, which the
/// TQFP has and the DIP does not. Pin 1 is `PC6`, which is `RESET` while the fuses keep it so.
pub const PACKAGE: [Pad; 30] = [
    Pad::Port(Port::C, 6),
    Pad::Port(Port::D, 0),
    Pad::Port(Port::D, 1),
    Pad::Port(Port::D, 2),
    Pad::Port(Port::D, 3),
    Pad::Port(Port::D, 4),
    Pad::Vcc,
    Pad::Gnd,
    Pad::Xtal,
    Pad::Xtal,
    Pad::Port(Port::D, 5),
    Pad::Port(Port::D, 6),
    Pad::Port(Port::D, 7),
    Pad::Port(Port::B, 0),
    Pad::Port(Port::B, 1),
    Pad::Port(Port::B, 2),
    Pad::Port(Port::B, 3),
    Pad::Port(Port::B, 4),
    Pad::Port(Port::B, 5),
    Pad::Avcc,
    Pad::Aref,
    Pad::Agnd,
    Pad::Port(Port::C, 0),
    Pad::Port(Port::C, 1),
    Pad::Port(Port::C, 2),
    Pad::Port(Port::C, 3),
    Pad::Port(Port::C, 4),
    Pad::Port(Port::C, 5),
    Pad::Adc(6),
    Pad::Adc(7),
];

/// The package index of each port pin, in package order. The image and the watches follow
/// this order.
pub const PORT_PINS: [usize; 21] = {
    let mut pins = [0; 21];
    let (mut k, mut n) = (0, 0);
    while k < PACKAGE.len() {
        if let Pad::Port(..) = PACKAGE[k] {
            pins[n] = k;
            n += 1;
        }
        k += 1;
    }
    assert!(n == pins.len());
    pins
};

/// The die's id for each of [`PORT_PINS`].
const DIE: [u32; PORT_PINS.len()] = {
    let mut die = [0; PORT_PINS.len()];
    let mut n = 0;
    while n < die.len() {
        if let Pad::Port(port, bit) = PACKAGE[PORT_PINS[n]] {
            die[n] = port.pin_id(bit);
        }
        n += 1;
    }
    die
};

/// The output and watch index of port pin `pad`.
pub fn port_index(pad: Pad) -> usize {
    PORT_PINS
        .iter()
        .position(|&k| PACKAGE[k] == pad)
        .expect("a port pin")
}

/// An ATmega328P on its crystal.
#[derive(Debug)]
pub struct Bridge {
    timeline: Timeline,
    clock_hz: u64,
    image: [Drive; PORT_PINS.len()],
    /// The levels it last took, in watch order. Empty before the power-on input.
    levels: Vec<bool>,
    /// How many TWI phases `deliver` has taken, which numbers the next one.
    phases: u64,
    /// When the chip stopped on its refusal, once a stop has given it.
    refused: Option<Time>,
}

clone_fields!(Bridge {
    timeline,
    clock_hz,
    image,
    levels,
    phases,
    refused
});

impl Bridge {
    /// The chip on a `clock_hz` crystal, with `fuses` programmed, `bootloader` in its boot section
    /// and `flash` from address 0, at its first instruction at 0 ps. [`Bridge::power_on`] starts
    /// it at power-on instead.
    pub fn new(clock_hz: u32, fuses: FuseBytes, bootloader: Bootloader, flash: &[u8]) -> Bridge {
        let mut cpu = Cpu::new();
        cpu.set_clock_hz(clock_hz);
        cpu.set_fuses(fuses.low, fuses.high, fuses.extended);
        cpu.set_bootloader(bootloader);
        cpu.load_flash(flash);
        let mut bridge = Bridge {
            timeline: Timeline::new(cpu),
            clock_hz: u64::from(clock_hz),
            image: [Drive::Off; PORT_PINS.len()],
            levels: Vec::new(),
            phases: 0,
            refused: None,
        };
        bridge.refresh();
        bridge
    }

    /// Supply rises at 0 ps: the chip is held in reset from there, so the operating point at
    /// t = 0 sees its reset current, and the first instruction runs after the start from reset
    /// the fuses select and the bootloader's way to the sketch, as [`Cpu::power_on`] says.
    /// Called before the first run.
    pub fn power_on(&mut self) {
        self.timeline.power_on();
        self.refresh();
    }

    pub fn cpu(&self) -> &Cpu {
        self.timeline.cpu()
    }

    /// The last oscillator cycle at or before `at`.
    fn cycle(&self, at: Time) -> u64 {
        u64::try_from(u128::from(at.as_ps()) * u128::from(self.clock_hz) / 1_000_000_000_000)
            .expect("an oscillator cycle count fits in u64")
    }

    /// The first picosecond at or after oscillator cycle `cycle`.
    fn time(&self, cycle: u64) -> Time {
        let at = (u128::from(cycle) * 1_000_000_000_000).div_ceil(u128::from(self.clock_hz));
        Time::from_ps(u64::try_from(at).expect("a cycle inside the run"))
    }

    fn refresh(&mut self) {
        let drives = self.timeline.drives();
        for (drive, &pin) in self.image.iter_mut().zip(&DIE) {
            *drive = match drives[pin as usize] {
                PinDrive::High => Drive::High,
                PinDrive::Low => Drive::Low,
                PinDrive::PullUp => Drive::PullUp,
                PinDrive::Off => Drive::Off,
            };
        }
    }
}

impl Mcu for Bridge {
    const USARTS: &'static [&'static str] = &["USART0"];
    const TWI: bool = true;
    const WORLD: &'static [Quantity] = &[Quantity::Temperature];

    #[inline]
    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let halt = self.timeline.run_until(self.cycle(limit))?;
        self.refresh();
        if halt.refused {
            self.refused = Some(self.time(halt.at));
        }
        Some(Stop {
            at: self.time(halt.at),
            sample: halt.sample,
            record: false,
        })
    }

    #[inline]
    fn drive(&self) -> &[Drive] {
        &self.image
    }

    /// Held while `VCC` is under `VPOT` or the brown-out window or `RESET` is low, from the
    /// cycle the timeline stops at for it.
    fn held(&self) -> bool {
        self.timeline.held()
    }

    /// The port pins' levels in [`PORT_PINS`] order, then `VCC` above the brown-out window
    /// when the fuses enable it, then `VCC` above `VPOT`. Each comes after the pins, so a
    /// `RESET` that reads low because `VCC` fell to 0 V is already low when the power-on reset
    /// clears its flag.
    fn input(&mut self, _at: Time, levels: &[bool]) {
        let brown_out = self.cpu().brown_out_window().is_some();
        for (k, &level) in levels.iter().enumerate() {
            if self.levels.get(k) == Some(&level) {
                continue;
            }
            match DIE.get(k) {
                Some(&pin) => self.timeline.set_pad(pin, level),
                None if brown_out && k == DIE.len() => self.timeline.set_brown_out(!level),
                None => self.timeline.set_power_on_reset(!level),
            }
        }
        self.levels = levels.to_vec();
    }

    /// A port pin is read while its drive is off once the firmware loads its port's `PINx`,
    /// which holds all eight, or while its `INTn` or `PCINTn` interrupt is enabled.
    fn read(&self, output: usize) -> Option<Time> {
        let pin = *DIE.get(output)?;
        self.cpu().pin_read_at(pin).map(|cycle| self.time(cycle))
    }

    /// ADC0 to ADC7, `AVCC`, and `AREF`, each against the analog GND. The §24.8 sensor reads
    /// the world's temperature when the scene declares one, and its 25 °C default when not.
    fn sample(&mut self, _at: Time, volts: &[f64], world: &Readings) {
        if let Some(celsius) = world.get(Quantity::Temperature) {
            self.timeline.set_temperature_c(celsius as f32);
        }
        let v = |k: usize| volts[k] as f32;
        self.timeline.sample(std::array::from_fn(v), v(8), v(9));
    }

    /// Takes the frames whose stop bit ended on a cycle at or before `through`'s last, which is
    /// the same as that cycle's first picosecond being at or before `through`, and the TWI
    /// phases that ended by then. Drops every SPI byte, which no output reads.
    fn deliver(&mut self, through: Time, mut sent: impl FnMut(Frame), mut saw: impl FnMut(Phase)) {
        let through = self.cycle(through);
        while let Some(frame) = self.timeline.take_sent(through) {
            sent(Frame {
                usart: frame.usart,
                byte: frame.byte,
                start: self.time(frame.start),
                end: self.time(frame.end),
            });
        }
        for channel in [Channel::Spi, Channel::SpiHostMaster] {
            while self.timeline.take(channel).is_some() {}
        }
        while let Some(stamp) = self.timeline.take_i2c_through(through) {
            let phase = match stamp.note {
                I2cNote::Start { repeated: false } => I2cPhase::Start,
                I2cNote::Start { repeated: true } => I2cPhase::RepeatedStart,
                I2cNote::Address { addr, read, ack } => I2cPhase::Address {
                    address: addr,
                    read,
                    ack,
                },
                I2cNote::Data { byte, write, ack } => I2cPhase::Data {
                    byte,
                    read: !write,
                    ack,
                },
                I2cNote::Stop => I2cPhase::Stop,
                I2cNote::ArbitrationLost => I2cPhase::ArbitrationLost,
                I2cNote::BusError => I2cPhase::BusError,
            };
            saw(Phase {
                phase,
                start: self.time(stamp.start),
                end: self.time(stamp.at),
                count: self.phases,
            });
            self.phases += 1;
        }
    }

    /// The instruction or setting the chip refused, from the stop that gave it. The checks
    /// and their datasheet reasons are in the bus and the CPU.
    fn fault(&self) -> Option<(Time, Fault)> {
        let at = self.refused?;
        let refusal = self.cpu().refusal()?;
        let pc = Pc {
            pc: u32::from(refusal.pc) * 2,
            core: 0,
        };
        let setting = |peripheral, setting| Fault::UnsupportedSetting {
            peripheral,
            setting,
            pc,
        };
        let fault = match refusal.what {
            Refused::Instruction(op) => Fault::UnsupportedInstruction {
                op,
                reason: Unsupported::Op,
                pc,
            },
            Refused::Setting {
                peripheral,
                setting: name,
            } => setting(peripheral, Setting::Register(name)),
            Refused::Interrupt { peripheral } => setting(peripheral, Setting::Interrupt),
            Refused::Sleep => setting("CPU", Setting::Sleep),
            Refused::Unsimulated { peripheral, what } => {
                setting(peripheral, Setting::Unsimulated(what))
            }
        };
        Some((at, fault))
    }

    /// Each register the firmware reached by `through` that the model keeps without running
    /// what it does, once per run.
    fn notes(&self, through: Time) -> Vec<(Time, Note)> {
        self.cpu()
            .noted()
            .iter()
            .take_while(|noted| self.cpu().refusal().is_none_or(|r| noted.at <= r.at))
            .map(|noted| (self.time(noted.at), noted))
            .filter(|&(at, _)| at <= through)
            .map(|(at, noted)| {
                let note = Note::UnmodeledRegister {
                    address: u32::from(noted.what.address()),
                    register: noted.what.register().to_string(),
                    pc: Pc {
                        pc: u32::from(noted.pc) * 2,
                        core: 0,
                    },
                    write: noted.write,
                    kept: true,
                };
                (at, note)
            })
            .collect()
    }
}
