use crate::diagnostic::ValueRule;
use crate::netlist::Rating;
use crate::netlist::json::{WireRating, WireValue, resolve_value};
use orbe_model::Time;
use serde::Deserialize;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, TWO_PIN, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::{ParseCtx, PinMap, resolve_pins, resolve_rating};
use crate::netlist::{Conduction, Kind, NetId, broken_positive};
use crate::newton::Side;
use crate::world::json::{FieldRule, WireParam, resolve_param};
use crate::world::{Driven, Param};

#[derive(Clone, Debug, PartialEq)]
pub struct Resistor {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Ohm, greater than 0 with a finite reciprocal, fixed or driven by a world quantity.
    pub resistance: Param,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Rating>,
    /// Operating voltage of a sounding drive, `(min, max)` in volt. `None` when the part
    /// does not rate one. A quiet part is not checked.
    pub operating: Option<(f64, f64)>,
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
    power: Option<WireRating>,
    /// Absent on a resistor that rates only power. Present, both ends are required.
    #[serde(default)]
    voltage: Option<WireOperatingVoltage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireOperatingVoltage {
    min: WireValue,
    max: WireValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireResistor {
    pub(crate) name: String,
    pins: PinMap,
    resistance: WireParam,
    ratings: WireResistorRatings,
}

pub(crate) fn resolve_resistor(ctx: ParseCtx<'_>, wire: WireResistor) -> Option<Resistor> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        world,
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
    let rule = FieldRule {
        field: "resistance",
        unit: "ohm",
        broken: broken_positive,
    };
    let resistance = resolve_param(part_ref, rule, world, resistance, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    let operating = resolve_operating(part_ref, ratings.voltage, out);
    match (pins, resistance, max_power, operating) {
        (Some(pins), Some(resistance), Some(max_power), Some(operating)) => Some(Resistor {
            pins,
            resistance,
            max_power,
            operating,
        }),
        _ => None,
    }
}

/// `Some(None)` when the part rates no operating voltage. `None` when `min` or `max` is invalid.
fn resolve_operating(
    part: &crate::name::Ref,
    voltage: Option<WireOperatingVoltage>,
    out: &mut Vec<crate::diagnostic::Diagnostic>,
) -> Option<Option<(f64, f64)>> {
    let Some(WireOperatingVoltage { min, max }) = voltage else {
        return Some(None);
    };
    let min = resolve_value(part, "ratings.voltage.min", "V", true, min, out);
    let max = resolve_value(part, "ratings.voltage.max", "V", true, max, out);
    match (min, max) {
        (Some(min), Some(max)) if min.si.get() < max.si.get() => {
            Some(Some((min.si.get(), max.si.get())))
        }
        (Some(min), Some(_)) => {
            out.push(crate::diagnostic::Diagnostic::NetlistValue {
                part: part.clone(),
                field: "ratings.voltage.min".into(),
                value: min.si.get(),
                rule: ValueRule::AboveHigh,
            });
            None
        }
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
        let conditions = stamp.conditions;
        let g = 1.0 / self.resistance.read(conditions.side(), conditions.at);
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
    fn ratings(&self) -> (Option<(Quantity, &Rating)>, Option<&Rating>) {
        (None, self.max_power.as_ref())
    }
    fn observe(&self, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed> {
        let ohms = self.resistance.read(side, at);
        let i = (probe.v(self.pins[0]) - probe.v(self.pins[1])) / ohms;
        Some(Observed::two_pin(self.pins, i, probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Limits
    }
    fn driven_mut(&mut self) -> Option<&mut Driven> {
        self.resistance.driven_mut()
    }
}

#[cfg(test)]
mod operating_voltage {
    fn pulse(volts: f64) -> String {
        format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "transient": {{ "stop_ps": 2000000, "max_step_ps": 1000000, "grid_ps": 1000000 }} }},
              "ground": "gnd",
              "nets": ["gnd", "n"],
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "n", "n": "gnd" }},
                  "voltage": {{ "kind": "pulse",
                    "initial": {{ "value": 0.0, "unit": "V" }},
                    "pulsed": {{ "value": {volts}, "unit": "V" }},
                    "delay_ps": 100000, "width_ps": 400000, "period_ps": 1000000 }} }},
                {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "n", "b": "gnd" }},
                  "resistance": {{ "value": 42.0, "unit": "ohm" }},
                  "ratings": {{ "power": null, "voltage": {{
                    "min": {{ "value": 3.0, "unit": "V" }},
                    "max": {{ "value": 5.0, "unit": "V" }} }} }} }}
              ]
            }}"#
        )
    }

    fn warnings(json: &str) -> String {
        let netlist = crate::netlist::parse(json.as_bytes()).unwrap();
        crate::simulate(&netlist)
            .diagnostics()
            .iter()
            .map(|d| serde_json::to_string(d).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_drive_outside_3_v_to_5_v_warns_and_one_inside_does_not() {
        let quiet = warnings(&pulse(0.7));
        assert!(quiet.contains("rating.voltage"), "{quiet}");
        assert!(quiet.contains("operating voltage"), "{quiet}");
        assert!(quiet.contains("0.7"), "{quiet}");
        let inside = warnings(&pulse(4.0));
        assert!(!inside.contains("rating.voltage"), "{inside}");
        let hot = warnings(&pulse(6.0));
        assert!(hot.contains("rating.voltage"), "{hot}");
        assert!(hot.contains("operating voltage"), "{hot}");
    }
}
