//! The `atmega328p` part: its netlist struct and how parse reads it, and the outputs, watches
//! and samples its package pins give the circuit around `orbe_microchip::Bridge`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::active::{Chip, Supply};
use crate::diagnostic::{Diagnostic, FirmwareRule, ValueRule};
use crate::mna::Node;
use crate::name::{Name, Ref};
use crate::netlist::IntValue;
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Rating;
use crate::netlist::Threshold;
use crate::netlist::Value;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::WireThreshold;
use crate::netlist::json::threshold::resolve_threshold;
use crate::netlist::json::{
    PinMap, WireInt, WireRating, WireValue, resolve_int, resolve_limit, resolve_rating,
    resolve_some_pins, resolve_value,
};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};
use crate::ratings::{Ceiling, PinRatings, PinWindow};
use crate::watch::{Signal, Watch};
use orbe_microchip::{
    Bridge, FuseBytes, HexRule, PACKAGE, PORT_PINS, Pad, Port, parse_hex, port_index,
};

/// The supply at which the chip's own conductance draws `active_current` or `reset_current`,
/// volt.
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
/// VCC and to `GND_8` through its leakage, which every drive state stamps. The crystal pins,
/// `AREF`, `ADC6` and `ADC7` join nothing. Neither does `AVCC`: every stage draws from VCC, and
/// `AVCC` is read only at a sample. Parse puts `GND_22` on `GND_8`'s net.
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

    // `RSTDISBL` unprogrammed: `PC6` is `RESET`, with its own pull-up and thresholds.
    let reset = fuses.has_reset().then(|| port_index(RESET));
    let pads = super::Pads {
        vcc: mcu.pins[package_pin(Pad::Vcc)],
        gnd: mcu.pins[package_pin(Pad::Gnd)],
        output: &mcu.output,
        leakage: mcu.leakage_resistance.si.get(),
        pads: PORT_PINS
            .iter()
            .enumerate()
            .map(|(n, &k)| {
                let (pull_up, threshold) = if reset == Some(n) {
                    (&mcu.reset_pull_up_resistance, &mcu.reset_threshold)
                } else {
                    (&mcu.pull_up_resistance, &mcu.input_threshold)
                };
                super::Pad {
                    net: mcu.pins[k],
                    pull_up: pull_up.si.get(),
                    pull_down: None,
                    threshold,
                }
            })
            .collect(),
    };
    let (outputs, mut watches) = pads.build();
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

    // ADC0 to ADC5 are `PC0` to `PC5`. A part without `ADC6` or `ADC7`, as the DIP is, reads
    // 0 V there.
    let samples = (0..6)
        .map(|bit| Pad::Port(Port::C, bit))
        .chain([Pad::Adc(6), Pad::Adc(7), Pad::Avcc, Pad::Aref])
        .map(|pad| Signal {
            terms: match mcu.net(pad) {
                Some(net) => vec![(Node::from(net), 1.0), (node(Pad::Agnd), -1.0)],
                None => Vec::new(),
            },
            offset: 0.0,
        })
        .collect();

    let output_pins = PORT_PINS.iter().map(|&k| Atmega328p::PINS[k]).collect();
    let mut chip = super::chip(
        part,
        outputs,
        output_pins,
        watches,
        signals,
        samples,
        bridge,
    );
    let conductance = |current: Option<&Value>| current.map(|c| c.si.get() / ACTIVE_AT);
    let running = conductance(mcu.active_current.as_ref());
    let held = conductance(mcu.reset_current.as_ref()).or(running);
    chip.supply = held.map(|held| Supply {
        vcc: mcu.pins[VCC],
        gnd: mcu.pins[GND_8],
        conductance: running.unwrap_or(0.0),
        held,
    });
    chip
}

