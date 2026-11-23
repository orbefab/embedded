//! A `ws2812` chain behind the seam. It watches `DIN` and takes `Pulses` on that net. The
//! device in [`model`] keeps the bits and the colours, shows the colours, and drives `DOUT`
//! when the part wires it. The pulse engine times the line.

use serde::Deserialize;

use orbe_model::{Device, Fields, Value as FieldValue};

use crate::active::native::{Hosted, Native};
use crate::active::pulse::Pulse;
use crate::active::{Chip, Core, Output, PixelDraw, Stage, Takes};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireInt, WireValue, bounded, resolve_int, resolve_pins};
use crate::netlist::{IntValue, NetId, PartId, Threshold, Value};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage_at};
use crate::watch::{Signal, Watch};

pub(crate) mod model;

#[cfg(test)]
mod tests;

const VCC: usize = 0;
const GND: usize = 1;
const DIN: usize = 2;
const DOUT: usize = 3;

/// The pin pairs it joins at DC: `DOUT` to `VCC` and to `GND` when it is wired. `DIN` draws no
/// current.
const DC_PATHS: [(usize, usize); 2] = [(DOUT, VCC), (DOUT, GND)];

pub(crate) fn dc_paths(chain: &Ws2812) -> &'static [(usize, usize)] {
    if chain.dout.is_some() { &DC_PATHS } else { &[] }
}

/// The longest `dout.delay`, picoseconds: T1H's longest high, so the edges still due at once
/// stay few.
pub const DOUT_DELAY_MAX: u64 = 1_000_000;

/// Amperes at a channel value of 255 when the part omits `channel_current`. SK6812MINI-HS
/// section 9 names that part at 12 mA, and the sheet gives one current, so each channel uses it.
pub const CHANNEL_CURRENT: f64 = 0.012;

/// Amperes per LED when the part omits `idle_current`. SK6812MINI-HS section 10, static IDD
/// typical, is 1 mA.
pub const IDLE_CURRENT: f64 = 0.001;

/// No events. A chain takes its bits from the line.
pub(crate) const ROW: crate::device::Row = crate::device::Row::new(&[]);

pub(crate) fn chip(part: PartId, chain: &Ws2812) -> Chip {
    let pins = &chain.pins;
    let against = |fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[DIN]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let threshold = &chain.input_threshold;
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let outputs = chain
        .dout
        .iter()
        .map(|dout| Output {
            pin: pins[DOUT],
            vcc: pins[VCC],
            gnd: pins[GND],
            high: stage(&dout.output.high_drop, &dout.output.high_resistance),
            low: stage(&dout.output.low_drop, &dout.output.low_resistance),
            pull_up: None,
            pull_down: None,
            leakage: None,
        })
        .collect();
    let delay = chain.dout.as_ref().map_or(0, |dout| dout.delay.value);
    Chip {
        part: Some(part),
        outputs,
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
        pixels: Some(PixelDraw {
            vcc: pins[VCC],
            gnd: pins[GND],
            channel: chain.channel_current,
            idle: chain.idle_current,
            count: chain.leds,
        }),
        takes: Takes {
            spi: None,
            duty: None,
            pulses: Some(pins[DIN]),
        },
        core: Core::Device(core(chain.leds, delay)),
    }
}

/// The device sized to the chain: the smallest of 16, 64, 256 and 1024 LEDs that holds it. A
/// landing copies the state, so a short chain copies little. `delay` is `DIN` to `DOUT` in
/// picoseconds, 0 for a chain with no `DOUT`.
fn core(leds: u16, delay: u64) -> Box<dyn Hosted> {
    match leds {
        0..=16 => sized::<16>(leds, delay),
        17..=64 => sized::<64>(leds, delay),
        65..=256 => sized::<256>(leds, delay),
        _ => sized::<{ Ws2812::LED_MAX as usize }>(leds, delay),
    }
}

fn sized<const N: usize>(leds: u16, delay: u64) -> Box<dyn Hosted> {
    let delay = i64::try_from(delay).expect("parse bounds the delay");
    let device = model::Ws2812::<N>::new(&Fields::new(&[
        ("leds", FieldValue::Int(i64::from(leds))),
        ("delay", FieldValue::Int(delay)),
    ]))
    .expect("the state holds the chain");
    let outputs = if delay > 0 { model::OUTPUTS } else { 0 };
    Box::new(Native::new(device, Pulse::new(), outputs))
}

