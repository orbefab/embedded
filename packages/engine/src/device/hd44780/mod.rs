//! The HD44780U character LCD controller (Hitachi ADE-207-272) and the `hd44780` kind, a 16×2
//! module on the controller's own parallel bus.
//!
//! [`model`] is the parallel device. Its [`controller`](model::controller) is the HD44780U, and
//! its [`backpack`](model::backpack) the PCF8574 front end the `hd44780_i2c` kind runs on that
//! same controller. [`glass`] draws either kind's 16×2 glass with the A00 5×8 ROM [`font`].
//! This file holds the `hd44780` netlist struct, its JSON wire type, and its electrical side:
//! `DB0..DB7` push-pull from `VDD` and into `VSS` during a read, the MOS pull-ups on `DB0..DB7`,
//! `RS` and `R/W`, and the watches on the bus, the supply, `V0`, the backlight and the module's
//! operating voltage.

pub mod font;
pub(crate) mod glass;
pub(crate) mod model;
#[cfg(test)]
mod tests;

use serde::Deserialize;

use orbe_model::{Device, Fields, Value as FieldValue};

use super::Row;
use crate::active::native::{Native, Quiet};
use crate::active::{Chip, Core, Output, Stage};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{
    PinMap, WireValue, bounded, resolve_limit, resolve_pins, resolve_value,
};
use crate::netlist::{NetId, PartId, Threshold, Value};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage_at};
use crate::ratings::{Ceiling, PinRatings, PinWindow};
use crate::watch::{Signal, Watch};

pub use glass::{backlight_on, glass, text_visible, visible_lines};
pub use model::controller::{HEIGHT, WIDTH};

const VSS: usize = 0;
const VDD: usize = 1;
const V0: usize = 2;
const RS: usize = 3;
const RW: usize = 4;
const E: usize = 5;
const DB0: usize = 6;
const A: usize = 14;
const K: usize = 15;

/// `RS`, `R/W`, `E` and `DB0..DB7`, the order of [`model::Hd44780`]'s first watches.
const BUS_PINS: [usize; 11] = [RS, RW, E, DB0, 7, 8, 9, 10, 11, 12, 13];
/// `DB0..DB7`, `RS` and `R/W`, the order [`model::Hd44780`] drives.
const OUTPUT_PINS: [usize; model::OUTPUTS] = [DB0, 7, 8, 9, 10, 11, 12, 13, RS, RW];

/// Hitachi ADE-207-272 Fig. 23/24: 15 ms power-on reset wait threshold, 4.5 V. It also splits the
/// bus timing tables, p. 49 below it and p. 52 from it.
const VDD_FAST_V: f64 = 4.5;
const BOUND_INSIDE_V: f64 = 1e-6;

/// No events. The pins are the inputs.
pub(crate) const ROW: Row = Row::new(&[]);

/// DC paths for `net.floating`: each pull-up to `VDD`, and each data line into `VSS`.
pub(crate) const DC_PATHS: [(usize, usize); 18] = [
    (DB0, VDD),
    (DB0, VSS),
    (7, VDD),
    (7, VSS),
    (8, VDD),
    (8, VSS),
    (9, VDD),
    (9, VSS),
    (10, VDD),
    (10, VSS),
    (11, VDD),
    (11, VSS),
    (12, VDD),
    (12, VSS),
    (13, VDD),
    (13, VSS),
    (RS, VDD),
    (RW, VDD),
];

