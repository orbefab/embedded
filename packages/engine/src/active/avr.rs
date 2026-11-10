//! The ATmega328P behind the seam. Its package pins become outputs, watches, and samples
//! around `orbe_avr::Timeline`, and its oscillator converts between picoseconds and cycles.

use crate::i2c::I2cPhase;
use crate::netlist::{Atmega328p, PartId, Value};
use crate::time::Time;
use orbe_avr::{Channel, Cpu, I2cNote, PinDrive, Port, Timeline};

use super::{Chip, Drive, Frame, Output, Phase, Stage, Stop, Takes};
use crate::mna::Node;
use crate::watch::{Signal, Watch};

/// What a package pin is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pad {
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

/// What each package pin is, in `Atmega328p::PINS` order, the DIP's. Pin 1 is `PC6`, which is
/// `RESET` while the fuses keep it so.
pub(crate) const PACKAGE: [Pad; 28] = [
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

/// `RESET` while `RSTDISBL` is unprogrammed.
const RESET: Pad = Pad::Port(Port::C, 6);

/// The port pins in package order, each with the die's id for it. The image and the watches
/// follow this order.
const PORT_PINS: [(usize, u32); 21] = {
    let mut pins = [(0, 0); 21];
    let (mut k, mut n) = (0, 0);
    while k < PACKAGE.len() {
        if let Pad::Port(port, bit) = PACKAGE[k] {
            pins[n] = (k, port.pin_id(bit));
            n += 1;
        }
        k += 1;
    }
    assert!(n == pins.len());
    pins
};

const VCC: usize = 6;
const GND_8: usize = 7;
const _: () = assert!(matches!(PACKAGE[VCC], Pad::Vcc));
const _: () = assert!(matches!(PACKAGE[GND_8], Pad::Gnd));

/// The pin pairs the chip joins at DC, for `net.floating`, by package index: each port pin to
/// VCC and to `GND_8` through its leakage, which every drive state stamps. The crystal pins and
/// `AREF` join nothing. Neither does `AVCC`: every stage draws from VCC, and `AVCC` is read only
/// at a sample. Parse puts `GND_22` on `GND_8`'s net.
pub(crate) const DC_PATHS: [(usize, usize); 42] = {
    let mut paths = [(0, 0); 42];
    let mut k = 0;
    while k < PORT_PINS.len() {
        let pin = PORT_PINS[k].0;
        paths[2 * k] = (pin, VCC);
        paths[2 * k + 1] = (pin, GND_8);
        k += 1;
    }
    paths
};

/// The package index of `pad`.
fn package_pin(pad: Pad) -> usize {
    PACKAGE
        .iter()
        .position(|&p| p == pad)
        .expect("a pad of the package")
}

/// The output and watch index of port pin `pad`.
pub(crate) fn port_index(pad: Pad) -> usize {
    PORT_PINS
        .iter()
        .position(|&(k, _)| PACKAGE[k] == pad)
        .expect("a port pin")
}

#[derive(Debug)]
pub(crate) struct Core {
    timeline: Timeline,
    clock_hz: u64,
    image: [Drive; PORT_PINS.len()],
    /// The levels it last took, in watch order. Empty before the power-on input.
    levels: Vec<bool>,
    /// How many TWI phases `deliver` has taken, which numbers the next one.
    phases: u64,
}

/// By hand for `clone_from`, which reuses the timeline's buffers. The patterns name every field.
impl Clone for Core {
    fn clone(&self) -> Self {
        let Core {
            timeline,
            clock_hz,
            image,
            levels,
            phases,
        } = self;
        Core {
            timeline: timeline.clone(),
            clock_hz: *clock_hz,
            image: *image,
            levels: levels.clone(),
            phases: *phases,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        let Core {
            timeline,
            clock_hz,
            image,
            levels,
            phases,
        } = self;
        timeline.clone_from(&source.timeline);
        *clock_hz = source.clock_hz;
        *image = source.image;
        levels.clone_from(&source.levels);
        *phases = source.phases;
    }
}

pub(crate) fn chip(part: PartId, mcu: &Atmega328p) -> Chip {
    let node = |pad: Pad| Node::from(mcu.pins[package_pin(pad)]);
    let (vcc, gnd) = (node(Pad::Vcc), node(Pad::Gnd));
    let clock_hz = u32::try_from(mcu.clock_hz.value).expect("parse keeps the clock in u32");
    let fuses = mcu.fuses;
    let mut cpu = Cpu::new();
    cpu.set_clock_hz(clock_hz);
    cpu.set_fuses(fuses.low(), fuses.high(), fuses.extended());
    cpu.load_flash(mcu.firmware.flash());

    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let high = stage(&mcu.output.high_drop, &mcu.output.high_resistance);
    let low = stage(&mcu.output.low_drop, &mcu.output.low_resistance);

    // `RSTDISBL` unprogrammed: `PC6` is `RESET`, with its own pull-up and thresholds.
    let reset = fuses.has_reset().then(|| port_index(RESET));
    let outputs = PORT_PINS
        .iter()
        .enumerate()
        .map(|(n, &(k, _))| Output {
            pin: mcu.pins[k],
            vcc: mcu.pins[package_pin(Pad::Vcc)],
            gnd: mcu.pins[package_pin(Pad::Gnd)],
            high,
            low,
            pull_up: Some(if reset == Some(n) {
                mcu.reset_pull_up_resistance.si.get()
            } else {
                mcu.pull_up_resistance.si.get()
            }),
            leakage: Some(mcu.leakage_resistance.si.get()),
        })
        .collect();

    // V(pin) - f * V(VCC) - (1 - f) * V(GND): the pin against a fraction of its own supply.
    let against = |k: usize, fraction: f64| Signal {
        terms: vec![
            (Node::from(mcu.pins[k]), 1.0),
            (vcc, -fraction),
            (gnd, fraction - 1.0),
        ],
        offset: 0.0,
    };
    let mut watches: Vec<Watch> = PORT_PINS
        .iter()
        .enumerate()
        .map(|(n, &(k, _))| {
            let threshold = if reset == Some(n) {
                &mcu.reset_threshold
            } else {
                &mcu.input_threshold
            };
            Watch {
                rise: against(k, threshold.high.si.get()),
                fall: against(k, threshold.low.si.get()),
            }
        })
        .collect();
    let mut signals: Vec<(&'static str, usize)> = PORT_PINS
        .iter()
        .enumerate()
        .map(|(n, &(k, _))| (Atmega328p::PINS[k], n))
        .collect();
    if let Some((below, above)) = cpu.brown_out_window() {
        signals.push((Atmega328p::PINS[VCC], watches.len()));
        let supply = |millivolts: u16| Signal {
            terms: vec![(vcc, 1.0), (gnd, -1.0)],
            offset: f64::from(millivolts) / 1000.0,
        };
        watches.push(Watch {
            rise: supply(above),
            fall: supply(below),
        });
    }
    signals.sort_unstable();

    // ADC0 to ADC5 are `PC0` to `PC5`. The DIP has no ADC6 or ADC7.
    let to_agnd = |pad: Pad| Signal {
        terms: vec![(node(pad), 1.0), (node(Pad::Agnd), -1.0)],
        offset: 0.0,
    };
    let samples = (0..6)
        .map(|bit| Pad::Port(Port::C, bit))
        .chain([Pad::Avcc, Pad::Aref])
        .map(to_agnd)
        .collect();

    let mut core = Core {
        timeline: Timeline::new(cpu),
        clock_hz: u64::from(clock_hz),
        image: [Drive::Off; PORT_PINS.len()],
        levels: Vec::new(),
        phases: 0,
    };
    core.refresh();
    Chip {
        part: Some(part),
        outputs,
        watches,
        signals,
        samples,
        usarts: &["USART0"],
        twi: true,
        sends: false,
        takes: Takes::default(),
        core: super::Core::Avr(Box::new(core)),
    }
}

impl Core {
    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let halt = self.timeline.run_until(self.cycle(limit))?;
        self.refresh();
        Some(Stop {
            at: self.time(halt.at),
            sample: halt.sample,
            record: false,
        })
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

    pub(crate) fn drive(&self) -> &[Drive] {
        &self.image
    }

    /// ADC0 to ADC5, `AVCC`, and `AREF`, each against the analog GND.
    pub(crate) fn sample(&mut self, _at: Time, volts: &[f64]) {
        let v = |k: usize| volts[k] as f32;
        self.timeline
            .sample([v(0), v(1), v(2), v(3), v(4), v(5), 0.0, 0.0], v(6), v(7));
    }

    /// The port pins' levels in [`PORT_PINS`] order, then `VCC` above the brown-out window
    /// when the fuses enable it.
    pub(crate) fn input(&mut self, _at: Time, levels: &[bool]) {
        for (k, &level) in levels.iter().enumerate() {
            if self.levels.get(k) == Some(&level) {
                continue;
            }
            match PORT_PINS.get(k) {
                Some(&(_, pin)) => self.timeline.set_pad(pin, level),
                None => self.timeline.set_brown_out(!level),
            }
        }
        self.levels = levels.to_vec();
    }

    #[cfg(test)]
    pub(crate) fn cpu(&self) -> &Cpu {
        self.timeline.cpu()
    }

    /// Takes the frames whose stop bit ended on a cycle at or before `through`'s last, which is
    /// the same as that cycle's first picosecond being at or before `through`, and the TWI
    /// phases that ended by then.
    pub(crate) fn deliver(
        &mut self,
        through: Time,
        mut sent: impl FnMut(Frame),
        mut saw: impl FnMut(Phase),
    ) {
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

    fn refresh(&mut self) {
        let drives = self.timeline.drives();
        for (drive, &(_, pin)) in self.image.iter_mut().zip(&PORT_PINS) {
            *drive = match drives[pin as usize] {
                PinDrive::High => Drive::High,
                PinDrive::Low => Drive::Low,
                PinDrive::PullUp => Drive::PullUp,
                PinDrive::Off => Drive::Off,
            };
        }
    }
}
