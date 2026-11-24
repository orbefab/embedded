//! The `ssd1306` OLED controller part. [`model`] is the device: the command decoder, the
//! registers and the GDDRAM. [`glass`] draws what the panel shows. This file is its netlist
//! struct, its JSON wire type, its row in the device table, and the electrical side: `SDA` is
//! open drain, and `SCL` and `RES` are inputs the part does not hold. The host's I2C engine runs
//! the bus at its `address`.

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
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Threshold;
use crate::netlist::Value;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, bounded, resolve_pins, resolve_value};
use crate::watch::{Signal, Watch};

pub use glass::glass;
pub use model::{HEIGHT, WIDTH};

const VDD: usize = 0;
const VSS: usize = 1;
const SCL: usize = 2;
const SDA: usize = 3;
const RES: usize = 4;

/// The 7-bit addresses `SA0` selects, `0111100` and `0111101` (datasheet 8.1.5.1).
const ADDRESSES: [u8; 2] = [0x3c, 0x3d];

/// Table 12-1, logic supply minimum, volt. The comparator is high above `offset`, so the
/// bound sits 1 µV under the inclusive end the sheet names. There is no upper bound: over its
/// ratings the controller keeps running, as the real part mostly does.
const VDD_MIN_V: f64 = 1.65;
const BOUND_INSIDE_V: f64 = 1e-6;

/// No events. A panel takes its commands from the bus.
pub(crate) const ROW: Row = Row::new(&[]);

/// The pin pairs the chip joins at DC, for `net.floating`: SDA can pull to `VSS`.
pub(crate) const DC_PATHS: [(usize, usize); 1] = [(SDA, VSS)];

pub(crate) fn chip(part: PartId, dev: &Ssd1306) -> Chip {
    let pins = dev.pins;
    let low = Stage {
        drop: dev.low_drop.si.get(),
        resistance: dev.low_resistance.si.get(),
    };
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
    let supply = Watch::comparator(Signal {
        terms: vec![(Node::from(pins[VDD]), 1.0), (Node::from(pins[VSS]), -1.0)],
        offset: VDD_MIN_V - BOUND_INSIDE_V,
    });
    let mut signals = [
        (Ssd1306::PINS[SCL], model::WATCH_SCL),
        (Ssd1306::PINS[SDA], model::WATCH_SDA),
        (Ssd1306::PINS[RES], model::WATCH_RES),
    ];
    signals.sort_unstable();
    let bus = Bus {
        address: dev.address,
        scl: model::WATCH_SCL,
        sda: model::WATCH_SDA,
        reset: Some(model::WATCH_RES),
        address_high: None,
        power: &[],
    };
    Chip {
        part: Some(part),
        outputs: vec![Output {
            pin: pins[SDA],
            vcc: pins[VDD],
            gnd: pins[VSS],
            high: low,
            low,
            pull_up: None,
            pull_down: None,
            leakage: None,
        }],
        watches: vec![watch(SCL), watch(SDA), watch(RES), supply],
        signals: signals.to_vec(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: dev.draw.as_ref().map(|_| (pins[VDD], pins[VSS])),
        takes: crate::active::Takes::default(),
        core: Core::Device(Box::new(Native::new(
            device(dev.draw.as_ref()),
            Engine::new(bus),
            model::OUTPUTS,
        ))),
    }
}

/// The device, which takes its draw as whole nanoamperes and millivolts.
fn device(draw: Option<&PanelDraw>) -> model::Ssd1306 {
    let whole =
        |value: &Value, scale: f64| FieldValue::Int((value.si.get() * scale).round() as i64);
    let fields: Vec<(&str, FieldValue)> = draw.map_or_else(Vec::new, |draw| {
        vec![
            ("sleep_na", whole(&draw.sleep_current, 1e9)),
            ("logic_na", whole(&draw.logic_current, 1e9)),
            ("half_lit_na", whole(&draw.half_lit_current, 1e9)),
            ("all_lit_na", whole(&draw.all_lit_current, 1e9)),
            ("holds_above_mv", whole(&draw.holds_above, 1e3)),
        ]
    });
    model::Ssd1306::new(&Fields::new(&fields)).expect("parse bounds the draw")
}

/// An SSD1306 128 by 64 OLED controller on I2C, an active chip with no clock of its own. It is
/// an I2C target at `address`, 0x3C or 0x3D. `SDA` is open drain into `VSS` through
/// `low_resistance` above `low_drop`. `SCL` and `RES` are inputs, and the part holds neither.
/// `RES` low resets it. It reads each of those three pins through `input_threshold`, fractions
/// of its supply. It answers the bus and lights the glass while `V(VDD) - V(VSS)` is at least
/// 1.65 V, at any supply above that. Below it the bus sees no ACK and the glass is dark. With
/// `draw` it draws from `VDD` into `VSS` as [`PanelDraw`] says, and with none it draws nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Ssd1306 {
    pub pins: [NetId; 5],
    pub address: u8,
    /// Volt, at least 0. The open-drain low side.
    pub low_drop: Value,
    /// Ohm.
    pub low_resistance: Value,
    pub input_threshold: Threshold,
    /// What the panel draws from `VDD` into `VSS`. `None` draws nothing.
    pub draw: Option<PanelDraw>,
}

