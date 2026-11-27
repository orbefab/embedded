//! The `hc595` shift register. [`model`] is the device: the shift, the latch,
//! the clear and `OE`. This file is its netlist struct, its JSON wire type, its
//! row in the device table, and the electrical side. `QA` to `QH` and `QHp`
//! are push-pull from `VCC` and into `GND`.

pub(crate) mod model;
#[cfg(test)]
mod tests;

use serde::Deserialize;

use orbe_model::Device;

use super::Row;
use crate::active::native::{Hosting, Native, Port};
use crate::active::{Chip, Core, Output, Stage};
use crate::diagnostic::Diagnostic;
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{
    PinMap, WireRating, WireValue, resolve_limit, resolve_pins, resolve_rating,
};
use crate::netlist::{NetId, PartId, Rating, Threshold, Value};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage_at};
use crate::ratings::{Ceiling, PinRatings, PinWindow, SupplyMin};
use crate::watch::{Signal, Watch};
use crate::world::World;

use orbe_model::{Drive, Time};

const QB: usize = 0;
const QC: usize = 1;
const QD: usize = 2;
const QE: usize = 3;
const QF: usize = 4;
const QG: usize = 5;
const QH: usize = 6;
const GND: usize = 7;
const QHP: usize = 8;
const SRCLR: usize = 9;
const SRCLK: usize = 10;
const RCLK: usize = 11;
const OE: usize = 12;
const SER: usize = 13;
const QA: usize = 14;
const VCC: usize = 15;

/// `QA` to `QH`, then `QHp`, the order [`model::Hc595`] drives.
const OUTPUT_PINS: [usize; model::OUTPUTS] = [QA, QB, QC, QD, QE, QF, QG, QH, QHP];

/// Each output can drive either rail, including while `OE` holds `QA`–`QH` off.
pub(crate) const DC_PATHS: [(usize, usize); 18] = [
    (QA, VCC),
    (QA, GND),
    (QB, VCC),
    (QB, GND),
    (QC, VCC),
    (QC, GND),
    (QD, VCC),
    (QD, GND),
    (QE, VCC),
    (QE, GND),
    (QF, VCC),
    (QF, GND),
    (QG, VCC),
    (QG, GND),
    (QH, VCC),
    (QH, GND),
    (QHP, VCC),
    (QHP, GND),
];

/// Margin inside the strict supply comparator so an exact 2.0 V rail reads high and does not
/// chatter on 1-ULP solver rounding.
const BOUND_INSIDE_V: f64 = 1e-6;

/// No events. The pins are the inputs.
pub(crate) const ROW: Row = Row::new(&[]);

/// A port with no pins. The device reads its watches itself.
#[derive(Clone, Copy, Debug)]
struct Quiet;

impl<D: Device> Port<D> for Quiet {
    const PINS: usize = 0;

    fn drive(&self, _image: &mut [Drive]) {}

    fn due_at(&self) -> Option<Time> {
        None
    }

    fn take_due(&mut self, _dev: &mut Hosting<D>, _world: &World, _at: Time) -> bool {
        false
    }

    fn input(&mut self, _dev: &mut Hosting<D>, _world: &World, _at: Time, _levels: &[bool]) {}
}

