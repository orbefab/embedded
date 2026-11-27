//! The `max7219` 8×8 LED matrix driver part. [`model`] is the device: the 16-bit shift register,
//! `DOUT` chaining, digit/control registers, Code B decoder, intensity/scan-limit PWM, and `ISET`
//! Table 11 current scaling. [`glass`] draws the 8×8 LED matrix. This file is its netlist struct,
//! JSON wire type, device table row, electrical pin stages, and datasheet ratings.

mod glass;
pub(crate) mod model;
#[cfg(test)]
mod tests;

use serde::Deserialize;

use orbe_model::{Device, Drive, Fields, Time, Value as FieldValue};

use super::Row;
use crate::active::native::{Hosting, Native, Port};
use crate::active::{Chip, Core, Output, Stage};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::{
    PinMap, WireRating, WireValue, bounded, resolve_limit, resolve_pins, resolve_rating,
    resolve_value,
};
use crate::netlist::{NetId, PartId, Rating, Threshold, Value};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage_at};
use crate::ratings::{Ceiling, PinRatings, PinWindow};
use crate::watch::{Signal, Watch};
use crate::world::World;

pub use glass::{dot_brightness, glass};
pub use model::{HEIGHT, WIDTH};

const VCC: usize = 0;
const GND: usize = 1;
const DIN: usize = 2;
const CLK: usize = 3;
const LOAD: usize = 4;
const DOUT: usize = 5;
const ISET: usize = 6;

/// `DOUT` drives both rails, `ISET` clamps to `GND`, and the supply draw runs from `VCC` to `GND`.
pub(crate) const DC_PATHS: [(usize, usize); 4] =
    [(DOUT, VCC), (DOUT, GND), (ISET, GND), (VCC, GND)];

/// Margin inside the 4.0 V minimum supply comparator so an exact 4.0 V rail reads high without
/// chattering on 1-ULP solver rounding.
const BOUND_INSIDE_V: f64 = 1e-6;

/// No events. All inputs arrive on pins.
pub(crate) const ROW: Row = Row::new(&[]);

/// A port with no pins. The device reads its watches directly.
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