/// The part's ratings on its pins, `None` when it has no `ratings` object. Each port pin is an
/// output, in `PORT_PINS` order. Every pin it has but the two ground pins has a window over
/// `GND_8`: `rail_margin` below it, and `rail_margin` above `VCC`, but for `VCC`, whose
/// ceiling is `supply_voltage`, and `PC6` while it is `RESET`, whose ceiling is
/// `reset_voltage` (DS40002061B §29.1).
pub(crate) fn pin_ratings(mcu: &Atmega328p) -> Option<PinRatings> {
    let ratings = mcu.ratings.as_ref()?;
    let si = |value: &Option<Value>| value.as_ref().map(|value| value.si.get());
    let margin = si(&ratings.rail_margin);
    let reset = mcu.fuses.has_reset().then(|| package_pin(RESET));
    let windows = (0..mcu.pins.len())
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
                pin: mcu.names[k],
                net: mcu.pins[k],
                below: margin,
                above,
            })
        })
        .collect();
    Some(PinRatings {
        outputs: PORT_PINS.iter().map(|&k| Atmega328p::PINS[k]).collect(),
        pin_current: ratings.pin_current.clone(),
        supply_current: ratings.supply_current.clone(),
        vcc: (Atmega328p::PINS[VCC], mcu.pins[VCC]),
        gnd: (Atmega328p::PINS[GND_8], mcu.pins[GND_8]),
        windows,
    })
}

/// An ATmega328P running `firmware` from power-on, in its 28-pin DIP or in a package that adds
/// `ADC6` and `ADC7`, as the TQFP does. Microchip DS40002061B.
///
/// Each I/O pin drives through `output` from VCC and into `GND_8`, pulls up through
/// `pull_up_resistance`, or is off, as the firmware sets it, leaks to VCC and to `GND_8` through
/// `leakage_resistance`, and reads its level through `input_threshold`. `PC6` is `RESET` while
/// `RSTDISBL` is unprogrammed, with its own pull-up and thresholds. `PB6` and `PB7` hold the
/// crystal or resonator the fuses select, and `clock_hz` is its frequency. Only parse builds
/// one, so `GND_8` and `GND_22` name one net.
#[derive(Clone, Debug, PartialEq)]
pub struct Atmega328p {
    /// The net of each pin the part has, in `PINS` order: the DIP's 28, then `ADC6` and `ADC7`
    /// where it has them.
    pub pins: Vec<NetId>,
    /// The name of each of `pins`.
    pub names: Vec<&'static str>,
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
    /// Ampere, greater than 0. The core's own draw at `VCC` = 5 V while it runs, a conductance
    /// from `VCC` to `GND_8`. `None` when the part leaves it out, and the chip then draws only
    /// what its pins pass while it runs.
    pub active_current: Option<Value>,
    /// Ampere, greater than 0. The core's own draw at `VCC` = 5 V while a reset, from `RESET`
    /// or brown-out, holds it, a conductance from `VCC` to `GND_8` in place of
    /// `active_current`'s. `None` when the part leaves it out, and the chip then draws
    /// `active_current` while held too.
    pub reset_current: Option<Value>,
    /// The absolute maximum ratings the run checks. `None` when the part has no `ratings`
    /// object, which leaves the chip unrated.
    pub ratings: Option<Atmega328pRatings>,
}

/// An ATmega328P's absolute maximum ratings. Each is `None` when the part leaves it unrated.
#[derive(Clone, Debug, PartialEq)]
pub struct Atmega328pRatings {
    /// Ampere. The current through any one port pin.
    pub pin_current: Option<Rating>,
    /// Ampere. The current through `VCC`, and the current through the ground pins.
    pub supply_current: Option<Rating>,
    /// Volt. How far a pin may go below `GND_8`, and every pin but `VCC` and `RESET` above
    /// `VCC`.
    pub rail_margin: Option<Value>,
    /// Volt. The most `PC6` may sit above `GND_8` while it is `RESET`.
    pub reset_voltage: Option<Value>,
    /// Volt. The most `VCC` may sit above `GND_8`.
    pub supply_voltage: Option<Value>,
}

