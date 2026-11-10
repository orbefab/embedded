//! The ATmega328P behind the seam. Its package pins become outputs, watches, and samples
//! around `orbe_microchip::Timeline`, and its oscillator converts between picoseconds and cycles.

use std::collections::BTreeMap;

use serde::Deserialize;

use orbe_model::{Drive, Stop, Time};

use crate::active::{Chip, Frame, Output, Phase, Stage, Takes};
use crate::diagnostic::{Diagnostic, FirmwareRule, ValueRule};
use crate::i2c::I2cPhase;
use crate::mna::Node;
use crate::name::{Name, Ref};
use crate::netlist::IntValue;
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Threshold;
use crate::netlist::Value;
use crate::netlist::json::threshold::WireThreshold;
use crate::netlist::json::threshold::resolve_threshold;
use crate::netlist::json::{PinMap, WireInt, WireValue, resolve_int, resolve_pins, resolve_value};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};
use crate::watch::{Signal, Watch};
use orbe_microchip::{Channel, Cpu, I2cNote, PinDrive, Port, Timeline};

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
        core: crate::active::Core::Avr(Box::new(core)),
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

/// An ATmega328P in its 28-pin DIP, running `firmware` from power-on. Microchip DS40002061B.
///
/// Each I/O pin drives through `output` from VCC and into `GND_8`, pulls up through
/// `pull_up_resistance`, or is off, as the firmware sets it, leaks to VCC and to `GND_8` through
/// `leakage_resistance`, and reads its level through `input_threshold`. `PC6` is `RESET` while
/// `RSTDISBL` is unprogrammed, with its own pull-up and thresholds. `PB6` and `PB7` hold the
/// crystal or resonator the fuses select, and `clock_hz` is its frequency. Only parse builds
/// one, so `GND_8` and `GND_22` name one net.
#[derive(Clone, Debug, PartialEq)]
pub struct Atmega328p {
    pub pins: [NetId; 28],
    pub firmware: Firmware,
    /// Hertz, greater than 0, at most `u32::MAX`.
    pub clock_hz: IntValue,
    pub fuses: Fuses,
    pub output: OutputStage,
    /// Ohm. Every I/O pin's pull-up.
    pub pull_up_resistance: Value,
    /// Ohm. `PC6`'s pull-up while it is `RESET`.
    pub reset_pull_up_resistance: Value,
    /// Ohm. Every port pin's to VCC and again to `GND_8`, in every drive state, `PC6` as
    /// `RESET` included.
    pub leakage_resistance: Value,
    /// Every I/O pin's.
    pub input_threshold: Threshold,
    /// `PC6`'s while it is `RESET`.
    pub reset_threshold: Threshold,
}

impl Atmega328p {
    /// DIP order, DS40002061B Figure 1-1. The two ground pins carry their pin numbers.
    pub const PINS: [&'static str; 28] = [
        "PC6", "PD0", "PD1", "PD2", "PD3", "PD4", "VCC", "GND_8", "PB6", "PB7", "PD5", "PD6",
        "PD7", "PB0", "PB1", "PB2", "PB3", "PB4", "PB5", "AVCC", "AREF", "GND_22", "PC0", "PC1",
        "PC2", "PC3", "PC4", "PC5",
    ];
    /// The flash, 32 KB, DS40002061B Table 2-1.
    pub const FLASH_BYTES: usize = 32 * 1024;
}

/// A program image from flash address 0. Only parse builds one, so it fits the flash.
#[derive(Clone, Debug, PartialEq)]
pub struct Firmware {
    flash: Vec<u8>,
}

impl Firmware {
    /// At most `Atmega328p::FLASH_BYTES`. A gap between records is erased flash, `0xFF`.
    pub fn flash(&self) -> &[u8] {
        &self.flash
    }
}

