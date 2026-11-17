//! The `atmega328p` part: its netlist struct and how parse reads it, and the outputs, watches
//! and samples its package pins give the circuit around `orbe_microchip::Bridge`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::active::{Chip, Output, Stage, Supply};
use crate::diagnostic::{Diagnostic, FirmwareRule, ValueRule};
use crate::mna::Node;
use crate::name::{Name, Ref};
use crate::netlist::IntValue;
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Threshold;
use crate::netlist::Value;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::WireThreshold;
use crate::netlist::json::threshold::resolve_threshold;
use crate::netlist::json::{
    PinMap, WireInt, WireValue, resolve_int, resolve_pins, resolve_rating, resolve_value,
};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};
use crate::ratings::{Ceiling, PinRatings, PinWindow};
use crate::watch::{Signal, Watch};
use orbe_microchip::{Bridge, FuseBytes, PACKAGE, PORT_PINS, Pad, Port, port_index};

/// The supply at which the chip's own conductance draws `active_current`, volt.
const ACTIVE_AT: f64 = 5.0;

/// `RESET` while `RSTDISBL` is unprogrammed.
const RESET: Pad = Pad::Port(Port::C, 6);

const VCC: usize = 6;
const GND_8: usize = 7;
const GND_22: usize = 21;
const _: () = assert!(matches!(PACKAGE[VCC], Pad::Vcc));
const _: () = assert!(matches!(PACKAGE[GND_8], Pad::Gnd));
const _: () = assert!(matches!(PACKAGE[GND_22], Pad::Agnd));