/// The panel's draw from `VDD` into `VSS`, its logic and its charge pump together, as the
/// modules tie `VBAT` to `VDD`. With the display off, `RES#` low or `VDD` under its logic
/// minimum it is `sleep_current`. With the display on it is `logic_current`, and while the
/// charge pump is on it adds the pump's current, on the line through `half_lit_current` with
/// half the glass lit and `all_lit_current` with all of it, at no less than 0. The draw holds
/// while `VDD` is at or above `holds_above`, and below that fades to none at 0 V, as
/// `orbe_model::Draw` says.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelDraw {
    /// Ampere, each at least 0.
    pub sleep_current: Value,
    pub logic_current: Value,
    pub half_lit_current: Value,
    pub all_lit_current: Value,
    /// Volt, greater than 0.
    pub holds_above: Value,
}

impl Ssd1306 {
    pub const PINS: [&'static str; 5] = ["VDD", "VSS", "SCL", "SDA", "RES"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSsd1306 {
    pub(crate) name: String,
    pins: PinMap,
    address: u8,
    low_drop: WireValue,
    low_resistance: WireValue,
    input_threshold: WireThreshold,
    #[serde(default)]
    draw: Option<WireDraw>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDraw {
    sleep_current: WireValue,
    logic_current: WireValue,
    half_lit_current: WireValue,
    all_lit_current: WireValue,
    holds_above: WireValue,
}

pub(crate) fn resolve_ssd1306(ctx: ParseCtx<'_>, wire: WireSsd1306) -> Option<Ssd1306> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Ssd1306::PINS, &wire.pins.0, nets, out);
    let address = if ADDRESSES.contains(&wire.address) {
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
        let sleep_current = current("draw.sleep_current", draw.sleep_current, out);
        let logic_current = current("draw.logic_current", draw.logic_current, out);
        let half_lit_current = current("draw.half_lit_current", draw.half_lit_current, out);
        let all_lit_current = current("draw.all_lit_current", draw.all_lit_current, out);
        let holds_above = resolve_value(
            part_ref,
            "draw.holds_above",
            "V",
            true,
            draw.holds_above,
            out,
        );
        Some(PanelDraw {
            sleep_current: sleep_current?,
            logic_current: logic_current?,
            half_lit_current: half_lit_current?,
            all_lit_current: all_lit_current?,
            holds_above: holds_above?,
        })
    });
    let draw = match draw {
        Some(draw) => Some(draw?),
        None => None,
    };
    Some(Ssd1306 {
        pins: pins?,
        address: address?,
        low_drop: low_drop?,
        low_resistance: low_resistance?,
        input_threshold: input_threshold?,
        draw,
    })
}
