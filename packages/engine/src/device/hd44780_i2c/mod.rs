//! The `hd44780_i2c` 16×2 character LCD with PCF8574 I2C backpack.
//!
//! [`model`] implements the PCF8574 8-bit quasi-bidirectional expander driving an HD44780U
//! controller (Hitachi ADE-207-272). [`glass`] extracts the visible 16×2 text and draws the blue
//! backlit glass with the A00 5×8 ROM font and CGRAM custom characters. This file holds the
//! netlist struct, its JSON wire type, and its electrical interface (`SDA` open-drain, `SCL` input,
//! `VCC`/`GND` supply watch and optional current draw).

pub mod font;
mod glass;
pub(crate) mod model;
#[cfg(test)]
mod tests;

use serde::Deserialize;

use orbe_model::{Device, Fields, Value as FieldValue};

use super::Row;
use crate::active::i2c::{Bus, Engine};
use crate::active::native::Native;
use crate::active::{Chip, Core, Output, Stage};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, bounded, resolve_pins, resolve_value};
use crate::netlist::{NetId, PartId, Threshold, Value};
use crate::watch::{Signal, Watch};

pub use glass::{backlight_on, glass, text_visible, visible_lines};
pub use model::{HEIGHT, WIDTH};

const VCC: usize = 0;
const GND: usize = 1;
const SCL: usize = 2;
const SDA: usize = 3;

/// The 7-bit PCF8574 addresses A0..A2 select: 0x20 to 0x27 (NXP PCF8574 Table 4).
const ADDRESS_MIN: u8 = 0x20;
const ADDRESS_MAX: u8 = 0x27;

/// Hitachi ADE-207-272 Table 6, HD44780U operating voltage minimum, 2.7 V.
const VCC_MIN_V: f64 = 2.7;
/// Hitachi ADE-207-272 Fig. 23/24: 15 ms power-on reset wait threshold, 4.5 V.
const VCC_FAST_V: f64 = 4.5;
const BOUND_INSIDE_V: f64 = 1e-6;

/// No external events; the LCD takes its commands over I2C.
pub(crate) const ROW: Row = Row::new(&[]);

/// DC path for `net.floating`: `SDA` pulls to `GND` through its open-drain stage.
pub(crate) const DC_PATHS: [(usize, usize); 1] = [(SDA, GND)];

