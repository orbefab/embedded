//! The `dht22` temperature and humidity sensor part. [`model`] is the device: the measurement
//! and the reply. This file is its netlist struct, its JSON wire type, its row in the device
//! table, and the electrical side: `DATA` is an open drain the host's pulse engine pulls low
//! for each low of a reply, and the line rises on the circuit's own pull-up between them.

pub(crate) mod model;
#[cfg(test)]
mod tests;

use std::borrow::Cow;

use serde::Deserialize;

use orbe_model::{Device, Fields};

use super::Row;
use crate::active::native::Native;
use crate::active::pulse::Pulse;
use crate::active::{Chip, Core, Output, Stage, Takes};
use crate::diagnostic::{Diagnostic, WorldRule};
use crate::mna::Node;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, resolve_pins, resolve_value};
use crate::netlist::{NetId, PartId, Threshold, Value};
use crate::watch::{Signal, Watch};

const VCC: usize = 0;
const DATA: usize = 1;
const GND: usize = 2;

/// The pin pairs it joins at DC, for `net.floating`: `DATA` can pull to `GND`.
pub(crate) const DC_PATHS: [(usize, usize); 1] = [(DATA, GND)];

/// No events. The sensor reads the world.
pub(crate) const ROW: Row = Row::new(&[]);

pub(crate) fn chip(part: PartId, dev: &Dht22) -> Chip {
    let pins = dev.pins;
    let drain = Stage {
        drop: 0.0,
        resistance: dev.low_resistance.si.get(),
    };
    let against = |fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[DATA]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    Chip {
        part: Some(part),
        outputs: vec![Output {
            pin: pins[DATA],
            vcc: pins[VCC],
            gnd: pins[GND],
            high: drain,
            low: drain,
            pull_up: None,
            strong_pull_up: None,
            pull_down: None,
            leakage: None,
        }],
        watches: vec![Watch {
            rise: against(dev.input_threshold.high.si.get()),
            fall: against(dev.input_threshold.low.si.get()),
        }],
        signals: vec![(Dht22::PINS[DATA], 0)],
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: None,
        takes: Takes::default(),
        core: Core::Device(Box::new(Native::new(
            model::Dht22::new(&Fields::NONE).expect("a dht22 takes no params"),
            Pulse::new(),
            0,
        ))),
    }
}

/// A DHT22 (AM2302) temperature and humidity sensor, an active chip with no clock of its own.
/// `DATA` is open drain into `GND` through `low_resistance`, and the part reads it through
/// `input_threshold`, fractions of its supply. It reads the netlist's `temperature` and
/// `humidity`, which the netlist's `world` must declare.
#[derive(Clone, Debug, PartialEq)]
pub struct Dht22 {
    pub pins: [NetId; 3],
    /// Ohm.
    pub low_resistance: Value,
    pub input_threshold: Threshold,
}

impl Dht22 {
    pub const PINS: [&'static str; 3] = ["VCC", "DATA", "GND"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireDht22 {
    pub(crate) name: String,
    pins: PinMap,
    low_resistance: WireValue,
    input_threshold: WireThreshold,
}

pub(crate) fn resolve_dht22(ctx: ParseCtx<'_>, wire: WireDht22) -> Option<Dht22> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        world,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Dht22::PINS, &wire.pins.0, nets, out);
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
    let mut declared = true;
    for &quantity in model::Dht22::WORLD {
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
    let dht22 = Dht22 {
        pins: pins?,
        low_resistance: low_resistance?,
        input_threshold: input_threshold?,
    };
    declared.then_some(dht22)
}
