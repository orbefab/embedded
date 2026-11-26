//! The `bmp280` temperature and pressure sensor part. [`model`] is the device: the register
//! file and the conversion. This file is its netlist struct, its JSON wire type, and the
//! electrical side. `SDI` is open drain. `SCK`, `CSB` and `SDO` are inputs. The host's I2C
//! engine runs the bus. SDO low is address 0x76 and SDO high is 0x77. CSB low selects SPI,
//! and the engine does not answer on I2C while it is low.

pub(crate) mod model;
#[cfg(test)]
mod tests;

use std::borrow::Cow;

use serde::Deserialize;

use orbe_model::{Device, Fields};

use super::Row;
use crate::active::i2c::{Bus, Engine};
use crate::active::native::Native;
use crate::active::{Chip, Core, Output, Stage};
use crate::diagnostic::{Diagnostic, WorldRule};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, resolve_limit, resolve_pins, resolve_value};
use crate::netlist::{NetId, PartId, Threshold, Value};
use crate::ratings::{Ceiling, PinRatings, PinWindow, SupplyMin};
use crate::watch::{Signal, Watch};

const VDD: usize = 0;
const VDDIO: usize = 1;
const GND: usize = 2;
const CSB: usize = 3;
const SDI: usize = 4;
const SCK: usize = 5;
const SDO: usize = 6;

/// SDO selects the address bit. CSB low holds the bus out of I2C, which is SPI mode. A supply
/// below its minimum holds it too, so the part releases SDA the moment it loses power.
const BUS: Bus = Bus {
    address: model::ADDRESS,
    scl: model::WATCH_SCK,
    sda: model::WATCH_SDI,
    reset: Some(model::WATCH_CSB),
    address_high: Some(model::WATCH_SDO),
    power: &[model::WATCH_VDD_MIN, model::WATCH_VDDIO_MIN],
};

/// `SDI` can pull to `GND`.
pub(crate) const DC_PATHS: [(usize, usize); 1] = [(SDI, GND)];

/// No events. The sensor reads the world.
pub(crate) const ROW: Row = Row::new(&[]);

