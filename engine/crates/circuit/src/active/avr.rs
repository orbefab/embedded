//! The ATmega328P behind the seam. Its package pins become outputs, watches, and samples
//! around `sim_avr::Timeline`, and its oscillator converts between picoseconds and cycles.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the netlist kind that builds this chip comes next"
    )
)]

use std::sync::Arc;

use sim_avr::{Cpu, FuseBytes, PinDrive, Port, Timeline};
use sim_core::netlist::NetId;
use sim_core::time::Time;

use super::{Chip, Drive, Output, Stage, Stop};
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

/// The package pins in DIP order. Pin 1 is `PC6`, which is `RESET` while the fuses keep it so.
pub(crate) const PACKAGE: [(&str, Pad); 28] = [
    ("PC6", Pad::Port(Port::C, 6)),
    ("PD0", Pad::Port(Port::D, 0)),
    ("PD1", Pad::Port(Port::D, 1)),
    ("PD2", Pad::Port(Port::D, 2)),
    ("PD3", Pad::Port(Port::D, 3)),
    ("PD4", Pad::Port(Port::D, 4)),
    ("VCC", Pad::Vcc),
    ("GND_8", Pad::Gnd),
    ("PB6", Pad::Xtal),
    ("PB7", Pad::Xtal),
    ("PD5", Pad::Port(Port::D, 5)),
    ("PD6", Pad::Port(Port::D, 6)),
    ("PD7", Pad::Port(Port::D, 7)),
    ("PB0", Pad::Port(Port::B, 0)),
    ("PB1", Pad::Port(Port::B, 1)),
    ("PB2", Pad::Port(Port::B, 2)),
    ("PB3", Pad::Port(Port::B, 3)),
    ("PB4", Pad::Port(Port::B, 4)),
    ("PB5", Pad::Port(Port::B, 5)),
    ("AVCC", Pad::Avcc),
    ("AREF", Pad::Aref),
    ("GND_22", Pad::Agnd),
    ("PC0", Pad::Port(Port::C, 0)),
    ("PC1", Pad::Port(Port::C, 1)),
    ("PC2", Pad::Port(Port::C, 2)),
    ("PC3", Pad::Port(Port::C, 3)),
    ("PC4", Pad::Port(Port::C, 4)),
    ("PC5", Pad::Port(Port::C, 5)),
];

/// `RESET` while `RSTDISBL` is unprogrammed.
const RESET: Pad = Pad::Port(Port::C, 6);

/// The port pins in package order, each with the die's id for it. The image and the watches
/// follow this order.
const PORT_PINS: [(usize, u32); 21] = {
    let mut pins = [(0, 0); 21];
    let (mut k, mut n) = (0, 0);
    while k < PACKAGE.len() {
        if let Pad::Port(port, bit) = PACKAGE[k].1 {
            pins[n] = (k, port.pin_id(bit));
            n += 1;
        }
        k += 1;
    }
    assert!(n == pins.len());
    pins
};

/// The package index of `pad`.
fn package_pin(pad: Pad) -> usize {
    PACKAGE
        .iter()
        .position(|&(_, p)| p == pad)
        .expect("a pad of the package")
}

/// The output and watch index of port pin `pad`.
pub(crate) fn port_index(pad: Pad) -> usize {
    PORT_PINS
        .iter()
        .position(|&(k, _)| PACKAGE[k].1 == pad)
        .expect("a port pin")
}

/// The output stage on each side: no drop, and 25 ohm. Figures 31-353 and 31-355 at 5 V and
/// 25 C read about 0.45 V at 20 mA sunk and 0.55 V below VCC at 20 mA sourced, 22.5 and
/// 27.5 ohm by eye. Both sides take 25 ohm, between the two, so an eye reading is not frozen
/// as a measured difference.
pub(crate) const STAGE: Stage = Stage {
    drop: 0.0,
    resistance: 25.0,
};

/// Ohm. Figure 31-348 at 5 V, typical: about 140 uA at 0 V, falling straight to 0 at 5 V.
pub(crate) const PULL_UP: f64 = 35_714.0;

/// Ohm. Figure 31-351 at 5 V, typical: about 108 uA at 0 V.
pub(crate) const RESET_PULL_UP: f64 = 46_296.0;

/// Where an input pin reads low and high, as fractions of the chip's VCC. Figures 31-357 and
/// 31-356 at 5 V, typical: about 2.1 V falling and 2.6 V rising.
pub(crate) const THRESHOLD: Threshold = Threshold {
    low: 0.42,
    high: 0.52,
};

