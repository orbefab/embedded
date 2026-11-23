//! The `cst816` touch controller part. [`model`] is the device: the register file, `INT` and
//! `RST`. This file is its netlist struct, its JSON wire type, its row in the device table, and
//! the electrical side: `SDA` and `INT` are open drain, and `SCL` is an input the part does not
//! hold. The host's I2C engine runs the bus at address 0x15.

mod model;
#[cfg(test)]
mod tests;

use serde::Deserialize;

use orbe_model::{Device, Fields, Value as FieldValue};

use super::{EventSpec, Events, FieldSpec, Row, resolve_events};
use crate::active::i2c::{Bus, Engine};
use crate::active::native::Native;
use crate::active::{Chip, Core, Output, Stage};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::mna::Node;
use crate::name::Ref;
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Threshold;
use crate::netlist::Value;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, resolve_pins, resolve_value};
use crate::watch::{Signal, Watch};

const VCC: usize = 0;
const GND: usize = 1;
const SCL: usize = 2;
const SDA: usize = 3;
const INT_PIN: usize = 4;
const RST: usize = 5;

/// 7-bit address. A write byte is 0x2A and a read byte is 0x2B.
const ADDRESS: u8 = 0x15;

/// The bus the I2C engine runs for the part. `RST` low holds it in reset.
const BUS: Bus = Bus {
    address: ADDRESS,
    scl: model::WATCH_SCL,
    sda: model::WATCH_SDA,
    reset: Some(model::WATCH_RST),
};

/// The largest `x` or `y` of a touch.
const COORD_MAX: i64 = 4095;

/// A finger: down at `(x, y)`, or up.
const TOUCH: EventSpec = EventSpec {
    name: "touch",
    field: "touches",
    fields: &[
        ("x", FieldSpec::Int { max: COORD_MAX }),
        ("y", FieldSpec::Int { max: COORD_MAX }),
        ("down", FieldSpec::Bool),
    ],
};

pub(crate) const ROW: Row = Row::new(&[TOUCH]);

/// The pin pairs the chip joins at DC, for `net.floating`: SDA and INT can each pull to GND.
pub(crate) const DC_PATHS: [(usize, usize); 2] = [(SDA, GND), (INT_PIN, GND)];

pub(crate) fn chip(part: PartId, dev: &Cst816) -> Chip {
    let pins = dev.pins;
    let low = Stage {
        drop: dev.low_drop.si.get(),
        resistance: dev.low_resistance.si.get(),
    };
    let output = |pin| Output {
        pin,
        vcc: pins[VCC],
        gnd: pins[GND],
        high: low,
        low,
        pull_up: None,
        pull_down: None,
        leakage: None,
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
    let mut signals = [
        (Cst816::PINS[SCL], model::WATCH_SCL),
        (Cst816::PINS[SDA], model::WATCH_SDA),
        (Cst816::PINS[INT_PIN], model::WATCH_INT),
        (Cst816::PINS[RST], model::WATCH_RST),
    ];
    signals.sort_unstable();
    Chip {
        part: Some(part),
        outputs: vec![output(pins[SDA]), output(pins[INT_PIN])],
        watches: vec![watch(SCL), watch(SDA), watch(INT_PIN), watch(RST)],
        signals: signals.to_vec(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        pixels: None,
        takes: crate::active::Takes::default(),
        core: Core::Device(Box::new(Native::new(
            model::Cst816::new(&Fields::NONE).expect("a cst816 takes no params"),
            Engine::new(BUS),
            model::OUTPUTS,
        ))),
    }
}

/// A CST816 touch controller, an active chip with no clock of its own. It is an I2C target at
/// address 0x15. `SDA` and `INT` are open drain into `GND` through `low_resistance` above
/// `low_drop`. `SCL` is an input, and the part does not hold it. `RST` low clears it. It reads
/// each of those four pins through `input_threshold`, fractions of its supply.
///
/// `events` are the finger's touches. Each is one point, after t = 0 and at least
/// [`MIN_GAP_PS`](crate::netlist::MIN_GAP_PS) after the previous one. `x` and `y` run from 0 to
/// 4095.
#[derive(Clone, Debug, PartialEq)]
pub struct Cst816 {
    pub pins: [NetId; 6],
    /// Volt, at least 0. The open-drain low side.
    pub low_drop: Value,
    /// Ohm.
    pub low_resistance: Value,
    pub input_threshold: Threshold,
    pub events: Events,
}

impl Cst816 {
    pub const PINS: [&'static str; 6] = ["VCC", "GND", "SCL", "SDA", "INT", "RST"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireCst816 {
    pub(crate) name: String,
    pins: PinMap,
    low_drop: WireValue,
    low_resistance: WireValue,
    input_threshold: WireThreshold,
    #[serde(default)]
    touches: Vec<WireTouch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTouch {
    t_ps: u64,
    x: u16,
    y: u16,
    down: bool,
}

pub(crate) fn resolve_cst816(ctx: ParseCtx<'_>, wire: WireCst816) -> Option<Cst816> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Cst816::PINS, &wire.pins.0, nets, out);
    let low_drop = resolve_drop(part_ref, wire.low_drop, out);
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
    let touches = wire
        .touches
        .into_iter()
        .map(|touch| {
            let fields = vec![
                ("x".to_string(), FieldValue::Int(touch.x.into())),
                ("y".to_string(), FieldValue::Int(touch.y.into())),
                ("down".to_string(), FieldValue::Bool(touch.down)),
            ];
            (touch.t_ps, fields)
        })
        .collect();
    let events = resolve_events(part_ref, &TOUCH, touches, out);
    Some(Cst816 {
        pins: pins?,
        low_drop: low_drop?,
        low_resistance: low_resistance?,
        input_threshold: input_threshold?,
        events: events?,
    })
}

fn resolve_drop(
    part: &Ref,
    wire: WireValue,
    out: &mut Vec<Diagnostic>,
) -> Option<crate::netlist::Value> {
    let volts = wire.value.get();
    let value = resolve_value(part, "low_drop", "V", false, wire, out)?;
    if volts < 0.0 {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "low_drop".into(),
            value: volts,
            rule: ValueRule::NonNegative,
        });
        return None;
    }
    Some(value)
}