pub(crate) fn chip(part: PartId, dev: &Hc595) -> Chip {
    let pins = dev.pins;
    let stage = |drop: &crate::netlist::Value, resistance: &crate::netlist::Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let output = |index: usize, kind: &OutputStage| Output {
        pin: pins[index],
        vcc: pins[VCC],
        gnd: pins[GND],
        high: stage(&kind.high_drop, &kind.high_resistance),
        low: stage(&kind.low_drop, &kind.low_resistance),
        pull_up: None,
        strong_pull_up: None,
        pull_down: None,
        leakage: None,
    };
    let mut outputs = Vec::with_capacity(model::OUTPUTS);
    for (n, &index) in OUTPUT_PINS.iter().enumerate() {
        let kind = if n == model::QHP {
            &dev.serial_output
        } else {
            &dev.output
        };
        outputs.push(output(index, kind));
    }
    let against = |pin: usize, fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[pin]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let watch = |pin| Watch {
        rise: against(pin, dev.input_threshold.high.si.get()),
        fall: against(pin, dev.input_threshold.low.si.get()),
    };
    let watched = [SER, SRCLK, RCLK, SRCLR, OE];
    // High once the supply reaches the recommended minimum, inset by 1 uV with 1 uV hysteresis
    // so an exact 2.0 V rail reads high and does not chatter on 1-ULP solver rounding.
    let supply = |offset| Signal {
        terms: vec![(Node::from(pins[VCC]), 1.0), (Node::from(pins[GND]), -1.0)],
        offset,
    };
    let mut watches: Vec<Watch> = watched.into_iter().map(watch).collect();
    watches.push(Watch {
        rise: supply(model::MIN_VCC_V - BOUND_INSIDE_V),
        fall: supply(model::MIN_VCC_V - 2.0 * BOUND_INSIDE_V),
    });
    let mut signals: Vec<(&'static str, usize)> = watched
        .into_iter()
        .enumerate()
        .map(|(index, pin)| (Hc595::PINS[pin], index))
        .collect();
    signals.sort_unstable();
    Chip {
        part: Some(part),
        outputs,
        watches,
        signals,
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: None,
        takes: crate::active::Takes::default(),
        core: Core::Device(Box::new(Native::new(
            model::Hc595::new(&orbe_model::Fields::NONE).expect("an hc595 takes no params"),
            Quiet,
            model::OUTPUTS,
        ))),
    }
}

/// The part's ratings on `QA`–`QH`, `QHp`, `VCC` and `GND`, and the voltage window on
/// each input and output when the part rates one.
pub(crate) fn pin_ratings(part: &Hc595) -> Option<PinRatings> {
    let mut windows = Vec::new();
    let mut add = |pins: &[usize], margin: Option<&Value>| {
        let Some(margin) = margin else {
            return;
        };
        let margin = margin.si.get();
        for &pin in pins {
            windows.push(PinWindow {
                pin: Hc595::PINS[pin],
                net: part.pins[pin],
                below: Some(margin),
                above: Some(Ceiling::OverSupply(margin)),
            });
        }
    };
    add(
        &[SRCLR, SRCLK, RCLK, OE, SER],
        part.ratings.input_margin.as_ref(),
    );
    add(&OUTPUT_PINS, part.ratings.output_margin.as_ref());
    let supply_mins = part
        .ratings
        .supply_min
        .as_ref()
        .map(|min| SupplyMin {
            pin: Hc595::PINS[VCC],
            net: part.pins[VCC],
            gnd: part.pins[GND],
            min: min.si.get(),
            clock_hz: None,
        })
        .into_iter()
        .collect();
    Some(PinRatings {
        outputs: OUTPUT_PINS.iter().map(|&pin| Hc595::PINS[pin]).collect(),
        pin_current: part.ratings.pin_current.clone(),
        supply_current: part.ratings.supply_current.clone(),
        vcc: (Hc595::PINS[VCC], part.pins[VCC]),
        gnd: (Hc595::PINS[GND], part.pins[GND]),
        windows,
        supply_mins,
    })
}

/// A 74HC595 8-bit shift register. `QA` to `QH` drive the storage register
/// through `output`, and `QHp` drives the shift register's last stage through
/// `serial_output`. The inputs read through `input_threshold`, fractions of
/// the supply. `ratings` bounds each output's current, the current through
/// `VCC` and `GND`, and how far an input or an output may sit past either rail.
#[derive(Clone, Debug, PartialEq)]
pub struct Hc595 {
    pub pins: [NetId; 16],
    pub output: OutputStage,
    pub serial_output: OutputStage,
    pub input_threshold: Threshold,
    pub ratings: Hc595Ratings,
}

/// Absolute maximum ratings. Each is `None` when the part leaves it unrated.
#[derive(Clone, Debug, PartialEq)]
pub struct Hc595Ratings {
    /// Ampere. The current through any one of `QA`–`QH` and `QHp`.
    pub pin_current: Option<Rating>,
    /// Ampere. The current through `VCC`, and the current through `GND`.
    pub supply_current: Option<Rating>,
    /// Volt. How far `SER`, `SRCLK`, `RCLK`, `SRCLR` and `OE` may sit below `GND`
    /// and above `VCC`.
    pub input_margin: Option<Value>,
    /// Volt. How far `QA`–`QH` and `QHp` may sit below `GND` and above `VCC`.
    pub output_margin: Option<Value>,
    /// Volt. The least operating `VCC` may sit above `GND`.
    pub supply_min: Option<Value>,
}

impl Hc595 {
    /// DIP-16 order, TI SCLS041J table 5-1. `QHp` is the sheet's QH'.
    pub const PINS: [&'static str; 16] = [
        "QB", "QC", "QD", "QE", "QF", "QG", "QH", "GND", "QHp", "SRCLR", "SRCLK", "RCLK", "OE",
        "SER", "QA", "VCC",
    ];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireHc595 {
    pub(crate) name: String,
    pins: PinMap,
    output: WireOutputStage,
    serial_output: WireOutputStage,
    input_threshold: WireThreshold,
    ratings: WireRatings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    pin_current: Option<WireRating>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_current: Option<WireRating>,
    #[serde(deserialize_with = "Option::deserialize")]
    input_margin: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    output_margin: Option<WireValue>,
    #[serde(default)]
    supply_min: Option<WireValue>,
}

pub(crate) fn resolve_hc595(ctx: ParseCtx<'_>, wire: WireHc595) -> Option<Hc595> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Hc595::PINS, &wire.pins.0, nets, out);
    let output = resolve_output_stage_at(part_ref, "output", wire.output, out);
    let serial_output = resolve_output_stage_at(part_ref, "serial_output", wire.serial_output, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let ratings = resolve_ratings(part_ref, wire.ratings, out);
    Some(Hc595 {
        pins: pins?,
        output: output?,
        serial_output: serial_output?,
        input_threshold: input_threshold?,
        ratings: ratings?,
    })
}

fn resolve_ratings(
    part: &crate::name::Ref,
    wire: WireRatings,
    out: &mut Vec<Diagnostic>,
) -> Option<Hc595Ratings> {
    let pin_current = resolve_rating(part, "ratings.pin_current", "A", wire.pin_current, out);
    let supply_current = resolve_rating(
        part,
        "ratings.supply_current",
        "A",
        wire.supply_current,
        out,
    );
    let input_margin = resolve_limit(part, "ratings.input_margin", "V", wire.input_margin, out);
    let output_margin = resolve_limit(part, "ratings.output_margin", "V", wire.output_margin, out);
    let supply_min = resolve_limit(part, "ratings.supply_min", "V", wire.supply_min, out);
    Some(Hc595Ratings {
        pin_current: pin_current?,
        supply_current: supply_current?,
        input_margin: input_margin?,
        output_margin: output_margin?,
        supply_min: supply_min?,
    })
}
