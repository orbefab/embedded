//! A `ws2812` chain behind the seam. It watches `DIN` and takes `Pulses` on that net. The
//! device in [`model`] keeps the bits and the colours, shows the colours, and drives `DOUT`
//! when the part wires it. The pulse engine times the line, and sends what a `Pulses` record
//! passes on to `DOUT` as a record on that net.

use serde::Deserialize;

use orbe_model::{Device, Fields, Value as FieldValue};

use crate::active::native::{Hosted, Native};
use crate::active::pulse::{Pulse, Relay};
use crate::active::{Chip, Core, Output, Stage, Takes};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{
    PinMap, WireInt, WireValue, bounded, resolve_int, resolve_limit, resolve_pins, resolve_value,
};
use crate::netlist::{IntValue, NetId, PartId, Threshold, Value};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage_at};
use crate::ratings::{Ceiling, PinRatings, PinWindow};
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
            strong_pull_up: None,
            pull_down: None,
            leakage: None,
        })
        .collect();
    let delay = chain.dout.as_ref().map_or(0, |dout| dout.delay.value);
    let dout = chain.dout.as_ref().map(|_| pins[DOUT]);
    let mut watches = vec![Watch {
        rise: against(threshold.high.si.get()),
        fall: against(threshold.low.si.get()),
    }];
    if let Some(min_supply) = &chain.min_supply {
        watches.push(Watch::comparator(Signal {
            terms: vec![(Node::from(pins[VCC]), 1.0), (Node::from(pins[GND]), -1.0)],
            offset: min_supply.si.get().next_down(),
        }));
    }
    Chip {
        part: Some(part),
        outputs,
        watches,
        signals: vec![(Ws2812::PINS[DIN], model::WATCH_DIN)],
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: dout.is_some(),
        supply: None,
        draw: chain.draw.as_ref().map(|_| (pins[VCC], pins[GND])),
        takes: Takes {
            spi: None,
            duty: None,
            pulses: Some(pins[DIN]),
            pulses_pin: Some(DIN),
        },
        core: Core::Device(core(
            chain.leds,
            delay,
            dout,
            chain.draw.as_ref(),
            chain.min_supply.as_ref(),
        )),
    }
}

/// The device sized to the chain: the smallest of 16, 64, 256 and 1024 LEDs that holds it. A
/// landing copies the state, so a short chain copies little. `delay` is `DIN` to `DOUT` in
/// picoseconds, 0 for a chain with no `DOUT`, and `dout` the net of `DOUT`, which the records it
/// relays go on.
fn core(
    leds: u16,
    delay: u64,
    dout: Option<NetId>,
    draw: Option<&PixelDraw>,
    min_supply: Option<&Value>,
) -> Box<dyn Hosted> {
    match leds {
        0..=16 => sized::<16>(leds, delay, dout, draw, min_supply),
        17..=64 => sized::<64>(leds, delay, dout, draw, min_supply),
        65..=256 => sized::<256>(leds, delay, dout, draw, min_supply),
        _ => sized::<{ Ws2812::LED_MAX as usize }>(leds, delay, dout, draw, min_supply),
    }
}

/// The device takes its draw as whole nanoamperes and millivolts, and its minimum supply as
/// whole microvolts.
fn sized<const N: usize>(
    leds: u16,
    delay: u64,
    dout: Option<NetId>,
    draw: Option<&PixelDraw>,
    min_supply: Option<&Value>,
) -> Box<dyn Hosted> {
    let delay = i64::try_from(delay).expect("parse bounds the delay");
    let whole = |value: &Value, scale: f64| (value.si.get() * scale).round() as i64;
    let mut fields = vec![
        ("leds", FieldValue::Int(i64::from(leds))),
        ("delay", FieldValue::Int(delay)),
    ];
    if let Some(draw) = draw {
        fields.extend([
            (
                "channel_na",
                FieldValue::Int(whole(&draw.channel_current, 1e9)),
            ),
            ("idle_na", FieldValue::Int(whole(&draw.idle_current, 1e9))),
            (
                "holds_above_mv",
                FieldValue::Int(whole(&draw.holds_above, 1e3)),
            ),
        ]);
    }
    if let Some(min_supply) = min_supply {
        let uv = (min_supply.si.get() * 1e6).round().max(1.0) as i64;
        fields.push(("min_supply_uv", FieldValue::Int(uv)));
    }
    let device = model::Ws2812::<N>::new(&Fields::new(&fields)).expect("the state holds the chain");
    let outputs = if delay > 0 { model::OUTPUTS } else { 0 };
    let mut port = match dout {
        Some(net) => Pulse::relaying(Relay::new(Ws2812::PINS[DOUT], net)),
        None => Pulse::new(),
    };
    if min_supply.is_some() {
        port = port.with_supply(model::WATCH_SUPPLY);
    }
    Box::new(Native::new(device, port, outputs))
}