/// The pin pairs the chip joins at DC, for `net.floating`, by package index: each port pin to
/// VCC and to `GND_8` through its leakage, which every drive state stamps. The crystal pins and
/// `AREF` join nothing. Neither does `AVCC`: every stage draws from VCC, and `AVCC` is read only
/// at a sample. Parse puts `GND_22` on `GND_8`'s net.
pub(crate) const DC_PATHS: [(usize, usize); 42] = {
    let mut paths = [(0, 0); 42];
    let mut k = 0;
    while k < PORT_PINS.len() {
        let pin = PORT_PINS[k];
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

pub(crate) fn chip(part: PartId, mcu: &Atmega328p) -> Chip {
    let node = |pad: Pad| Node::from(mcu.pins[package_pin(pad)]);
    let (vcc, gnd) = (node(Pad::Vcc), node(Pad::Gnd));
    let clock_hz = u32::try_from(mcu.clock_hz.value).expect("parse keeps the clock in u32");
    let fuses = mcu.fuses;
    let bridge = Bridge::new(
        clock_hz,
        FuseBytes {
            low: fuses.low(),
            high: fuses.high(),
            extended: fuses.extended(),
        },
        mcu.firmware.flash(),
    );

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
        .map(|(n, &k)| Output {
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
        .map(|(n, &k)| {
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
        .map(|(n, &k)| (Atmega328p::PINS[k], n))
        .collect();
    if let Some((below, above)) = bridge.cpu().brown_out_window() {
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

    let mut chip = super::chip(part, outputs, watches, signals, samples, bridge);
    chip.supply = mcu.active_current.as_ref().map(|current| Supply {
        vcc: mcu.pins[VCC],
        gnd: mcu.pins[GND_8],
        conductance: current.si.get() / ACTIVE_AT,
    });
    chip
}

/// The part's ratings on its package pins, `None` when it has no `ratings` object. Each port
/// pin is an output, in `PORT_PINS` order. Every pin but the two ground pins has a window over
/// `GND_8`: `rail_margin` below it, and `rail_margin` above `VCC`, but for `VCC`, whose
/// ceiling is `supply_voltage`, and `PC6` while it is `RESET`, whose ceiling is
/// `reset_voltage` (DS40002061B §29.1).
pub(crate) fn pin_ratings(mcu: &Atmega328p) -> Option<PinRatings> {
    let ratings = mcu.ratings.as_ref()?;
    let si = |value: &Option<Value>| value.as_ref().map(|value| value.si.get());
    let margin = si(&ratings.rail_margin);
    let reset = mcu.fuses.has_reset().then(|| package_pin(RESET));
    let windows = (0..Atmega328p::PINS.len())
        .filter(|&k| k != GND_8 && k != GND_22)
        .filter_map(|k| {
            let above = if k == VCC {
                si(&ratings.supply_voltage).map(Ceiling::Fixed)
            } else if Some(k) == reset {
                si(&ratings.reset_voltage).map(Ceiling::Fixed)
            } else {
                margin.map(Ceiling::OverSupply)
            };
            (margin.is_some() || above.is_some()).then(|| PinWindow {
                pin: Atmega328p::PINS[k],
                net: mcu.pins[k],
                below: margin,
                above,
            })
        })
        .collect();
    Some(PinRatings {
        outputs: PORT_PINS.iter().map(|&k| Atmega328p::PINS[k]).collect(),
        pin_current: si(&ratings.pin_current),
        supply_current: si(&ratings.supply_current),
        vcc: (Atmega328p::PINS[VCC], mcu.pins[VCC]),
        gnd: (Atmega328p::PINS[GND_8], mcu.pins[GND_8]),
        windows,
    })
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
    /// Ampere, greater than 0. The core's own draw at `VCC` = 5 V, a conductance from `VCC` to
    /// `GND_8` in every state. `None` when the part leaves it out, and the chip then draws only
    /// what its pins pass.
    pub active_current: Option<Value>,
    /// The absolute maximum ratings the run checks. `None` when the part has no `ratings`
    /// object, which leaves the chip unrated.
    pub ratings: Option<Atmega328pRatings>,
}

/// An ATmega328P's absolute maximum ratings. Each is `None` when the part leaves it unrated.
#[derive(Clone, Debug, PartialEq)]
pub struct Atmega328pRatings {
    /// Ampere. The current through any one port pin.
    pub pin_current: Option<Value>,
    /// Ampere. The current through `VCC`, and the current through the ground pins.
    pub supply_current: Option<Value>,
    /// Volt. How far a pin may go below `GND_8`, and every pin but `VCC` and `RESET` above
    /// `VCC`.
    pub rail_margin: Option<Value>,
    /// Volt. The most `PC6` may sit above `GND_8` while it is `RESET`.
    pub reset_voltage: Option<Value>,
    /// Volt. The most `VCC` may sit above `GND_8`.
    pub supply_voltage: Option<Value>,
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
    /// Optional, so a netlist written before the chip drew its own current keeps its meaning.
    #[serde(default)]
    active_current: Option<WireValue>,
    /// Optional, so a netlist written before the chip had ratings keeps its meaning.
    #[serde(default)]
    ratings: Option<WireRatings>,
}

/// Every key is required, and `null` leaves that rating out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    pin_current: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_current: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    rail_margin: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    reset_voltage: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_voltage: Option<WireValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFuses {
    low: u8,
    high: u8,
    extended: u8,
}

pub(crate) fn resolve_atmega328p(ctx: ParseCtx<'_>, wire: WireAtmega328p) -> Option<Atmega328p> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
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
    let active_current = match wire.active_current {
        None => Some(None),
        Some(value) => resolve_value(part_ref, "active_current", "A", true, value, out).map(Some),
    };
    let ratings = match wire.ratings {
        None => Some(None),
        Some(wire) => resolve_ratings(part_ref, wire, out).map(Some),
    };
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
        active_current: active_current?,
        ratings: ratings?,
    })
}

fn resolve_ratings(
    part: &Ref,
    wire: WireRatings,
    out: &mut Vec<Diagnostic>,
) -> Option<Atmega328pRatings> {
    let pin_current = resolve_rating(part, "ratings.pin_current", "A", wire.pin_current, out);
    let supply_current = resolve_rating(
        part,
        "ratings.supply_current",
        "A",
        wire.supply_current,
        out,
    );
    let rail_margin = resolve_rating(part, "ratings.rail_margin", "V", wire.rail_margin, out);
    let reset_voltage = resolve_rating(part, "ratings.reset_voltage", "V", wire.reset_voltage, out);
    let supply_voltage = resolve_rating(
        part,
        "ratings.supply_voltage",
        "V",
        wire.supply_voltage,
        out,
    );
    Some(Atmega328pRatings {
        pin_current: pin_current?,
        supply_current: supply_current?,
        rail_margin: rail_margin?,
        reset_voltage: reset_voltage?,
        supply_voltage: supply_voltage?,
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