/// The three fuse bytes as programmed, DS40002061B Tables 28-6, 28-8 and 28-9, where a
/// programmed bit is 0. Only parse builds one, so `CKSEL` selects a crystal or ceramic
/// resonator (Table 9-1, `0110` to `1111`), `CKOUT`, `DWEN` and `BOOTRST` are unprogrammed,
/// and `BODLEVEL` is a level of Table 29-12 or disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fuses {
    low: u8,
    high: u8,
    extended: u8,
}

impl Fuses {
    pub fn low(self) -> u8 {
        self.low
    }

    pub fn high(self) -> u8 {
        self.high
    }

    pub fn extended(self) -> u8 {
        self.extended
    }

    /// `RSTDISBL` unprogrammed: `PC6` is `RESET`.
    pub fn has_reset(self) -> bool {
        self.high & 0x80 != 0
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireAtmega328p {
    pub(crate) name: String,
    pins: PinMap,
    firmware: String,
    clock_hz: WireInt<u32>,
    fuses: WireFuses,
    output: WireOutputStage,
    pull_up_resistance: WireValue,
    reset_pull_up_resistance: WireValue,
    leakage_resistance: WireValue,
    input_threshold: WireThreshold,
    reset_threshold: WireThreshold,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFuses {
    low: u8,
    high: u8,
    extended: u8,
}

pub(crate) fn resolve_atmega328p(
    name: &Name,
    part_ref: &Ref,
    wire: WireAtmega328p,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Atmega328p> {
    let ohm = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part_ref, field, "ohm", true, value, out)
    };
    let pins = resolve_pins(name, Atmega328p::PINS, &wire.pins.0, nets, out)
        .and_then(|pins| one_ground(name, pins, nets, out));
    let firmware = match flash(&wire.firmware) {
        Ok(flash) => Some(Firmware { flash }),
        Err((line, rule)) => {
            out.push(Diagnostic::NetlistFirmware {
                part: part_ref.clone(),
                line,
                rule,
            });
            None
        }
    };
    let clock_hz = resolve_int(part_ref, "clock_hz", "Hz", wire.clock_hz, out);
    let fuses = resolve_fuses(part_ref, &wire.fuses, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let pull_up = ohm("pull_up_resistance", wire.pull_up_resistance, out);
    let reset_pull_up = ohm(
        "reset_pull_up_resistance",
        wire.reset_pull_up_resistance,
        out,
    );
    let leakage = ohm("leakage_resistance", wire.leakage_resistance, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let reset_threshold = resolve_threshold(
        part_ref,
        ["reset_threshold.low", "reset_threshold.high"],
        wire.reset_threshold,
        out,
    );
    Some(Atmega328p {
        pins: pins?,
        firmware: firmware?,
        clock_hz: clock_hz?,
        fuses: fuses?,
        output: output?,
        pull_up_resistance: pull_up?,
        reset_pull_up_resistance: reset_pull_up?,
        leakage_resistance: leakage?,
        input_threshold: input_threshold?,
        reset_threshold: reset_threshold?,
    })
}

/// Figure 1-1 names pins 8 and 22 `GND`, one function, so they name one net.
fn one_ground(
    part: &Name,
    pins: [NetId; 28],
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<[NetId; 28]> {
    let index = |pin| Atmega328p::PINS.iter().position(|&p| p == pin).unwrap();
    let (gnd_8, gnd_22) = (pins[index("GND_8")], pins[index("GND_22")]);
    if gnd_8 == gnd_22 {
        return Some(pins);
    }
    let name = |id| {
        let (net, _) = nets.iter().find(|&(_, &net)| net == id).unwrap();
        Name::new(net).expect("a resolved net name")
    };
    let pin = |pin| Ref::Pin(part.clone(), Name::new(pin).expect("kind pin name"));
    out.push(Diagnostic::NetlistSameNet {
        pin: pin("GND_22"),
        with: pin("GND_8"),
        found: name(gnd_22),
        expected: name(gnd_8),
    });
    None
}

/// Each byte the model would not run is `unsupported`: a `CKSEL` outside the crystal and
/// ceramic resonator rows of Table 9-1 or a programmed `CKOUT` (Table 28-9), a programmed
/// `DWEN` or `BOOTRST` (Table 28-8), or a reserved `BODLEVEL` (Table 29-12).
fn resolve_fuses(part: &Ref, wire: &WireFuses, out: &mut Vec<Diagnostic>) -> Option<Fuses> {
    let checks = [
        (
            "fuses.low",
            wire.low,
            matches!(wire.low & 0x0F, 0b0110..=0b1111) && wire.low & 0x40 != 0,
        ),
        ("fuses.high", wire.high, wire.high & 0x41 == 0x41),
        (
            "fuses.extended",
            wire.extended,
            matches!(wire.extended & 0x07, 0b100..=0b111),
        ),
    ];
    let mut ok = true;
    for (field, byte, supported) in checks {
        if !supported {
            ok = false;
            out.push(Diagnostic::NetlistValue {
                part: part.clone(),
                field: field.into(),
                value: f64::from(byte),
                rule: ValueRule::Unsupported,
            });
        }
    }
    ok.then_some(Fuses {
        low: wire.low,
        high: wire.high,
        extended: wire.extended,
    })
}

/// Flash from Intel HEX text, one record a line. Records 00 to 05 are read: 02 and 04 move the
/// base address, and 01, 03 and 05 may name a start address, which a part at reset does not
/// use. A gap is erased flash, `0xFF`, and no byte is written twice. A fault names its 1-based
/// line.
fn flash(text: &str) -> Result<Vec<u8>, (Option<usize>, FirmwareRule)> {
    let mut flash = Vec::new();
    let mut placed = Vec::new();
    let mut base = 0;
    let mut ended = false;
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let fault = |rule| (Some(index + 1), rule);
        if ended {
            return Err(fault(FirmwareRule::AfterEnd));
        }
        let bytes = record(line).ok_or(fault(FirmwareRule::Record))?;
        if bytes.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte)) != 0 {
            return Err(fault(FirmwareRule::Checksum));
        }
        let address = usize::from(u16::from_be_bytes([bytes[1], bytes[2]]));
        let data = &bytes[4..bytes.len() - 1];
        let upper = |shift: u32| match *data {
            [high, low] => Ok(usize::from(u16::from_be_bytes([high, low])) << shift),
            _ => Err(fault(FirmwareRule::Record)),
        };
        match bytes[3] {
            0x00 => {
                let start = base + address;
                let end = start + data.len();
                if end > Atmega328p::FLASH_BYTES {
                    return Err(fault(FirmwareRule::PastFlash));
                }
                if flash.len() < end {
                    flash.resize(end, 0xFF);
                    placed.resize(end, false);
                }
                if placed[start..end].contains(&true) {
                    return Err(fault(FirmwareRule::Overlap));
                }
                flash[start..end].copy_from_slice(data);
                placed[start..end].fill(true);
            }
            0x01 if data.is_empty() => ended = true,
            0x01 => return Err(fault(FirmwareRule::Record)),
            0x02 => base = upper(4)?,
            0x04 => base = upper(16)?,
            0x03 | 0x05 if data.len() == 4 => {}
            0x03 | 0x05 => return Err(fault(FirmwareRule::Record)),
            _ => return Err(fault(FirmwareRule::RecordType)),
        }
    }
    if !ended {
        return Err((None, FirmwareRule::NoEnd));
    }
    Ok(flash)
}

/// The record's bytes, its count through its checksum, when its length matches its count.
fn record(line: &str) -> Option<Vec<u8>> {
    let digits = line.strip_prefix(':')?.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Vec<u8> = digits
        .chunks(2)
        .map(|pair| {
            let nibble = |digit: u8| char::from(digit).to_digit(16);
            Some((nibble(pair[0])? << 4 | nibble(pair[1])?) as u8)
        })
        .collect::<Option<_>>()?;
    (bytes.len() >= 5 && bytes.len() == usize::from(bytes[0]) + 5).then_some(bytes)
}
