//! The ATmega328P on the `orbe-mcu` contract. Its port pins become the image and the watches
//! around [`Timeline`], in package order, and its oscillator converts between picoseconds and
//! cycles.

use orbe_mcu::{Frame, I2cPhase, Mcu, Phase};
use orbe_model::{Drive, Stop, Time};

use crate::{Channel, Cpu, FuseBytes, I2cNote, PinDrive, Port, Timeline};

/// What a package pin is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pad {
    /// `P<port><bit>`, which the chip drives and reads.
    Port(Port, u8),
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

/// What each pin of the 28-pin DIP is, pin 1 first. Pin 1 is `PC6`, which is `RESET` while the
/// fuses keep it so.
pub const PACKAGE: [Pad; 28] = [
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
}

clone_fields!(Bridge {
    timeline,
    clock_hz,
    image,
    levels,
    phases
});

impl Bridge {
    /// The chip at power-on on a `clock_hz` crystal, with `fuses` programmed and `flash` from
    /// address 0.
    pub fn new(clock_hz: u32, fuses: FuseBytes, flash: &[u8]) -> Bridge {
        let mut cpu = Cpu::new();
        cpu.set_clock_hz(clock_hz);
        cpu.set_fuses(fuses.low, fuses.high, fuses.extended);
        cpu.load_flash(flash);
        let mut bridge = Bridge {
            timeline: Timeline::new(cpu),
            clock_hz: u64::from(clock_hz),
            image: [Drive::Off; PORT_PINS.len()],
            levels: Vec::new(),
            phases: 0,
        };
        bridge.refresh();
        bridge
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

    #[inline]
    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let halt = self.timeline.run_until(self.cycle(limit))?;
        self.refresh();
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

    /// The port pins' levels in [`PORT_PINS`] order, then `VCC` above the brown-out window
    /// when the fuses enable it.
    fn input(&mut self, _at: Time, levels: &[bool]) {
        for (k, &level) in levels.iter().enumerate() {
            if self.levels.get(k) == Some(&level) {
                continue;
            }
            match DIE.get(k) {
                Some(&pin) => self.timeline.set_pad(pin, level),
                None => self.timeline.set_brown_out(!level),
            }
        }
        self.levels = levels.to_vec();
    }

    /// ADC0 to ADC5, `AVCC`, and `AREF`, each against the analog GND.
    fn sample(&mut self, _at: Time, volts: &[f64]) {
        let v = |k: usize| volts[k] as f32;
        self.timeline
            .sample([v(0), v(1), v(2), v(3), v(4), v(5), 0.0, 0.0], v(6), v(7));
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
}