pub(crate) fn chip(part: PartId, dev: &Hd44780) -> Chip {
    let pins = dev.pins;
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let outputs = OUTPUT_PINS
        .iter()
        .map(|&pin| Output {
            pin: pins[pin],
            vcc: pins[VDD],
            gnd: pins[VSS],
            high: stage(&dev.output.high_drop, &dev.output.high_resistance),
            low: stage(&dev.output.low_drop, &dev.output.low_resistance),
            pull_up: Some(dev.pull_up.si.get()),
            strong_pull_up: None,
            pull_down: None,
            leakage: None,
        })
        .collect();
    let against = |pin: usize, fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[pin]), 1.0),
            (Node::from(pins[VDD]), -fraction),
            (Node::from(pins[VSS]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let watch = |pin: usize| Watch {
        rise: against(pin, dev.input_threshold.high.si.get()),
        fall: against(pin, dev.input_threshold.low.si.get()),
    };
    let across = |plus: usize, minus: usize, volts: f64| {
        Watch::comparator(Signal {
            terms: vec![
                (Node::from(pins[plus]), 1.0),
                (Node::from(pins[minus]), -1.0),
            ],
            offset: volts - BOUND_INSIDE_V,
        })
    };
    let mut watches: Vec<Watch> = BUS_PINS.into_iter().map(watch).collect();
    let powered = |volts: f64| Signal {
        terms: vec![(Node::from(pins[VDD]), 1.0), (Node::from(pins[VSS]), -1.0)],
        offset: volts - BOUND_INSIDE_V,
    };
    watches.push(Watch {
        rise: powered(model::MIN_VDD_V),
        fall: powered(model::RELEASE_VDD_V),
    });
    watches.push(across(VDD, VSS, VDD_FAST_V));
    watches.push(across(VDD, V0, dev.lcd_voltage.si.get()));
    watches.push(across(A, K, dev.backlight_voltage.si.get()));
    watches.push(across(VDD, VSS, dev.operating_voltage.si.get()));
    let mut signals: Vec<(&'static str, usize)> = BUS_PINS
        .into_iter()
        .enumerate()
        .map(|(index, pin)| (Hd44780::PINS[pin], index))
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
        draw: Some((pins[VDD], pins[VSS])),
        takes: crate::active::Takes::default(),
        core: Core::Device(Box::new(Native::new(
            device(&dev.draw),
            Quiet,
            model::OUTPUTS,
        ))),
    }
}

fn device(draw: &LcdDraw) -> model::Hd44780 {
    let whole =
        |value: &Value, scale: f64| FieldValue::Int((value.si.get() * scale).round() as i64);
    let fields = [
        ("logic_na", whole(&draw.logic_current, 1e9)),
        ("holds_above_mv", whole(&draw.holds_above, 1e3)),
    ];
    model::Hd44780::new(&Fields::new(&fields)).expect("parse bounds the draw")
}

/// The part's voltage ratings on `VDD` and its logic pins, `None` when it has no `ratings`
/// object. Every window is over `VSS`: `VDD` takes `-rail_margin` to `supply_voltage`, and `RS`,
/// `R/W`, `E` and `DB0..DB7` take `-rail_margin` to `VDD + rail_margin`.
pub(crate) fn pin_ratings(dev: &Hd44780) -> Option<PinRatings> {
    let ratings = dev.ratings.as_ref()?;
    let si = |value: &Option<Value>| value.as_ref().map(|value| value.si.get());
    let margin = si(&ratings.rail_margin);
    let supply = si(&ratings.supply_voltage);
    let mut windows = Vec::new();
    if margin.is_some() || supply.is_some() {
        windows.push(PinWindow {
            pin: Hd44780::PINS[VDD],
            net: dev.pins[VDD],
            below: margin,
            above: supply.map(Ceiling::Fixed),
        });
    }
    if let Some(margin) = margin {
        for pin in BUS_PINS {
            windows.push(PinWindow {
                pin: Hd44780::PINS[pin],
                net: dev.pins[pin],
                below: Some(margin),
                above: Some(Ceiling::OverSupply(margin)),
            });
        }
    }
    Some(PinRatings {
        outputs: Vec::new(),
        pin_current: None,
        supply_current: None,
        vcc: (Hd44780::PINS[VDD], dev.pins[VDD]),
        gnd: (Hd44780::PINS[VSS], dev.pins[VSS]),
        windows,
        // `operating_voltage` already warns `low_supply` on a bus cycle under the module's
        // minimum, so the part carries no `supply_min`.
        supply_mins: Vec::new(),
    })
}

/// An HD44780U 16×2 module on its parallel bus. `DB0..DB7` drive a read through `output` and
/// hold `pull_up` otherwise, as `RS` and `R/W` do. The bus reads through `input_threshold`,
/// fractions of the supply. The glass shows text while `VDD - V0` is at `lcd_voltage` and the
/// backlight's `A - K` at `backlight_voltage`. A bus cycle with `VDD - VSS` under
/// `operating_voltage`, the module's minimum, warns.
#[derive(Clone, Debug, PartialEq)]
pub struct Hd44780 {
    pub pins: [NetId; 16],
    pub output: OutputStage,
    pub pull_up: Value,
    pub input_threshold: Threshold,
    pub lcd_voltage: Value,
    pub backlight_voltage: Value,
    pub operating_voltage: Value,
    pub draw: LcdDraw,
    /// The pin voltage ratings the run checks. `None` when the part has no `ratings` object.
    pub ratings: Option<Hd44780Ratings>,
}

/// An HD44780 module's voltage ratings. Each is `None` when the part leaves it unrated.
#[derive(Clone, Debug, PartialEq)]
pub struct Hd44780Ratings {
    /// Volt. How far `VDD` and the logic pins may sit below `VSS`, and the logic pins above
    /// `VDD`.
    pub rail_margin: Option<Value>,
    /// Volt. The most `VDD` may sit above `VSS`.
    pub supply_voltage: Option<Value>,
}

/// The controller's draw from `VDD` while powered.
#[derive(Clone, Debug, PartialEq)]
pub struct LcdDraw {
    pub logic_current: Value,
    pub holds_above: Value,
}

impl Hd44780 {
    /// The module's 16-pin header order.
    pub const PINS: [&'static str; 16] = [
        "VSS", "VDD", "V0", "RS", "RW", "E", "D0", "D1", "D2", "D3", "D4", "D5", "D6", "D7", "A",
        "K",
    ];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireHd44780 {
    pub(crate) name: String,
    pins: PinMap,
    output: WireOutputStage,
    pull_up: WireValue,
    input_threshold: WireThreshold,
    lcd_voltage: WireValue,
    backlight_voltage: WireValue,
    operating_voltage: WireValue,
    draw: WireLcdDraw,
    #[serde(default)]
    ratings: Option<WireRatings>,
}

/// Every key is required, and `null` leaves that rating out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    rail_margin: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_voltage: Option<WireValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLcdDraw {
    logic_current: WireValue,
    holds_above: WireValue,
}

pub(crate) fn resolve_hd44780(ctx: ParseCtx<'_>, wire: WireHd44780) -> Option<Hd44780> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Hd44780::PINS, &wire.pins.0, nets, out);
    let output = resolve_output_stage_at(part_ref, "output", wire.output, out);
    let pull_up = resolve_value(part_ref, "pull_up", "ohm", true, wire.pull_up, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let lcd_voltage = resolve_value(part_ref, "lcd_voltage", "V", true, wire.lcd_voltage, out);
    let backlight_voltage = resolve_value(
        part_ref,
        "backlight_voltage",
        "V",
        true,
        wire.backlight_voltage,
        out,
    );
    let operating_voltage = resolve_value(
        part_ref,
        "operating_voltage",
        "V",
        true,
        wire.operating_voltage,
        out,
    );
    let logic_current = bounded(
        part_ref,
        "draw.logic_current",
        "A",
        wire.draw.logic_current,
        |number| (number < 0.0).then_some(ValueRule::NonNegative),
        out,
    );
    let holds_above = resolve_value(
        part_ref,
        "draw.holds_above",
        "V",
        true,
        wire.draw.holds_above,
        out,
    );
    let ratings = match wire.ratings {
        None => Some(None),
        Some(wire) => resolve_ratings(part_ref, wire, out).map(Some),
    };
    Some(Hd44780 {
        pins: pins?,
        output: output?,
        pull_up: pull_up?,
        input_threshold: input_threshold?,
        lcd_voltage: lcd_voltage?,
        backlight_voltage: backlight_voltage?,
        operating_voltage: operating_voltage?,
        draw: LcdDraw {
            logic_current: logic_current?,
            holds_above: holds_above?,
        },
        ratings: ratings?,
    })
}

fn resolve_ratings(
    part: &crate::name::Ref,
    wire: WireRatings,
    out: &mut Vec<Diagnostic>,
) -> Option<Hd44780Ratings> {
    let rail_margin = resolve_limit(part, "ratings.rail_margin", "V", wire.rail_margin, out);
    let supply_voltage = resolve_limit(
        part,
        "ratings.supply_voltage",
        "V",
        wire.supply_voltage,
        out,
    );
    Some(Hd44780Ratings {
        rail_margin: rail_margin?,
        supply_voltage: supply_voltage?,
    })
}