/// A chain of WS2812E-1313 LEDs, an active chip with no clock of its own. It reads `DIN`
/// through `input_threshold`, fractions of its supply, and takes `Pulses` records on that net.
/// Each LED keeps the first 24 bits, green then red then blue, and passes the rest on. A low
/// longer than 280 µs latches. `leds` is from 1 to [`Ws2812::LED_MAX`]. With `dout`, the last
/// LED passes the rest on through `DOUT`. Only parse builds one.
#[derive(Clone, Debug, PartialEq)]
pub struct Ws2812 {
    /// `VCC`, `GND` and `DIN`, then `DOUT` when `dout` is given.
    pub pins: Vec<NetId>,
    pub leds: u16,
    pub input_threshold: Threshold,
    /// Amperes at a channel value of 255.
    pub channel_current: f64,
    /// Amperes per LED, at every colour.
    pub idle_current: f64,
    pub dout: Option<Dout>,
}

/// How a chain drives `DOUT`: through `output` from its own rails, each `DIN` high once every
/// LED holds its bits, `delay` later.
#[derive(Clone, Debug, PartialEq)]
pub struct Dout {
    pub output: OutputStage,
    /// Picoseconds, from 1 to [`DOUT_DELAY_MAX`].
    pub delay: IntValue,
}

impl Ws2812 {
    /// The first three are required. `DOUT` is wired exactly when the part gives `dout`.
    pub const PINS: [&'static str; 4] = ["VCC", "GND", "DIN", "DOUT"];
    /// How many of [`PINS`](Self::PINS) every chain wires.
    pub const REQUIRED: usize = 3;
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
    #[serde(default)]
    channel_current: Option<WireValue>,
    #[serde(default)]
    idle_current: Option<WireValue>,
    #[serde(default)]
    dout: Option<WireDout>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDout {
    output: WireOutputStage,
    delay: WireInt<u64>,
}

pub(crate) fn resolve_ws2812(ctx: ParseCtx<'_>, wire: WireWs2812) -> Option<Ws2812> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = if wire.dout.is_some() {
        resolve_pins(name, Ws2812::PINS, &wire.pins.0, nets, out).map(Vec::from)
    } else {
        let [vcc, gnd, din, _] = Ws2812::PINS;
        resolve_pins(name, [vcc, gnd, din], &wire.pins.0, nets, out).map(Vec::from)
    };
    let dout = wire.dout.map(|dout| {
        let output = resolve_output_stage_at(part_ref, "dout.output", dout.output, out);
        let delay = resolve_int(part_ref, "dout.delay", "ps", dout.delay, out);
        if let Some(delay) = &delay
            && delay.value > DOUT_DELAY_MAX
        {
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: "dout.delay".into(),
                value: delay.value as f64,
                rule: ValueRule::Delay,
            });
            return None;
        }
        Some(Dout {
            output: output?,
            delay: delay?,
        })
    });
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let channel_current = resolve_draw(
        part_ref,
        "channel_current",
        wire.channel_current,
        CHANNEL_CURRENT,
        out,
    );
    let idle_current = resolve_draw(
        part_ref,
        "idle_current",
        wire.idle_current,
        IDLE_CURRENT,
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
    let dout = match dout {
        Some(dout) => Some(dout?),
        None => None,
    };
    Some(Ws2812 {
        pins: pins?,
        leds: leds?,
        input_threshold: input_threshold?,
        channel_current: channel_current?,
        idle_current: idle_current?,
        dout,
    })
}

/// `wire` in amperes, or `absent` when the part omits it. Zero is legal.
fn resolve_draw(
    part: &crate::name::Ref,
    field: &'static str,
    wire: Option<WireValue>,
    absent: f64,
    out: &mut Vec<Diagnostic>,
) -> Option<f64> {
    let Some(wire) = wire else {
        return Some(absent);
    };
    bounded(
        part,
        field,
        "A",
        wire,
        |number| (number < 0.0).then_some(ValueRule::NonNegative),
        out,
    )
    .map(|value| value.si.get())
}