pub(crate) fn chip(part: PartId, dev: &Bmp280) -> Chip {
    let pins = dev.pins;
    let low = Stage {
        drop: 0.0,
        resistance: dev.low_resistance.si.get(),
    };
    let against = |pin: usize, fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[pin]), 1.0),
            (Node::from(pins[VDDIO]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let watch = |pin: usize| Watch {
        rise: against(pin, dev.input_threshold.high.si.get()),
        fall: against(pin, dev.input_threshold.low.si.get()),
    };
    let mut signals = [
        (Bmp280::PINS[SCK], model::WATCH_SCK),
        (Bmp280::PINS[SDI], model::WATCH_SDI),
        (Bmp280::PINS[CSB], model::WATCH_CSB),
        (Bmp280::PINS[SDO], model::WATCH_SDO),
    ];
    signals.sort_unstable();
    // At or above `limit` over GND, the minimum included by a microvolt. The comparator is true
    // strictly above 0.
    let above = |pin: usize, limit: f64| {
        Watch::comparator(Signal {
            terms: vec![(Node::from(pins[pin]), 1.0), (Node::from(pins[GND]), -1.0)],
            offset: limit - 1e-6,
        })
    };
    let watches = vec![
        watch(SCK),
        watch(SDI),
        watch(CSB),
        watch(SDO),
        above(VDD, model::VDD_MIN_V),
        above(VDDIO, model::VDDIO_MIN_V),
    ];
    Chip {
        part: Some(part),
        outputs: vec![Output {
            pin: pins[SDI],
            vcc: pins[VDDIO],
            gnd: pins[GND],
            high: low,
            low,
            pull_up: None,
            pull_down: None,
            strong_pull_up: None,
            leakage: None,
        }],
        watches,
        signals: signals.to_vec(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: Some((pins[VDD], pins[GND])),
        takes: crate::active::Takes::default(),
        core: Core::Device(Box::new(Native::new(
            model::Bmp280::new(&Fields::NONE).expect("a bmp280 takes no params"),
            Engine::new(BUS),
            model::OUTPUTS,
        ))),
    }
}

/// A BMP280 temperature and pressure sensor, an active chip with no clock of its own. It is an
/// I2C target at 0x76 when `SDO` is low and at 0x77 when `SDO` is high. `SDI` is open drain
/// into `GND` through `low_resistance`. `SCK`, `CSB` and `SDO` are inputs. It reads each of
/// those four through `input_threshold`, fractions of `VDDIO`. `CSB` low is SPI mode, and the
/// part does not answer on I2C then. It answers while `VDD` and `VDDIO` are at or above their
/// Table 2 minimums, past the sheet's maximum too, where the ratings warn. Below them it is off
/// and draws nothing. It reads the netlist's `temperature` and `pressure`. `VDD` draws the
/// Table 2 typical current.
#[derive(Clone, Debug, PartialEq)]
pub struct Bmp280 {
    pub pins: [NetId; 7],
    /// Ohm.
    pub low_resistance: Value,
    pub input_threshold: Threshold,
    pub ratings: Bmp280Ratings,
}

/// Table 3 and Table 2. `supply_voltage` is the ceiling on `VDD` and `VDDIO` over `GND`.
/// `pin_margin` is how far a pin may sit below `GND`, and an interface pin above `VDDIO`.
/// `supply_min` and `io_supply_min` are the Table 2 minimums on `VDD` and `VDDIO`.
#[derive(Clone, Debug, PartialEq)]
pub struct Bmp280Ratings {
    /// Volt, greater than 0.
    pub supply_voltage: Value,
    /// Volt, greater than 0.
    pub pin_margin: Value,
    /// Volt, greater than 0. Minimum operating `VDD` over `GND`.
    pub supply_min: Option<Value>,
    /// Volt, greater than 0. Minimum operating `VDDIO` over `GND`.
    pub io_supply_min: Option<Value>,
}

/// The part's Table 3 windows and Table 2 supply minimums. `VDD` and `VDDIO` use
/// `supply_voltage`. `CSB`, `SDI`, `SCK` and `SDO` use `pin_margin` over `VDDIO`. Every pin but
/// `GND` also uses `pin_margin` below `GND`.
pub(crate) fn pin_ratings(dev: &Bmp280) -> Option<PinRatings> {
    let margin = dev.ratings.pin_margin.si.get();
    let supply = dev.ratings.supply_voltage.si.get();
    let window = |pin: usize, above: Ceiling| PinWindow {
        pin: Bmp280::PINS[pin],
        net: dev.pins[pin],
        below: Some(margin),
        above: Some(above),
    };
    let mut windows = vec![
        window(VDD, Ceiling::Fixed(supply)),
        window(VDDIO, Ceiling::Fixed(supply)),
    ];
    for pin in [CSB, SDI, SCK, SDO] {
        windows.push(window(pin, Ceiling::OverSupply(margin)));
    }
    let mut supply_mins = Vec::new();
    if let Some(min) = &dev.ratings.supply_min {
        supply_mins.push(SupplyMin {
            pin: Bmp280::PINS[VDD],
            net: dev.pins[VDD],
            gnd: dev.pins[GND],
            min: min.si.get(),
            clock_hz: None,
        });
    }
    if let Some(min) = &dev.ratings.io_supply_min {
        supply_mins.push(SupplyMin {
            pin: Bmp280::PINS[VDDIO],
            net: dev.pins[VDDIO],
            gnd: dev.pins[GND],
            min: min.si.get(),
            clock_hz: None,
        });
    }
    Some(PinRatings {
        outputs: vec![Bmp280::PINS[SDI]],
        pin_current: None,
        supply_current: None,
        vcc: (Bmp280::PINS[VDDIO], dev.pins[VDDIO]),
        gnd: (Bmp280::PINS[GND], dev.pins[GND]),
        windows,
        supply_mins,
    })
}

impl Bmp280 {
    pub const PINS: [&'static str; 7] = ["VDD", "VDDIO", "GND", "CSB", "SDI", "SCK", "SDO"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireBmp280 {
    pub(crate) name: String,
    pins: PinMap,
    low_resistance: WireValue,
    input_threshold: WireThreshold,
    ratings: WireRatings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    supply_voltage: WireValue,
    pin_margin: WireValue,
    #[serde(default)]
    supply_min: Option<WireValue>,
    #[serde(default)]
    io_supply_min: Option<WireValue>,
}

pub(crate) fn resolve_bmp280(ctx: ParseCtx<'_>, wire: WireBmp280) -> Option<Bmp280> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        world,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Bmp280::PINS, &wire.pins.0, nets, out);
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
    let supply_voltage = resolve_value(
        part_ref,
        "ratings.supply_voltage",
        "V",
        true,
        wire.ratings.supply_voltage,
        out,
    );
    let pin_margin = resolve_value(
        part_ref,
        "ratings.pin_margin",
        "V",
        true,
        wire.ratings.pin_margin,
        out,
    );
    let supply_min = resolve_limit(
        part_ref,
        "ratings.supply_min",
        "V",
        wire.ratings.supply_min,
        out,
    );
    let io_supply_min = resolve_limit(
        part_ref,
        "ratings.io_supply_min",
        "V",
        wire.ratings.io_supply_min,
        out,
    );
    let mut declared = true;
    for &quantity in model::Bmp280::WORLD {
        if world.schedule(quantity).is_none() {
            declared = false;
            out.push(Diagnostic::NetlistWorld {
                part: Some(part_ref.clone()),
                field: Cow::Borrowed("kind"),
                name: quantity.name().into(),
                rule: WorldRule::Undeclared,
            });
        }
    }
    let bmp280 = Bmp280 {
        pins: pins?,
        low_resistance: low_resistance?,
        input_threshold: input_threshold?,
        ratings: Bmp280Ratings {
            supply_voltage: supply_voltage?,
            pin_margin: pin_margin?,
            supply_min: supply_min?,
            io_supply_min: io_supply_min?,
        },
    };
    declared.then_some(bmp280)
}
