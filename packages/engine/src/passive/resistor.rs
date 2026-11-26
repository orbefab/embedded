use crate::diagnostic::ValueRule;
use crate::netlist::json::{WireRating, WireValue, resolve_value};
use crate::netlist::{Rating, Value};
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

/// A sounding drive that leaves the coil quiet for longer than this has stopped. It is the
/// half-cycle of 31 Hz, the lowest note an Uno or Nano plays with `tone()`, plus the most a note
/// change can stretch one half-cycle, rounded up: 16.384 ms is 256 timer ticks at 64 us.
pub const DRIVE_REST_PS: u64 = 17_000_000_000;

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
    /// Maximum DC voltage across `a` and `b`, in volt. `|V(a) - V(b)|` above this warns
    /// `rating.voltage` on every sample. `None` when not rated.
    pub max_voltage: Option<Value>,
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
    /// Absent on a resistor that rates only power. Either an operating window `{ min, max }`
    /// or a single maximum voltage `{ value, unit }`.
    #[serde(default)]
    voltage: Option<WireResistorVoltage>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum WireResistorVoltage {
    Operating(WireOperatingVoltage),
    Max(WireValue),
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
    let voltage = resolve_voltage(part_ref, ratings.voltage, out);
    match (pins, resistance, max_power, voltage) {
        (Some(pins), Some(resistance), Some(max_power), Some((operating, max_voltage))) => {
            Some(Resistor {
                pins,
                resistance,
                max_power,
                operating,
                max_voltage,
            })
        }
        _ => None,
    }
}

type ResolvedVoltage = (Option<(f64, f64)>, Option<Value>);

/// `Some((None, None))` when the part rates no voltage. `None` when the rating is invalid.
fn resolve_voltage(
    part: &crate::name::Ref,
    voltage: Option<WireResistorVoltage>,
    out: &mut Vec<crate::diagnostic::Diagnostic>,
) -> Option<ResolvedVoltage> {
    let Some(voltage) = voltage else {
        return Some((None, None));
    };
    match voltage {
        WireResistorVoltage::Operating(WireOperatingVoltage { min, max }) => {
            let min = resolve_value(part, "ratings.voltage.min", "V", true, min, out);
            let max = resolve_value(part, "ratings.voltage.max", "V", true, max, out);
            match (min, max) {
                (Some(min), Some(max)) if min.si.get() < max.si.get() => {
                    Some((Some((min.si.get(), max.si.get())), None))
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
        WireResistorVoltage::Max(max) => {
            resolve_value(part, "ratings.voltage", "V", true, max, out).map(|max| (None, Some(max)))
        }
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

    /// Five 6 V pulses, 100 us wide every 200 us from 100 us, then 4 ms of quiet.
    fn burst() -> String {
        let mut events = Vec::new();
        for pulse in 0..5u64 {
            let rise = 100_000_000 + pulse * 200_000_000;
            events.push(format!(
                r#"{{ "t_ps": {rise}, "level": {{ "value": 6.0, "unit": "V" }} }}"#
            ));
            events.push(format!(
                r#"{{ "t_ps": {}, "level": {{ "value": 0.0, "unit": "V" }} }}"#,
                rise + 100_000_000
            ));
        }
        format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "transient": {{ "stop_ps": 5000000000, "max_step_ps": 10000000, "grid_ps": 10000000 }} }},
              "ground": "gnd",
              "nets": ["gnd", "n"],
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "n", "n": "gnd" }},
                  "voltage": {{ "kind": "steps",
                    "initial": {{ "value": 0.0, "unit": "V" }},
                    "events": [{}] }} }},
                {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "n", "b": "gnd" }},
                  "resistance": {{ "value": 42.0, "unit": "ohm" }},
                  "ratings": {{ "power": null, "voltage": {{
                    "min": {{ "value": 3.0, "unit": "V" }},
                    "max": {{ "value": 5.0, "unit": "V" }} }} }} }}
              ]
            }}"#,
            events.join(", ")
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

    #[test]
    fn a_drive_outside_3_v_to_5_v_counts_while_it_sounds_and_not_after() {
        let netlist = crate::netlist::parse(burst().as_bytes()).unwrap();
        let results = crate::simulate(&netlist);
        let over_ps: Vec<u64> = results
            .diagnostics()
            .iter()
            .map(|d| serde_json::to_value(d).unwrap())
            .filter(|d| d["code"] == "rating.voltage")
            .map(|d| d["data"]["over_ps"].as_u64().unwrap())
            .collect();
        // From the first rise to the last fall is 900 us. The 4 ms of quiet after it is a rest.
        assert_eq!(over_ps.len(), 1, "{over_ps:?}");
        assert!(
            (900_000_000..=950_000_000).contains(&over_ps[0]),
            "{over_ps:?}"
        );
    }

    #[test]
    fn a_max_voltage_warns_past_either_polarity_and_stays_quiet_at_the_limit() {
        let dc = |volts: f64| {
            format!(
                r#"{{
                  "version": 1,
                  "seed": 0,
                  "analysis": {{ "op": {{}} }},
                  "ground": "gnd",
                  "nets": ["gnd", "n"],
                  "parts": [
                    {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "n", "n": "gnd" }},
                      "voltage": {{ "value": {volts}, "unit": "V" }} }},
                    {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "n", "b": "gnd" }},
                      "resistance": {{ "value": 10000.0, "unit": "ohm" }},
                      "ratings": {{ "power": null, "voltage": {{ "value": 50.0, "unit": "V" }} }} }}
                  ]
                }}"#
            )
        };
        assert!(!warnings(&dc(50.0)).contains("rating.voltage"));
        assert!(!warnings(&dc(-50.0)).contains("rating.voltage"));
        let high = warnings(&dc(60.0));
        assert!(high.contains("rating.voltage"), "{high}");
        assert!(
            high.contains("#r1.v reaches 60 V, outside its -50 V to 50 V rating"),
            "{high}"
        );
        assert!(!high.contains("chip's ground"), "{high}");
        let low = warnings(&dc(-60.0));
        assert!(low.contains("rating.voltage"), "{low}");
        assert!(
            low.contains("#r1.v reaches -60 V, outside its -50 V to 50 V rating"),
            "{low}"
        );
    }
}
