//! A `ws2812` chain behind the seam. It watches `DIN` and takes `Pulses` on that net. The
//! device in [`model`] keeps the bits and the colours, and shows the colours. The pulse engine
//! times the line.

use serde::Deserialize;

use orbe_model::{Device, Fields, Value};

use crate::active::native::{Hosted, Native};
use crate::active::pulse::Pulse;
use crate::active::{Chip, Core, Takes};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, resolve_pins};
use crate::netlist::{NetId, PartId, Threshold};
use crate::watch::{Signal, Watch};

pub(crate) mod model;

#[cfg(test)]
mod tests;

const VCC: usize = 0;
const GND: usize = 1;
const DIN: usize = 2;

/// The pin pairs it joins at DC. It joins none: `DIN` draws no current.
pub(crate) const DC_PATHS: [(usize, usize); 0] = [];

/// No events. A chain takes its bits from the line.
pub(crate) const ROW: crate::device::Row = crate::device::Row::new(&[]);

pub(crate) fn chip(part: PartId, chain: &Ws2812) -> Chip {
    let pins = chain.pins;
    let against = |fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[DIN]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let threshold = &chain.input_threshold;
    Chip {
        part: Some(part),
        outputs: Vec::new(),
        watches: vec![Watch {
            rise: against(threshold.high.si.get()),
            fall: against(threshold.low.si.get()),
        }],
        signals: vec![(Ws2812::PINS[DIN], 0)],
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        takes: Takes {
            spi: None,
            duty: None,
            pulses: Some(pins[DIN]),
        },
        core: Core::Device(core(chain.leds)),
    }
}

/// The device sized to the chain: the smallest of 16, 64, 256 and 1024 LEDs that holds it. A
/// landing copies the state, so a short chain copies little.
fn core(leds: u16) -> Box<dyn Hosted> {
    match leds {
        0..=16 => sized::<16>(leds),
        17..=64 => sized::<64>(leds),
        65..=256 => sized::<256>(leds),
        _ => sized::<{ Ws2812::LED_MAX as usize }>(leds),
    }
}

fn sized<const N: usize>(leds: u16) -> Box<dyn Hosted> {
    let device = model::Ws2812::<N>::new(&Fields::new(&[("leds", Value::Int(i64::from(leds)))]))
        .expect("the state holds the chain");
    Box::new(Native::new(device, Pulse::new(), 0))
}

/// A chain of WS2812E-1313 LEDs, an active chip with no clock of its own. It reads `DIN`
/// through `input_threshold`, fractions of its supply, and takes `Pulses` records on that net.
/// Each LED keeps the first 24 bits, green then red then blue, and passes the rest on. A low
/// longer than 280 µs latches. `leds` is from 1 to [`Ws2812::LED_MAX`]. Only parse builds one.
#[derive(Clone, Debug, PartialEq)]
pub struct Ws2812 {
    pub pins: [NetId; 3],
    pub leds: u16,
    pub input_threshold: Threshold,
}

impl Ws2812 {
    pub const PINS: [&'static str; 3] = ["VCC", "GND", "DIN"];
    /// The most LEDs one part holds.
    pub const LED_MAX: u16 = model::LED_MAX;
    /// The count a part omits.
    pub const LEDS_DEFAULT: u16 = 13;
}

fn default_leds() -> u16 {
    Ws2812::LEDS_DEFAULT
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireWs2812 {
    pub(crate) name: String,
    pins: PinMap,
    #[serde(default = "default_leds")]
    leds: u16,
    input_threshold: WireThreshold,
}

pub(crate) fn resolve_ws2812(ctx: ParseCtx<'_>, wire: WireWs2812) -> Option<Ws2812> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Ws2812::PINS, &wire.pins.0, nets, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let leds = if (1..=Ws2812::LED_MAX).contains(&wire.leds) {
        Some(wire.leds)
    } else {
        out.push(Diagnostic::NetlistValue {
            part: part_ref.clone(),
            field: "leds".into(),
            value: f64::from(wire.leds),
            rule: ValueRule::Leds,
        });
        None
    };
    Some(Ws2812 {
        pins: pins?,
        leds: leds?,
        input_threshold: input_threshold?,
    })
}
