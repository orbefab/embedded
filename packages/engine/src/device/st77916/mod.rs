//! The `st77916` panel controller part. [`model`] is the device: the command decoder, the
//! registers and the frame memory. This file is its netlist struct, its JSON wire type, its row
//! in the device table, and the electrical side: it reads `RST` through a watch and drives `TE`.
//! The host's SPI engine takes the `Spi` records on its `CS` and `SCLK` nets, and its PWM pin the
//! `Duty` records on `BLK`.

use serde::Deserialize;

use orbe_model::{Device, Fields};

use crate::active::native::Native;
use crate::active::{Chip, Core, Output, SpiTarget, Stage, Takes, duty, spi};
use crate::mna::Node;
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Threshold;
use crate::netlist::Value;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, resolve_limit, resolve_pins};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};
use crate::ratings::{PinRatings, SupplyMin};
use crate::watch::{Signal, Watch};

mod glass;
pub(crate) mod model;
#[cfg(test)]
mod tests;

pub use glass::glass;
pub use model::{HEIGHT, WIDTH};

const VCC: usize = 0;
const GND: usize = 1;
const CS: usize = 2;
const SCLK: usize = 3;
const D0: usize = 4;
const RST: usize = 8;
const TE: usize = 9;
const BLK: usize = 10;

/// The pin pairs the chip joins at DC, for `net.floating`: TE to `VCC` and to `GND`.
pub(crate) const DC_PATHS: [(usize, usize); 2] = [(TE, VCC), (TE, GND)];

/// No events. A panel takes its commands from the bus.
pub(crate) const ROW: crate::device::Row = crate::device::Row::new(&[]);

/// The host engines of its SPI bus and of its `BLK` pin.
pub(crate) type Port = (spi::Engine, duty::Pin);

pub(crate) fn chip(part: PartId, panel: &St77916) -> Chip {
    let pins = panel.pins;
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let output = Output {
        pin: pins[TE],
        vcc: pins[VCC],
        gnd: pins[GND],
        high: stage(&panel.output.high_drop, &panel.output.high_resistance),
        low: stage(&panel.output.low_drop, &panel.output.low_resistance),
        pull_up: None,
        strong_pull_up: None,
        pull_down: None,
        leakage: None,
    };
    // V(RST) - f * V(VCC) - (1 - f) * V(GND): RST against a fraction of the chip's supply.
    let against = |fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[RST]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let threshold = &panel.input_threshold;
    Chip {
        part: Some(part),
        outputs: vec![output],
        watches: vec![Watch {
            rise: against(threshold.high.si.get()),
            fall: against(threshold.low.si.get()),
        }],
        signals: vec![(St77916::PINS[RST], model::WATCH_RST)],
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: None,
        takes: Takes {
            spi: Some(SpiTarget {
                cs: pins[CS],
                sclk: pins[SCLK],
                pins: Some([CS, SCLK, D0, D0 + 1, D0 + 2, D0 + 3]),
            }),
            duty: Some(pins[BLK]),
            pulses: None,
            pulses_pin: None,
        },
        core: Core::Device(Box::new(Native::new(
            model::St77916::new(&Fields::NONE).expect("an st77916 takes no params"),
            port(&pins),
            model::OUTPUTS,
        ))),
    }
}

fn port(pins: &[NetId; 11]) -> Port {
    let bus = spi::Bus {
        data: [D0, D0 + 1, D0 + 2, D0 + 3].map(|k| pins[k]),
        reset: Some(model::WATCH_RST),
    };
    (spi::Engine::new(bus), duty::Pin)
}

/// The part's minimum operating supply voltage on `VCC` over `GND`, `None` when unrated.
pub(crate) fn pin_ratings(panel: &St77916) -> Option<PinRatings> {
    let min = panel.ratings.as_ref()?.supply_min.as_ref()?.si.get();
    Some(PinRatings {
        outputs: Vec::new(),
        pin_current: None,
        supply_current: None,
        vcc: (St77916::PINS[VCC], panel.pins[VCC]),
        gnd: (St77916::PINS[GND], panel.pins[GND]),
        windows: Vec::new(),
        supply_mins: vec![SupplyMin {
            pin: St77916::PINS[VCC],
            net: panel.pins[VCC],
            gnd: panel.pins[GND],
            min,
        }],
    })
}

/// A 360 by 360 QSPI panel controller, an active chip with no clock of its own. It takes the
/// `Spi` records on its `CS` and `SCLK` nets and the `Duty` records on `BLK`. It reads `RST`
/// through `input_threshold`, fractions of its supply, and drives `TE` low through `output`
/// from `VCC` and into `GND` for the whole run.
#[derive(Clone, Debug, PartialEq)]
pub struct St77916 {
    pub pins: [NetId; 11],
    pub output: OutputStage,
    pub input_threshold: Threshold,
    pub ratings: Option<St77916Ratings>,
}

/// An ST77916's ratings.
#[derive(Clone, Debug, PartialEq)]
pub struct St77916Ratings {
    /// Volt. The least operating `VCC` may sit above `GND`.
    pub supply_min: Option<Value>,
}

impl St77916 {
    pub const PINS: [&'static str; 11] = [
        "VCC", "GND", "CS", "SCLK", "D0", "D1", "D2", "D3", "RST", "TE", "BLK",
    ];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSt77916 {
    pub(crate) name: String,
    pins: PinMap,
    output: WireOutputStage,
    input_threshold: WireThreshold,
    #[serde(default)]
    ratings: Option<WireRatings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(default)]
    supply_min: Option<WireValue>,
}

pub(crate) fn resolve_st77916(ctx: ParseCtx<'_>, wire: WireSt77916) -> Option<St77916> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, St77916::PINS, &wire.pins.0, nets, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let ratings = match wire.ratings {
        None => Some(None),
        Some(wire) => resolve_limit(part_ref, "ratings.supply_min", "V", wire.supply_min, out)
            .map(|supply_min| Some(St77916Ratings { supply_min })),
    };
    Some(St77916 {
        pins: pins?,
        output: output?,
        input_threshold: input_threshold?,
        ratings: ratings?,
    })
}
