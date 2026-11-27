//! The `sg90` micro servo. [`model`] is the shaft and the supply current. `SIGNAL` is a
//! high-impedance input the host's pulse engine times. The draw runs from `VCC` to `GND`.

pub(crate) mod model;

use orbe_model::{Device, Fields};

use crate::active::native::Native;
use crate::active::pulse::Pulse;
use crate::active::{Chip, Core, Takes};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, resolve_limit, resolve_pins};
use crate::netlist::{NetId, PartId, Threshold, Value};
use crate::ratings::{PinRatings, SupplyMin};
use crate::watch::{Signal, Watch};

const VCC: usize = 0;
const GND: usize = 1;
const SIGNAL: usize = 2;

/// The pin pair it joins at DC: the supply draw from `VCC` to `GND`. `SIGNAL` draws nothing.
pub(crate) const DC_PATHS: [(usize, usize); 1] = [(VCC, GND)];

/// No events. The shaft follows the pulses on `SIGNAL`.
pub(crate) const ROW: crate::device::Row = crate::device::Row::new(&[]);

pub(crate) fn chip(part: PartId, dev: &Sg90) -> Chip {
    let pins = dev.pins;
    let against = |fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[SIGNAL]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let device = model::Sg90::new(&Fields::NONE).expect("an sg90 takes no params");
    Chip {
        part: Some(part),
        outputs: Vec::new(),
        watches: vec![Watch {
            rise: against(dev.input_threshold.high.si.get()),
            fall: against(dev.input_threshold.low.si.get()),
        }],
        signals: vec![(Sg90::PINS[SIGNAL], 0)],
        samples: vec![Signal {
            terms: vec![(Node::from(pins[VCC]), 1.0), (Node::from(pins[GND]), -1.0)],
            offset: 0.0,
        }],
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: Some((pins[VCC], pins[GND])),
        takes: Takes {
            spi: None,
            duty: None,
            pulses: Some(pins[SIGNAL]),
            pulses_pin: Some(SIGNAL),
        },
        core: Core::Device(Box::new(Native::new(device, Pulse::new(), 0))),
    }
}

/// The part's minimum operating supply voltage on `VCC` over `GND`, `None` when unrated.
pub(crate) fn pin_ratings(dev: &Sg90) -> Option<PinRatings> {
    let min = dev.ratings.as_ref()?.supply_min.as_ref()?.si.get();
    Some(PinRatings {
        outputs: Vec::new(),
        pin_current: None,
        supply_current: None,
        vcc: (Sg90::PINS[VCC], dev.pins[VCC]),
        gnd: (Sg90::PINS[GND], dev.pins[GND]),
        windows: Vec::new(),
        supply_mins: vec![SupplyMin {
            pin: Sg90::PINS[VCC],
            net: dev.pins[VCC],
            gnd: dev.pins[GND],
            min,
            clock_hz: None,
        }],
    })
}

/// An SG90 micro servo. `SIGNAL` is read through `input_threshold`, fractions of the supply.
/// The shaft angle, the speed and the supply current are the device's.
#[derive(Clone, Debug, PartialEq)]
pub struct Sg90 {
    pub pins: [NetId; 3],
    pub input_threshold: Threshold,
    pub ratings: Option<Sg90Ratings>,
}

/// An SG90's ratings.
#[derive(Clone, Debug, PartialEq)]
pub struct Sg90Ratings {
    /// Volt. The least operating `VCC` may sit above `GND`.
    pub supply_min: Option<Value>,
}

impl Sg90 {
    pub const PINS: [&'static str; 3] = ["VCC", "GND", "SIGNAL"];
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSg90 {
    pub(crate) name: String,
    pins: PinMap,
    input_threshold: WireThreshold,
    #[serde(default)]
    ratings: Option<WireRatings>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(default)]
    supply_min: Option<WireValue>,
}

pub(crate) fn resolve_sg90(ctx: ParseCtx<'_>, wire: WireSg90) -> Option<Sg90> {
    let pins = resolve_pins(ctx.name, Sg90::PINS, &wire.pins.0, ctx.nets, ctx.out);
    let input_threshold = resolve_threshold(
        ctx.part,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        ctx.out,
    );
    let ratings = match wire.ratings {
        None => Some(None),
        Some(wire) => resolve_limit(
            ctx.part,
            "ratings.supply_min",
            "V",
            wire.supply_min,
            ctx.out,
        )
        .map(|supply_min| Some(Sg90Ratings { supply_min })),
    };
    Some(Sg90 {
        pins: pins?,
        input_threshold: input_threshold?,
        ratings: ratings?,
    })
}
