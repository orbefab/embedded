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

use orbe_model::{Device, Fields};

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
use crate::netlist::json::{PinMap, WireValue, resolve_pins, resolve_value};
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
        watches: vec![watch(SCL), watch(SDA), watch(RES)],
        signals: signals.to_vec(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: None,
        takes: crate::active::Takes::default(),
        core: Core::Device(Box::new(Native::new(
            model::Ssd1306::new(&Fields::NONE).expect("an ssd1306 takes no params"),
            Engine::new(bus),
            model::OUTPUTS,
        ))),
    }
}

/// An SSD1306 128 by 64 OLED controller on I2C, an active chip with no clock of its own. It is
/// an I2C target at `address`, 0x3C or 0x3D. `SDA` is open drain into `VSS` through
/// `low_resistance` above `low_drop`. `SCL` and `RES` are inputs, and the part holds neither.
/// `RES` low resets it. It reads each of those three pins through `input_threshold`, fractions
/// of its supply.
#[derive(Clone, Debug, PartialEq)]
pub struct Ssd1306 {
    pub pins: [NetId; 5],
    pub address: u8,
    /// Volt, at least 0. The open-drain low side.
    pub low_drop: Value,
    /// Ohm.
    pub low_resistance: Value,
    pub input_threshold: Threshold,
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
    Some(Ssd1306 {
        pins: pins?,
        address: address?,
        low_drop: low_drop?,
        low_resistance: low_resistance?,
        input_threshold: input_threshold?,
    })
}
