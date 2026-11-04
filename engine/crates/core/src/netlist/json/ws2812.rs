//! The `ws2812` kind: its pins, its LED count and its `DIN` threshold.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::atmega328p::{WireThreshold, resolve_threshold};
use super::{PinMap, resolve_pins};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::name::{Name, Ref};
use crate::netlist::{NetId, Ws2812};

fn default_leds() -> u16 {
    Ws2812::LEDS_DEFAULT
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireWs2812 {
    pub(super) name: String,
    pins: PinMap,
    #[serde(default = "default_leds")]
    leds: u16,
    input_threshold: WireThreshold,
}

pub(super) fn resolve_ws2812(
    name: &Name,
    part_ref: &Ref,
    wire: WireWs2812,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Ws2812> {
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
