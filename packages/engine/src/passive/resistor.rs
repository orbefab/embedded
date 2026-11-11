use orbe_model::Time;
use serde::Deserialize;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, TWO_PIN, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::{
    ParseCtx, PinMap, WireValue, resolve_pins, resolve_rating, resolve_value,
};
use crate::netlist::{Conduction, Kind, NetId, Value};
use crate::newton::Side;

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

pub(crate) fn resolve_resistor(ctx: ParseCtx<'_>, wire: WireResistor) -> Option<Resistor> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
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
        (Some(pins), Some(resistance), Some(max_power)) => Some(Resistor {
            pins,
            resistance,
            max_power,
        }),
        _ => None,
    }
}

impl Kind for Resistor {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns::default()
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let g = 1.0 / self.resistance.si.get();
        stamp.conductance(self.pins[0], self.pins[1], g);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        TWO_PIN
    }
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
        (None, self.max_power.as_ref())
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        let i = (probe.v(self.pins[0]) - probe.v(self.pins[1])) / self.resistance.si.get();
        Some(Observed::two_pin(self.pins, i, probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Limits
    }
}