/// The part's voltage ratings on `VCC` and `DIN`, `None` when it has no `ratings` object.
/// Every window is over `GND`: `VCC` takes a ceiling of `supply_voltage`, and `DIN` takes
/// `-rail_margin` to `VCC + rail_margin`.
pub(crate) fn pin_ratings(chain: &Ws2812) -> Option<PinRatings> {
    let ratings = chain.ratings.as_ref()?;
    let si = |value: &Option<Value>| value.as_ref().map(|value| value.si.get());
    let margin = si(&ratings.rail_margin);
    let supply = si(&ratings.supply_voltage);
    let mut windows = Vec::new();
    if let Some(supply) = supply {
        windows.push(PinWindow {
            pin: Ws2812::PINS[VCC],
            net: chain.pins[VCC],
            below: None,
            above: Some(Ceiling::Fixed(supply)),
        });
    }
    if let Some(margin) = margin {
        windows.push(PinWindow {
            pin: Ws2812::PINS[DIN],
            net: chain.pins[DIN],
            below: Some(margin),
            above: Some(Ceiling::OverSupply(margin)),
        });
    }
    Some(PinRatings {
        outputs: Vec::new(),
        pin_current: None,
        supply_current: None,
        vcc: (Ws2812::PINS[VCC], chain.pins[VCC]),
        gnd: (Ws2812::PINS[GND], chain.pins[GND]),
        windows,
    })
}

/// A chain of WS2812E-1313 LEDs, an active chip with no clock of its own. It reads `DIN`
/// through `input_threshold`, fractions of its supply, and takes `Pulses` records on that net.
/// Each LED keeps the first 24 bits, green then red then blue, and passes the rest on. A low
/// longer than 280 µs latches. `leds` is from 1 to [`Ws2812::LED_MAX`]. With `dout`, the last
/// LED passes the rest on through `DOUT`. With `min_supply`, the chain ignores `DIN`, clears to
/// off and releases `DOUT` while `V(VCC) - V(GND)` is below it. Only parse builds one.
#[derive(Clone, Debug, PartialEq)]
pub struct Ws2812 {
    /// `VCC`, `GND` and `DIN`, then `DOUT` when `dout` is given.
    pub pins: Vec<NetId>,
    pub leds: u16,
    pub input_threshold: Threshold,
    /// Volt, greater than 0. `None` runs at any supply.
    pub min_supply: Option<Value>,
    pub dout: Option<Dout>,
    /// What each LED draws from `VCC` into `GND`. `None` draws nothing.
    pub draw: Option<PixelDraw>,
    /// The pin voltage ratings the run checks. `None` when the part has no `ratings` object.
    pub ratings: Option<Ws2812Ratings>,
}

/// A WS2812 / SK68XX chain's voltage ratings. Each is `None` when the part leaves it unrated.
#[derive(Clone, Debug, PartialEq)]
pub struct Ws2812Ratings {
    /// Volt. How far `DIN` may sit below `GND` and above `VCC`.
    pub rail_margin: Option<Value>,
    /// Volt. The most `VCC` may sit above `GND`.
    pub supply_voltage: Option<Value>,
}

/// Each LED's draw from `VCC` into `GND`: `idle_current` at every colour, black included, and
/// `channel_current` times each channel's latched value over 255, while the supply is at or
/// above `holds_above`. Below that it fades to none at 0 V, as `orbe_model::Draw` says.
#[derive(Clone, Debug, PartialEq)]
pub struct PixelDraw {
    /// Ampere, at least 0, at a channel value of 255.
    pub channel_current: Value,
    /// Ampere, at least 0.
    pub idle_current: Value,
    /// Volt, greater than 0.
    pub holds_above: Value,
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
    min_supply: Option<WireValue>,
    #[serde(default)]
    dout: Option<WireDout>,
    #[serde(default)]
    draw: Option<WireDraw>,
    #[serde(default)]
    ratings: Option<WireRatings>,
}

/// Every key is required, and `null` leaves that rating out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    rail_margin: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_voltage: Option<WireValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDraw {
    channel_current: WireValue,
    idle_current: WireValue,
    holds_above: WireValue,
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
    let min_supply = match wire.min_supply {
        None => Some(None),
        Some(value) => resolve_value(part_ref, "min_supply", "V", true, value, out).map(Some),
    };
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
        let channel_current = current("draw.channel_current", draw.channel_current, out);
        let idle_current = current("draw.idle_current", draw.idle_current, out);
        let holds_above = resolve_value(
            part_ref,
            "draw.holds_above",
            "V",
            true,
            draw.holds_above,
            out,
        );
        Some(PixelDraw {
            channel_current: channel_current?,
            idle_current: idle_current?,
            holds_above: holds_above?,
        })
    });
    let dout = match dout {
        Some(dout) => Some(dout?),
        None => None,
    };
    let draw = match draw {
        Some(draw) => Some(draw?),
        None => None,
    };
    let ratings = match wire.ratings {
        None => Some(None),
        Some(wire) => resolve_ratings(part_ref, wire, out).map(Some),
    };
    Some(Ws2812 {
        pins: pins?,
        leds: leds?,
        input_threshold: input_threshold?,
        min_supply: min_supply?,
        dout,
        draw,
        ratings: ratings?,
    })
}

fn resolve_ratings(
    part: &crate::name::Ref,
    wire: WireRatings,
    out: &mut Vec<Diagnostic>,
) -> Option<Ws2812Ratings> {
    let rail_margin = resolve_limit(part, "ratings.rail_margin", "V", wire.rail_margin, out);
    let supply_voltage = resolve_limit(
        part,
        "ratings.supply_voltage",
        "V",
        wire.supply_voltage,
        out,
    );
    Some(Ws2812Ratings {
        rail_margin: rail_margin?,
        supply_voltage: supply_voltage?,
    })
}
