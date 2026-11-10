use crate::diagnostic::Diagnostic;
use crate::name::Name;
use crate::name::Ref;
use crate::netlist::Model;
use crate::netlist::NetId;
use crate::netlist::Value;
use crate::netlist::json::PinMap;
use crate::netlist::json::WireValue;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_rating;
use crate::netlist::json::resolve_value;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::mna::{Probe, Stamp};

pub(crate) fn stamp(resistor: &Resistor, stamp: &mut Stamp<'_>) {
    let g = 1.0 / resistor.resistance.si.get();
    stamp.conductance(resistor.pins[0], resistor.pins[1], g);
}

pub(crate) fn current(resistor: &Resistor, probe: &Probe<'_>) -> f64 {
    (probe.v(resistor.pins[0]) - probe.v(resistor.pins[1])) / resistor.resistance.si.get()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resistor {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Ohm, greater than 0.
    pub resistance: Value,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

impl Resistor {
    pub const PINS: [&'static str; 2] = ["a", "b"];
}

// `Option::deserialize` makes each key required while its value may be `null`.
// Plain `Option` would read a missing key as `None`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireResistorRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    power: Option<WireValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireResistor {
    pub(crate) name: String,
    pins: PinMap,
    resistance: WireValue,
    ratings: WireResistorRatings,
}

pub(crate) fn resolve_resistor(
    name: &Name,
    part_ref: &Ref,
    wire: WireResistor,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Model> {
    let WireResistor {
        pins,
        resistance,
        ratings,
        ..
    } = wire;

    let pins = resolve_pins(name, Resistor::PINS, &pins.0, nets, out);
    let resistance = resolve_value(part_ref, "resistance", "ohm", true, resistance, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    match (pins, resistance, max_power) {
        (Some(pins), Some(resistance), Some(max_power)) => Some(Model::Resistor(Resistor {
            pins,
            resistance,
            max_power,
        })),
        _ => None,
    }
}