pub(crate) fn chip(part: PartId, dev: &Max7219) -> Chip {
    let pins = dev.pins;
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let dout_output = Output {
        pin: pins[DOUT],
        vcc: pins[VCC],
        gnd: pins[GND],
        high: stage(&dev.dout.high_drop, &dev.dout.high_resistance),
        low: stage(&dev.dout.low_drop, &dev.dout.low_resistance),
        pull_up: None,
        strong_pull_up: None,
        pull_down: None,
        leakage: None,
    };
    let iset_stage = Stage {
        drop: model::ISET_DROP_V,
        resistance: model::ISET_RESISTANCE_OHM,
    };
    let iset_output = Output {
        pin: pins[ISET],
        vcc: pins[VCC],
        gnd: pins[GND],
        high: iset_stage,
        low: iset_stage,
        pull_up: None,
        strong_pull_up: None,
        pull_down: None,
        leakage: None,
    };
    let above_gnd = |pin: usize, volts: f64| Signal {
        terms: vec![(Node::from(pins[pin]), 1.0), (Node::from(pins[GND]), -1.0)],
        offset: volts,
    };
    let watch = |pin: usize| Watch {
        rise: above_gnd(pin, dev.input_threshold.high.si.get()),
        fall: above_gnd(pin, dev.input_threshold.low.si.get()),
    };
    let supply = |offset: f64| Signal {
        terms: vec![(Node::from(pins[VCC]), 1.0), (Node::from(pins[GND]), -1.0)],
        offset,
    };
    let watched = [DIN, CLK, LOAD];
    let mut watches: Vec<Watch> = watched.into_iter().map(watch).collect();
    watches.push(Watch {
        rise: supply(model::MIN_VCC_V - BOUND_INSIDE_V),
        fall: supply(model::MIN_VCC_V - 2.0 * BOUND_INSIDE_V),
    });
    let mut signals: Vec<(&'static str, usize)> = watched
        .into_iter()
        .enumerate()
        .map(|(index, pin)| (Max7219::PINS[pin], index))
        .collect();
    signals.sort_unstable();
    let samples = vec![
        Signal {
            terms: vec![(Node::from(pins[VCC]), 1.0), (Node::from(pins[GND]), -1.0)],
            offset: 0.0,
        },
        Signal {
            terms: vec![(Node::from(pins[ISET]), 1.0), (Node::from(pins[GND]), -1.0)],
            offset: 0.0,
        },
    ];
    Chip {
        part: Some(part),
        outputs: vec![dout_output, iset_output],
        watches,
        signals,
        samples,
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: dev.draw.as_ref().map(|_| (pins[VCC], pins[GND])),
        takes: crate::active::Takes::default(),
        core: Core::Device(Box::new(Native::new(device(dev), Quiet, model::OUTPUTS))),
    }
}

fn device(dev: &Max7219) -> model::Max7219 {
    let whole =
        |value: &Value, scale: f64| FieldValue::Int((value.si.get() * scale).round() as i64);
    let mut fields = vec![("vled_mv", whole(&dev.led_forward_voltage, 1e3))];
    if let Some(draw) = &dev.draw {
        fields.push(("shutdown_na", whole(&draw.shutdown_current, 1e9)));
        fields.push(("logic_na", whole(&draw.logic_current, 1e9)));
        fields.push(("holds_above_mv", whole(&draw.holds_above, 1e3)));
    }
    model::Max7219::new(&Fields::new(&fields)).expect("parse bounds max7219 params")
}

/// Voltage ratings on all pins (MAX7219 p. 2 Absolute Maximum Ratings) and supply current rating.
pub(crate) fn pin_ratings(dev: &Max7219) -> Option<PinRatings> {
    let ratings = dev.ratings.as_ref()?;
    let si = |value: &Option<Value>| value.as_ref().map(|v| v.si.get());
    let margin = si(&ratings.rail_margin);
    let supply = si(&ratings.supply_voltage);
    let input = si(&ratings.input_voltage);
    let mut windows = Vec::new();
    if margin.is_some() || supply.is_some() {
        windows.push(PinWindow {
            pin: Max7219::PINS[VCC],
            net: dev.pins[VCC],
            below: margin,
            above: supply.map(Ceiling::Fixed),
        });
    }
    if margin.is_some() || input.is_some() {
        for pin in [DIN, CLK, LOAD] {
            windows.push(PinWindow {
                pin: Max7219::PINS[pin],
                net: dev.pins[pin],
                below: margin,
                above: input.map(Ceiling::Fixed),
            });
        }
    }
    if let Some(margin) = margin {
        for pin in [DOUT, ISET] {
            windows.push(PinWindow {
                pin: Max7219::PINS[pin],
                net: dev.pins[pin],
                below: Some(margin),
                above: Some(Ceiling::OverSupply(margin)),
            });
        }
    }
    Some(PinRatings {
        outputs: vec![Max7219::PINS[DOUT], Max7219::PINS[ISET]],
        pin_current: None,
        supply_current: ratings.supply_current.clone(),
        vcc: (Max7219::PINS[VCC], dev.pins[VCC]),
        gnd: (Max7219::PINS[GND], dev.pins[GND]),
        windows,
    })
}

/// A MAX7219 serially interfaced 8×8 LED matrix driver.
#[derive(Clone, Debug, PartialEq)]
pub struct Max7219 {
    pub pins: [NetId; 7],
    /// Fixed-volt input thresholds above `GND` (`VIL = 0.8 V`, `VIH = 3.5 V`, p. 3).
    pub input_threshold: Threshold,
    /// `DOUT` push-pull output stage (`VOH >= V+ - 1 V @ -1 mA`, `VOL <= 0.4 V @ 1.6 mA`, p. 3).
    pub dout: OutputStage,
    /// Matrix LED forward voltage `VLED` (volt), for Table 11 `RSET` to `ISEG` lookup.
    pub led_forward_voltage: Value,
    /// Supply draw parameters (`None` draws nothing).
    pub draw: Option<MatrixDraw>,
    /// Absolute maximum pin voltage and supply current ratings (`None` when unrated).
    pub ratings: Option<Max7219Ratings>,
}

/// MAX7219 supply draw parameters from Electrical Characteristics (p. 2).
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixDraw {
    /// Ampere, shutdown supply current (`150 µA` max).
    pub shutdown_current: Value,
    /// Ampere, operating supply current with `RSET` open (`8 mA` max).
    pub logic_current: Value,
    /// Volt, supply above which draw holds (`4.0 V`).
    pub holds_above: Value,
}

/// MAX7219 absolute maximum ratings (p. 2). Each is `None` when left unrated.
#[derive(Clone, Debug, PartialEq)]
pub struct Max7219Ratings {
    /// Volt, how far any pin may sit below `GND`, and `DOUT`/`ISET` above `VCC` (`0.3 V`).
    pub rail_margin: Option<Value>,
    /// Volt, maximum `VCC` over `GND` (`6 V`).
    pub supply_voltage: Option<Value>,
    /// Volt, maximum `DIN`, `CLK`, `LOAD` over `GND` (`6 V`).
    pub input_voltage: Option<Value>,
    /// Ampere, maximum `VCC`/`GND` current (`500 mA` `DIG0–DIG7` sink current).
    pub supply_current: Option<Rating>,
}

impl Max7219 {
    pub const PINS: [&'static str; 7] = ["VCC", "GND", "DIN", "CLK", "LOAD", "DOUT", "ISET"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireMax7219 {
    pub(crate) name: String,
    pins: PinMap,
    input_threshold: WireVoltageThreshold,
    dout: WireOutputStage,
    led_forward_voltage: WireValue,
    #[serde(default)]
    draw: Option<WireDraw>,
    #[serde(default)]
    ratings: Option<WireRatings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireVoltageThreshold {
    low: WireValue,
    high: WireValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDraw {
    shutdown_current: WireValue,
    logic_current: WireValue,
    holds_above: WireValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    rail_margin: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_voltage: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    input_voltage: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_current: Option<WireRating>,
}

pub(crate) fn resolve_max7219(ctx: ParseCtx<'_>, wire: WireMax7219) -> Option<Max7219> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Max7219::PINS, &wire.pins.0, nets, out);
    let input_threshold = resolve_voltage_threshold(part_ref, wire.input_threshold, out);
    let dout = resolve_output_stage_at(part_ref, "dout", wire.dout, out);
    let led_forward_voltage = resolve_value(
        part_ref,
        "led_forward_voltage",
        "V",
        true,
        wire.led_forward_voltage,
        out,
    );
    let draw = match wire.draw {
        None => Some(None),
        Some(draw) => resolve_draw(part_ref, draw, out).map(Some),
    };
    let ratings = match wire.ratings {
        None => Some(None),
        Some(ratings) => resolve_ratings(part_ref, ratings, out).map(Some),
    };
    Some(Max7219 {
        pins: pins?,
        input_threshold: input_threshold?,
        dout: dout?,
        led_forward_voltage: led_forward_voltage?,
        draw: draw?,
        ratings: ratings?,
    })
}

fn resolve_voltage_threshold(
    part: &crate::name::Ref,
    wire: WireVoltageThreshold,
    out: &mut Vec<Diagnostic>,
) -> Option<Threshold> {
    let low = bounded(
        part,
        "input_threshold.low",
        "V",
        wire.low,
        |v| (v < 0.0).then_some(ValueRule::NonNegative),
        out,
    );
    let high = resolve_value(part, "input_threshold.high", "V", true, wire.high, out);
    let (low, high) = (low?, high?);
    if low.si.get() > high.si.get() {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "input_threshold.low".into(),
            value: low.si.get(),
            rule: ValueRule::AboveHigh,
        });
        return None;
    }
    Some(Threshold { low, high })
}

fn resolve_draw(
    part: &crate::name::Ref,
    wire: WireDraw,
    out: &mut Vec<Diagnostic>,
) -> Option<MatrixDraw> {
    let current = |field, value, out: &mut Vec<Diagnostic>| {
        bounded(
            part,
            field,
            "A",
            value,
            |v| (v < 0.0).then_some(ValueRule::NonNegative),
            out,
        )
    };
    let shutdown_current = current("draw.shutdown_current", wire.shutdown_current, out);
    let logic_current = current("draw.logic_current", wire.logic_current, out);
    let holds_above = resolve_value(part, "draw.holds_above", "V", true, wire.holds_above, out);
    Some(MatrixDraw {
        shutdown_current: shutdown_current?,
        logic_current: logic_current?,
        holds_above: holds_above?,
    })
}

fn resolve_ratings(
    part: &crate::name::Ref,
    wire: WireRatings,
    out: &mut Vec<Diagnostic>,
) -> Option<Max7219Ratings> {
    let rail_margin = resolve_limit(part, "ratings.rail_margin", "V", wire.rail_margin, out);
    let supply_voltage = resolve_limit(
        part,
        "ratings.supply_voltage",
        "V",
        wire.supply_voltage,
        out,
    );
    let input_voltage = resolve_limit(part, "ratings.input_voltage", "V", wire.input_voltage, out);
    let supply_current = resolve_rating(
        part,
        "ratings.supply_current",
        "A",
        wire.supply_current,
        out,
    );
    Some(Max7219Ratings {
        rail_margin: rail_margin?,
        supply_voltage: supply_voltage?,
        input_voltage: input_voltage?,
        supply_current: supply_current?,
    })
}