pub(crate) fn chip(part: PartId, dev: &Hd44780I2c) -> Chip {
    let pins = dev.pins;
    let low = Stage {
        drop: dev.low_drop.si.get(),
        resistance: dev.low_resistance.si.get(),
    };
    let against = |pin: usize, fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[pin]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let watch = |pin: usize| Watch {
        rise: against(pin, dev.input_threshold.high.si.get()),
        fall: against(pin, dev.input_threshold.low.si.get()),
    };
    let supply = |volts: f64| {
        Watch::comparator(Signal {
            terms: vec![(Node::from(pins[VCC]), 1.0), (Node::from(pins[GND]), -1.0)],
            offset: volts - BOUND_INSIDE_V,
        })
    };
    let mut signals = [
        (Hd44780I2c::PINS[SCL], model::WATCH_SCL),
        (Hd44780I2c::PINS[SDA], model::WATCH_SDA),
    ];
    signals.sort_unstable();
    let bus = Bus {
        address: dev.address,
        scl: model::WATCH_SCL,
        sda: model::WATCH_SDA,
        reset: None,
    };
    Chip {
        part: Some(part),
        outputs: vec![Output {
            pin: pins[SDA],
            vcc: pins[VCC],
            gnd: pins[GND],
            high: low,
            low,
            pull_up: None,
            pull_down: None,
            leakage: None,
        }],
        watches: vec![
            watch(SCL),
            watch(SDA),
            supply(VCC_MIN_V),
            supply(VCC_FAST_V),
        ],
        signals: signals.to_vec(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: dev.draw.as_ref().map(|_| (pins[VCC], pins[GND])),
        takes: crate::active::Takes::default(),
        core: Core::Device(Box::new(Native::new(
            device(dev.backlight_jumper, dev.draw.as_ref()),
            Engine::new(bus),
            model::OUTPUTS,
        ))),
    }
}

fn device(backlight_jumper: bool, draw: Option<&LcdDraw>) -> model::Hd44780I2c {
    let whole =
        |value: &Value, scale: f64| FieldValue::Int((value.si.get() * scale).round() as i64);
    let mut fields: Vec<(&str, FieldValue)> =
        vec![("backlight_jumper", FieldValue::Bool(backlight_jumper))];
    if let Some(draw) = draw {
        fields.push(("logic_na", whole(&draw.logic_current, 1e9)));
        fields.push(("backlight_na", whole(&draw.backlight_current, 1e9)));
        fields.push(("holds_above_mv", whole(&draw.holds_above, 1e3)));
    }
    model::Hd44780I2c::new(&Fields::new(&fields)).expect("parse bounds the draw")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hd44780I2c {
    pub pins: [NetId; 4],
    pub address: u8,
    pub backlight_jumper: bool,
    pub low_drop: Value,
    pub low_resistance: Value,
    pub input_threshold: Threshold,
    pub draw: Option<LcdDraw>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LcdDraw {
    pub logic_current: Value,
    pub backlight_current: Value,
    pub holds_above: Value,
}

impl Hd44780I2c {
    pub const PINS: [&'static str; 4] = ["VCC", "GND", "SCL", "SDA"];
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireHd44780I2c {
    pub(crate) name: String,
    pins: PinMap,
    address: u8,
    #[serde(default = "default_true")]
    backlight_jumper: bool,
    low_drop: WireValue,
    low_resistance: WireValue,
    input_threshold: WireThreshold,
    #[serde(default)]
    draw: Option<WireLcdDraw>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLcdDraw {
    logic_current: WireValue,
    backlight_current: WireValue,
    holds_above: WireValue,
}

pub(crate) fn resolve_hd44780_i2c(ctx: ParseCtx<'_>, wire: WireHd44780I2c) -> Option<Hd44780I2c> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Hd44780I2c::PINS, &wire.pins.0, nets, out);
    let address = if (ADDRESS_MIN..=ADDRESS_MAX).contains(&wire.address) {
        Some(wire.address)
    } else {
        out.push(Diagnostic::NetlistValue {
            part: part_ref.clone(),
            field: "address".into(),
            value: f64::from(wire.address),
            rule: ValueRule::Unsupported,
        });
        None
    };
    let drop = wire.low_drop.value.get();
    let low_drop = resolve_value(part_ref, "low_drop", "V", false, wire.low_drop, out);
    let low_drop = match low_drop {
        Some(_) if drop < 0.0 => {
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: "low_drop".into(),
                value: drop,
                rule: ValueRule::NonNegative,
            });
            None
        }
        value => value,
    };
    let low_resistance = resolve_value(
        part_ref,
        "low_resistance",
        "ohm",
        true,
        wire.low_resistance,
        out,
    );
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let draw = wire.draw.map(|draw| {
        let current = |field, value, out: &mut Vec<Diagnostic>| {
            bounded(
                part_ref,
                field,
                "A",
                value,
                |number| (number < 0.0).then_some(ValueRule::NonNegative),
                out,
            )
        };
        let logic_current = current("draw.logic_current", draw.logic_current, out);
        let backlight_current = current("draw.backlight_current", draw.backlight_current, out);
        let holds_above = resolve_value(
            part_ref,
            "draw.holds_above",
            "V",
            true,
            draw.holds_above,
            out,
        );
        Some(LcdDraw {
            logic_current: logic_current?,
            backlight_current: backlight_current?,
            holds_above: holds_above?,
        })
    });
    let draw = match draw {
        Some(draw) => Some(draw?),
        None => None,
    };
    Some(Hd44780I2c {
        pins: pins?,
        address: address?,
        backlight_jumper: wire.backlight_jumper,
        low_drop: low_drop?,
        low_resistance: low_resistance?,
        input_threshold: input_threshold?,
        draw,
    })
}