/// Where `RESET` reads low and high, as fractions of the chip's VCC. Figures 31-360 and 31-359
/// at 5 V, typical: about 2.1 V falling and 2.2 V rising.
pub(crate) const RESET_THRESHOLD: Threshold = Threshold {
    low: 0.42,
    high: 0.44,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Threshold {
    /// A high input reads low at or below this fraction of VCC.
    pub(crate) low: f64,
    /// A low input reads high above this fraction of VCC.
    pub(crate) high: f64,
}

/// One ATmega328P, in plain values.
#[derive(Clone, Debug)]
pub(crate) struct Atmega328p {
    /// By package pin, in [`PACKAGE`] order.
    pub(crate) pins: [NetId; 28],
    pub(crate) high: Stage,
    pub(crate) low: Stage,
    /// Ohm, for every port pin but `RESET`.
    pub(crate) pull_up: f64,
    /// Ohm, for `PC6` while it is `RESET`.
    pub(crate) reset_pull_up: f64,
    /// For every port pin but `RESET`.
    pub(crate) threshold: Threshold,
    /// For `PC6` while it is `RESET`.
    pub(crate) reset_threshold: Threshold,
    /// The crystal, in hertz.
    pub(crate) clock_hz: u32,
    pub(crate) fuses: FuseBytes,
    /// The program, from flash address 0.
    pub(crate) flash: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub(crate) struct Core {
    timeline: Timeline,
    clock_hz: u64,
    image: [Drive; PORT_PINS.len()],
    /// The levels it last took, in watch order. Empty before the power-on input.
    levels: Vec<bool>,
}

/// The fuses pick a clock other than a crystal or ceramic resonator, which the adapter does
/// not run: `PB6` and `PB7` would be port pins, and the clock not the crystal's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotACrystal {
    /// The low fuse byte's `CKSEL3..0`.
    pub(crate) cksel: u8,
}

/// The chip at power-on.
pub(crate) fn chip(mcu: &Atmega328p) -> Result<Chip, NotACrystal> {
    if !mcu.fuses.has_crystal() {
        return Err(NotACrystal {
            cksel: mcu.fuses.low & 0x0F,
        });
    }
    let node = |pad: Pad| Node::from(mcu.pins[package_pin(pad)]);
    let (vcc, gnd) = (node(Pad::Vcc), node(Pad::Gnd));
    let mut cpu = Cpu::new();
    cpu.set_clock_hz(mcu.clock_hz);
    cpu.set_fuses(mcu.fuses.low, mcu.fuses.high, mcu.fuses.extended);
    cpu.load_flash(&mcu.flash);

    // `RSTDISBL` unprogrammed: `PC6` is `RESET`, with its own pull-up and thresholds.
    let reset = (mcu.fuses.high & 0x80 != 0).then(|| port_index(RESET));
    let outputs = PORT_PINS
        .iter()
        .enumerate()
        .map(|(n, &(k, _))| Output {
            pin: mcu.pins[k],
            vcc: mcu.pins[package_pin(Pad::Vcc)],
            gnd: mcu.pins[package_pin(Pad::Gnd)],
            high: mcu.high,
            low: mcu.low,
            pull_up: Some(if reset == Some(n) {
                mcu.reset_pull_up
            } else {
                mcu.pull_up
            }),
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
                mcu.reset_threshold
            } else {
                mcu.threshold
            };
            Watch {
                rise: against(k, threshold.high),
                fall: against(k, threshold.low),
            }
        })
        .collect();
    if let Some((below, above)) = cpu.brown_out_window() {
        let supply = |millivolts: u16| Signal {
            terms: vec![(vcc, 1.0), (gnd, -1.0)],
            offset: f64::from(millivolts) / 1000.0,
        };
        watches.push(Watch {
            rise: supply(above),
            fall: supply(below),
        });
    }

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
        clock_hz: u64::from(mcu.clock_hz),
        image: [Drive::Off; PORT_PINS.len()],
        levels: Vec::new(),
    };
    core.refresh();
    Ok(Chip {
        outputs,
        watches,
        samples,
        core: super::Core::Avr(Box::new(core)),
    })
}

impl Core {
    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let f = u128::from(self.clock_hz);
        let limit = u64::try_from(u128::from(limit.as_ps()) * f / 1_000_000_000_000)
            .expect("an oscillator cycle count fits in u64");
        let halt = self.timeline.run_until(limit)?;
        self.refresh();
        let at = (u128::from(halt.at) * 1_000_000_000_000).div_ceil(f);
        Some(Stop {
            at: Time::from_ps(u64::try_from(at).expect("a stop inside the run")),
            sample: halt.sample,
        })
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