impl Atmega328p {
    /// DIP order, DS40002061B Figure 1-1, then `ADC6` and `ADC7`, which the TQFP has and the DIP
    /// does not (§1.1.9). The two ground pins carry their pin numbers.
    pub const PINS: [&'static str; 30] = [
        "PC6", "PD0", "PD1", "PD2", "PD3", "PD4", "VCC", "GND_8", "PB6", "PB7", "PD5", "PD6",
        "PD7", "PB0", "PB1", "PB2", "PB3", "PB4", "PB5", "AVCC", "AREF", "GND_22", "PC0", "PC1",
        "PC2", "PC3", "PC4", "PC5", "ADC6", "ADC7",
    ];
    /// The DIP's pins, which every part wires. A part may leave `ADC6` and `ADC7` out.
    pub(crate) const REQUIRED: usize = 28;
    /// The flash, 32 KB, DS40002061B Table 2-1.
    pub const FLASH_BYTES: usize = orbe_microchip::FLASH_BYTES;

    /// The net of `pad`'s pin, `None` for `ADC6` or `ADC7` when the part leaves it out.
    fn net(&self, pad: Pad) -> Option<NetId> {
        let name = Atmega328p::PINS[package_pin(pad)];
        let k = self.names.iter().position(|&wired| wired == name)?;
        Some(self.pins[k])
    }
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
    /// Optional, so a netlist written before the chip drew less in reset keeps its meaning.
    #[serde(default)]
    reset_current: Option<WireValue>,
    /// Optional, so a netlist written before the chip had ratings keeps its meaning.
    #[serde(default)]
    ratings: Option<WireRatings>,
}

/// Every key is required, and `null` leaves that rating out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    pin_current: Option<WireRating>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_current: Option<WireRating>,
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
    let pins = resolve_some_pins(
        name,
        &Atmega328p::PINS,
        Atmega328p::REQUIRED,
        &wire.pins.0,
        nets,
        out,
    )
    .and_then(|(pins, names)| one_ground(name, pins, nets, out).map(|pins| (pins, names)));
    let firmware = match parse_hex(&wire.firmware) {
        Ok(flash) => Some(Firmware { flash }),
        Err(fault) => {
            out.push(Diagnostic::NetlistFirmware {
                part: part_ref.clone(),
                line: fault.line,
                rule: firmware_rule(fault.rule),
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
    let reset_current = match wire.reset_current {
        None => Some(None),
        Some(value) => resolve_value(part_ref, "reset_current", "A", true, value, out).map(Some),
    };
    let ratings = match wire.ratings {
        None => Some(None),
        Some(wire) => resolve_ratings(part_ref, wire, out).map(Some),
    };
    let (pins, names) = pins?;
    Some(Atmega328p {
        pins,
        names,
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
        reset_current: reset_current?,
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
    let rail_margin = resolve_limit(part, "ratings.rail_margin", "V", wire.rail_margin, out);
    let reset_voltage = resolve_limit(part, "ratings.reset_voltage", "V", wire.reset_voltage, out);
    let supply_voltage = resolve_limit(
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
    pins: Vec<NetId>,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Vec<NetId>> {
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

/// The rule a firmware breaks for each rule the chip's HEX reader names.
fn firmware_rule(rule: HexRule) -> FirmwareRule {
    match rule {
        HexRule::Record => FirmwareRule::Record,
        HexRule::Checksum => FirmwareRule::Checksum,
        HexRule::RecordType => FirmwareRule::RecordType,
        HexRule::AfterEnd => FirmwareRule::AfterEnd,
        HexRule::NoEnd => FirmwareRule::NoEnd,
        HexRule::Empty => FirmwareRule::Empty,
        HexRule::PastFlash => FirmwareRule::PastFlash,
        HexRule::Overlap => FirmwareRule::Overlap,
    }
}
