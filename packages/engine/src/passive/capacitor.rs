use orbe_model::Time;
use serde::Deserialize;

use crate::diagnostic::ValueRule;
use crate::element::{Observed, OwnedCapacitor, OwnedInductor, StoreEnd, TWO_PIN, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::{ParseCtx, PinMap, WireValue, bounded, resolve_pins, resolve_value};
use crate::netlist::{Conduction, Kind, NetId, Value};
use crate::newton::{History, Method, Side, Store};

/// Seconds. A held capacitor sits behind `HOLD / C` ohm, the backward Euler companion
/// of a step this long. An exact hold would make a capacitor across a voltage source,
/// or two in parallel, singular. The series resistance shares their current by
/// capacitance, and moves the held voltage by the current times `HOLD / C`.
const HOLD: f64 = 1e-15;

/// Stamp capacitor `k` from `ends[0]` to `ends[1]`. Branch `k` is the current entering
/// `ends[0]`. A held voltage scales with the sources. `initial` is the voltage held at
/// t = 0, and `None` leaves the capacitor open. A restart holds the voltage in the
/// solution before the edge. A step follows the companion. `v0` and `i0` come from the
/// last accepted solution on branch `k`.
pub(crate) fn stamp_capacitor(
    stamp: &mut Stamp<'_>,
    k: usize,
    ends: [StoreEnd; 2],
    capacitance: f64,
    initial: Option<f64>,
) {
    let [a, b] = ends.map(|end| end.node(stamp.layout, stamp.part));
    let scale = stamp.conditions.source_scale;
    stamp.branch_current(a, b, k);
    let held = |stamp: &mut Stamp<'_>, volts: f64| {
        stamp.branch_row(a, b, k, -HOLD / capacitance, 1.0, volts * scale);
    };
    match stamp.conditions.history {
        History::Start => match initial {
            None => stamp.branch_row(a, b, k, 1.0, 0.0, 0.0),
            Some(volts) => held(stamp, volts),
        },
        History::Restart { previous } => held(stamp, previous.node(a) - previous.node(b)),
        History::Step(step) => {
            let previous = stamp.probe(step.previous);
            let v0 = previous.v(a) - previous.v(b);
            let index = stamp.layout.charge_index(stamp.part, k);
            let method = step.method_of(Store::Charge(index));
            let (g, rhs) = companion(capacitance, method, step.h(), v0, previous.branch(k));
            stamp.branch_row(a, b, k, 1.0, -g, rhs);
        }
    }
}

/// The step's companion `i = g * v + rhs`, as `(g, rhs)`.
fn companion(capacitance: f64, method: Method, h: f64, v0: f64, i0: f64) -> (f64, f64) {
    match method {
        Method::BackwardEuler => {
            let g = capacitance / h;
            (g, -g * v0)
        }
        Method::Trapezoidal => {
            let g = 2.0 * capacitance / h;
            (g, -g * v0 - i0)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::netlist::{self};

    use super::*;

    #[test]
    fn backward_euler_is_the_slope_over_the_step() {
        let (g, rhs) = companion(1e-6, Method::BackwardEuler, 1e-3, 2.0, 5.0);
        assert_eq!(g, 1e-3);
        assert_eq!(g * 3.0 + rhs, 1e-6 * (3.0 - 2.0) / 1e-3);
        assert_eq!(g * 2.0 + rhs, 0.0);
    }

    #[test]
    fn trapezoidal_averages_the_two_currents() {
        let (g, rhs) = companion(1e-6, Method::Trapezoidal, 1e-3, 2.0, 5e-4);
        let i1 = g * 3.0 + rhs;
        let mean = (i1 + 5e-4) / 2.0;
        assert!((mean - 1e-6 * (3.0 - 2.0) / 1e-3).abs() <= 1e-18, "{mean}");
        let held = g * 2.0 + rhs;
        assert!((held + 5e-4).abs() <= 1e-15, "{held}");
    }

    /// `a_r` then `b_v` then `c_c`, so the capacitor's part index is not its charge index.
    /// A stiff charge takes backward Euler. Stiffening the part index leaves trapezoidal.
    #[test]
    fn a_step_stiffens_the_charge_index_and_not_the_part_index() {
        use crate::name::Name;

        use crate::mna::{Solution, System};
        use crate::newton::{ChargeIndex, Circuit, Conditions, Integration, Iterate, Store};

        let netlist = netlist::parse(
            br#"{
              "version": 1,
              "seed": 0,
              "analysis": { "op": {} },
              "ground": "gnd",
              "nets": ["gnd", "n"],
              "parts": [
                { "name": "a_r", "kind": "resistor", "pins": { "a": "n", "b": "gnd" },
                  "resistance": { "value": 1000.0, "unit": "ohm" },
                  "ratings": { "power": null } },
                { "name": "b_v", "kind": "voltage_source", "pins": { "p": "n", "n": "gnd" },
                  "voltage": { "value": 2.0, "unit": "V" } },
                { "name": "c_c", "kind": "capacitor", "pins": { "a": "n", "b": "gnd" },
                  "capacitance": { "value": 1e-6, "unit": "F" },
                  "initial": null }
              ]
            }"#,
        )
        .unwrap();
        let circuit = Circuit::new(netlist).unwrap();
        let cap = circuit.netlist.part_id(&Name::new("c_c").unwrap()).unwrap();
        let charge = circuit.layout.charge_index(cap, 0);
        assert_ne!(charge, ChargeIndex::new(cap.index()));
        let previous = Solution::from(vec![2.0, 0.0, 5e-4]);

        let solved = |stiff: &[Store]| -> f64 {
            let conditions = Conditions {
                history: History::Step(Integration {
                    method: Method::Trapezoidal,
                    stiff,
                    h_ps: 1_000_000_000,
                    previous: &previous,
                }),
                ..Conditions::NOMINAL
            };
            let iterate = Iterate::initial(&circuit);
            let mut system = System::new(&circuit.layout);
            system.reset(&circuit.layout, conditions.gmin);
            for (id, part) in circuit.netlist.parts() {
                crate::element::Device::stamp(
                    &part.model,
                    &mut Stamp::new(&mut system, &circuit.layout, id, &iterate, &conditions),
                );
            }
            let solution = system.solve().expect("the held source is nonsingular");
            solution.get(circuit.layout.branch_row(cap, 0))
        };

        let at_v0 = |method: Method| {
            let (g, rhs) = companion(1e-6, method, 1e-3, 2.0, 5e-4);
            g * 2.0 + rhs
        };
        let backward = solved(&[Store::Charge(charge)]);
        let trapezoidal = solved(&[]);
        let by_part = solved(&[Store::Charge(ChargeIndex::new(cap.index()))]);
        assert!(
            (backward - at_v0(Method::BackwardEuler)).abs() < 1e-9,
            "{backward}"
        );
        assert!(
            (trapezoidal - at_v0(Method::Trapezoidal)).abs() < 1e-9,
            "{trapezoidal}"
        );
        assert!((by_part - trapezoidal).abs() < 1e-12, "{by_part}");
        assert!((backward - trapezoidal).abs() > 1e-4);
    }
}

/// A capacitance, with an optional series resistance. With none, the capacitance runs from `a`
/// to `b`. With one, the resistance runs from `a` to an internal node and the capacitance from
/// that node to `b`.
#[derive(Clone, Debug, PartialEq)]
pub struct Capacitor {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Farad, greater than 0.
    pub capacitance: Value,
    /// Volt, any finite value. The operating point holds the voltage across the capacitance here
    /// and solves the current through it. `None` is an explicit `"initial": null`, open at the
    /// operating point.
    pub initial: Option<Value>,
    /// Volt, at least 0. `V(a) - V(b)` below minus this warns `rating.voltage`. 0 warns on any
    /// reverse voltage, and a pin sitting on pin `b` is inside. `None` is no `ratings` object,
    /// or an explicit `"reverse_voltage": null`.
    pub reverse_voltage: Option<Value>,
    /// Ohm, greater than 0. Between pin `a` and the capacitance. `None` is no series resistance.
    pub series_resistance: Option<Value>,
}

impl Capacitor {
    pub const PINS: [&'static str; 2] = ["a", "b"];

    /// The pins, or the internal node and pin `b` when a series resistance is set.
    fn dielectric(&self) -> [StoreEnd; 2] {
        match self.series_resistance {
            None => self.pins.map(StoreEnd::Net),
            Some(_) => [StoreEnd::Internal(0), StoreEnd::Net(self.pins[1])],
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireCapacitor {
    pub(crate) name: String,
    pins: PinMap,
    capacitance: WireValue,
    #[serde(deserialize_with = "Option::deserialize")]
    initial: Option<WireValue>,
    /// Absent on a capacitor written before reverse voltage was rated.
    #[serde(default)]
    ratings: Option<WireCapacitorRatings>,
    /// Absent on a capacitor with no series resistance. The capacitance then joins `a` to `b`.
    #[serde(default)]
    series_resistance: Option<WireValue>,
}

/// The key is required, and `null` leaves the capacitor unrated.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCapacitorRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    reverse_voltage: Option<WireValue>,
}

pub(crate) fn resolve_capacitor(ctx: ParseCtx<'_>, wire: WireCapacitor) -> Option<Capacitor> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let WireCapacitor {
        pins,
        capacitance,
        initial,
        ratings,
        series_resistance,
        ..
    } = wire;

    let pins = resolve_pins(name, Capacitor::PINS, &pins.0, nets, out);
    let capacitance = resolve_value(part_ref, "capacitance", "F", true, capacitance, out);
    let initial = match initial {
        None => Some(None),
        Some(initial) => resolve_value(part_ref, "initial", "V", false, initial, out).map(Some),
    };
    // 0 is a continuous reverse limit of none. Any reverse voltage then warns, and 0 V is inside.
    let reverse_voltage = match ratings {
        None => Some(None),
        Some(ratings) => match ratings.reverse_voltage {
            None => Some(None),
            Some(value) => bounded(
                part_ref,
                "ratings.reverse_voltage",
                "V",
                value,
                |number| (number < 0.0).then_some(ValueRule::NonNegative),
                out,
            )
            .map(Some),
        },
    };
    let series_resistance = match series_resistance {
        None => Some(None),
        Some(series_resistance) => resolve_value(
            part_ref,
            "series_resistance",
            "ohm",
            true,
            series_resistance,
            out,
        )
        .map(Some),
    };
    match (
        pins,
        capacitance,
        initial,
        reverse_voltage,
        series_resistance,
    ) {
        (
            Some(pins),
            Some(capacitance),
            Some(initial),
            Some(reverse_voltage),
            Some(series_resistance),
        ) => Some(Capacitor {
            pins,
            capacitance,
            initial,
            reverse_voltage,
            series_resistance,
        }),
        _ => None,
    }
}

impl Kind for Capacitor {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: usize::from(self.series_resistance.is_some()),
            branches: 1,
            ..Unknowns::default()
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        vec![OwnedCapacitor {
            k: 0,
            ends: self.dielectric(),
            farad: self.capacitance.si.get(),
            sized_by_kind: false,
        }]
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    /// Its only capacitor is branch 0.
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        // `capacitors_open` is the DC point a rating window starts from. The run's own
        // t = 0 solve holds `initial`.
        let initial = if stamp.conditions.capacitors_open {
            None
        } else {
            self.initial.as_ref().map(|initial| initial.si.get())
        };
        if let Some(esr) = &self.series_resistance {
            stamp.conductance(self.pins[0], stamp.internal(0), 1.0 / esr.si.get());
        }
        stamp_capacitor(
            stamp,
            0,
            self.dielectric(),
            self.capacitance.si.get(),
            initial,
        );
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    /// Open unless the operating point holds it at `initial`.
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        if self.initial.is_some() {
            &[(0, 1)]
        } else {
            &[]
        }
    }
    fn quantities(&self) -> &'static [Quantity] {
        TWO_PIN
    }
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(Observed::two_pin(self.pins, probe.branch(0), probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Other
    }
}

#[cfg(test)]
mod reverse_voltage {
    fn circuit(volts: f64, ratings: &str) -> String {
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
                {{ "name": "c1", "kind": "capacitor", "pins": {{ "a": "n", "b": "gnd" }},
                  "capacitance": {{ "value": 1e-5, "unit": "F" }},
                  "initial": null{ratings} }}
              ]
            }}"#
        )
    }

    fn warnings(json: &str) -> String {
        let results = crate::simulate(&crate::netlist::parse(json.as_bytes()).unwrap());
        results
            .diagnostics()
            .iter()
            .map(|d| serde_json::to_string(d).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_reverse_voltage_past_the_rating_warns_and_the_edge_does_not() {
        let rated = r#", "ratings": { "reverse_voltage": { "value": 1.0, "unit": "V" } }"#;
        let past = warnings(&circuit(-1.1, rated));
        assert!(past.contains("rating.voltage"), "{past}");
        assert!(past.contains("#c1.pin.a"), "{past}");
        assert!(past.contains("-1"), "{past}");
        let edge = warnings(&circuit(-1.0, rated));
        assert!(!edge.contains("rating.voltage"), "{edge}");
        let forward = warnings(&circuit(9.0, rated));
        assert!(!forward.contains("rating.voltage"), "{forward}");
        let bare = warnings(&circuit(-9.0, ""));
        assert!(!bare.contains("rating.voltage"), "{bare}");
        let unrated = warnings(&circuit(
            -9.0,
            r#", "ratings": { "reverse_voltage": null }"#,
        ));
        assert!(!unrated.contains("rating.voltage"), "{unrated}");
    }

    /// A continuous reverse limit of none. 0 V is inside. Any reverse voltage warns.
    #[test]
    fn a_reverse_rating_of_zero_warns_on_any_reverse_voltage() {
        let rated = r#", "ratings": { "reverse_voltage": { "value": 0.0, "unit": "V" } }"#;
        let reverse = warnings(&circuit(-1e-3, rated));
        assert!(reverse.contains("rating.voltage"), "{reverse}");
        let edge = warnings(&circuit(0.0, rated));
        assert!(!edge.contains("rating.voltage"), "{edge}");
        let forward = warnings(&circuit(5.0, rated));
        assert!(!forward.contains("rating.voltage"), "{forward}");
        let rejected = crate::netlist::parse(
            circuit(
                -1.0,
                r#", "ratings": { "reverse_voltage": { "value": -1.0, "unit": "V" } }"#,
            )
            .as_bytes(),
        );
        let err = rejected.expect_err("a negative reverse rating is refused");
        assert!(
            err.iter().any(|d| {
                matches!(
                    d,
                    crate::diagnostic::Diagnostic::NetlistValue {
                        rule: crate::diagnostic::ValueRule::NonNegative,
                        ..
                    }
                )
            }),
            "{err:?}"
        );
    }
}

#[cfg(test)]
mod series_resistance {
    use crate::diagnostic::Diagnostic;
    use crate::name::Ref;
    use crate::run_folder::Results;

    /// Nichicon UVR, 16 V, tan δ max 0.20 at 120 Hz and 20 °C, on the 100 µF the radial model
    /// is drawn as. ESR = tan δ / (2 π f C).
    const ESR_OHM: f64 = 2.65;
    /// The MBR0520 fit's series resistance, the Schottky between USB and the Nano's 5 V rail.
    const DIODE_RS_OHM: f64 = 0.091562;
    const FARAD: f64 = 100e-6;
    const USB_V: f64 = 5.0;

    fn op_of(json: &str) -> Results {
        let netlist = crate::netlist::parse(json.as_bytes()).expect("the netlist parses");
        let results = crate::simulate(&netlist);
        assert!(
            results.observations().op().is_some(),
            "{:?}",
            results.diagnostics()
        );
        results
    }

    fn quantity(results: &Results, reference: &str) -> f64 {
        let op = results.observations().op().expect("an operating point");
        op.values[&reference.parse::<Ref>().unwrap()].get()
    }

    fn held(esr: Option<f64>) -> String {
        let series = esr.map_or(String::new(), |ohm| {
            format!(r#", "series_resistance": {{ "value": {ohm}, "unit": "ohm" }}"#)
        });
        format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": ["gnd", "n"],
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "n", "n": "gnd" }},
                  "voltage": {{ "value": 5.0, "unit": "V" }} }},
                {{ "name": "c1", "kind": "capacitor", "pins": {{ "a": "n", "b": "gnd" }},
                  "capacitance": {{ "value": {FARAD}, "unit": "F" }},
                  "initial": {{ "value": 5.0, "unit": "V" }}{series} }}
              ]
            }}"#
        )
    }

    fn divider(esr: Option<f64>) -> String {
        let series = esr.map_or(String::new(), |ohm| {
            format!(r#", "series_resistance": {{ "value": {ohm}, "unit": "ohm" }}"#)
        });
        format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": ["gnd", "vin", "mid"],
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vin", "n": "gnd" }},
                  "voltage": {{ "value": 5.0, "unit": "V" }} }},
                {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "vin", "b": "mid" }},
                  "resistance": {{ "value": 1000.0, "unit": "ohm" }},
                  "ratings": {{ "power": null }} }},
                {{ "name": "r2", "kind": "resistor", "pins": {{ "a": "mid", "b": "gnd" }},
                  "resistance": {{ "value": 1000.0, "unit": "ohm" }},
                  "ratings": {{ "power": null }} }},
                {{ "name": "c1", "kind": "capacitor", "pins": {{ "a": "mid", "b": "gnd" }},
                  "capacitance": {{ "value": {FARAD}, "unit": "F" }},
                  "initial": null{series} }}
              ]
            }}"#
        )
    }

    /// 5 V through the Nano's Schottky into 100 µF held at 0 V. That hold is a short, so the
    /// current is the inrush peak.
    fn inrush(esr: Option<f64>) -> String {
        let series = esr.map_or(String::new(), |ohm| {
            format!(r#", "series_resistance": {{ "value": {ohm}, "unit": "ohm" }}"#)
        });
        format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": ["gnd", "vbus", "rail"],
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vbus", "n": "gnd" }},
                  "voltage": {{ "value": {USB_V}, "unit": "V" }} }},
                {{ "name": "d1", "kind": "diode", "pins": {{ "anode": "vbus", "cathode": "rail" }},
                  "saturation_current": {{ "value": 2.1308e-06, "unit": "A" }},
                  "emission_coefficient": {{ "value": 0.97642, "unit": "1" }},
                  "series_resistance": {{ "value": {DIODE_RS_OHM}, "unit": "ohm" }},
                  "ratings": {{ "current": {{ "value": 0.5, "unit": "A" }}, "power": null }} }},
                {{ "name": "c1", "kind": "capacitor", "pins": {{ "a": "rail", "b": "gnd" }},
                  "capacitance": {{ "value": {FARAD}, "unit": "F" }},
                  "initial": {{ "value": 0.0, "unit": "V" }}{series} }}
              ]
            }}"#
        )
    }

    /// A capacitor held at the source voltage draws no surge, with or without series resistance.
    #[test]
    fn a_held_capacitor_at_its_initial_voltage_draws_no_surge() {
        for esr in [None, Some(ESR_OHM)] {
            let results = op_of(&held(esr));
            let current = quantity(&results, "#c1.i").abs();
            assert!(
                current < 1e-6,
                "esr {esr:?}: {current} A is a surge, not a hold at the source voltage"
            );
            let volts = quantity(&results, "#c1.v");
            assert!((volts - 5.0).abs() < 1e-9, "esr {esr:?}: {volts} V");
        }
    }

    /// An open capacitor at DC does not move a resistive divider when it gains series resistance.
    #[test]
    fn a_steady_divider_is_unchanged_by_series_resistance() {
        let plain = op_of(&divider(None));
        let with = op_of(&divider(Some(ESR_OHM)));
        for reference in ["#net.mid", "#r1.i", "#c1.i"] {
            let a = quantity(&plain, reference);
            let b = quantity(&with, reference);
            assert!(
                (a - b).abs() < 1e-8,
                "{reference}: {a} without series resistance, {b} with it"
            );
        }
        assert!((quantity(&plain, "#net.mid") - 2.5).abs() < 1e-6);
    }

    /// No series resistance leaves the Schottky's own resistance, and the peak is tens of
    /// amps. The electrolytic's ESR brings it under 5 V / 2.65 ohm.
    #[test]
    fn inrush_through_the_schottky_falls_from_tens_of_amps_to_a_few() {
        let bare = quantity(&op_of(&inrush(None)), "#c1.i");
        assert!(
            bare > 10.0,
            "without ESR the peak is {bare} A, not the tens of amps a shorted capacitor draws"
        );
        let limited = quantity(&op_of(&inrush(Some(ESR_OHM))), "#c1.i");
        assert!(
            bare > 10.0,
            "without ESR the peak is {bare} A, not the tens of amps a shorted capacitor draws"
        );
        let ceiling = USB_V / ESR_OHM;
        assert!(
            limited > 1.0 && limited < ceiling,
            "with {ESR_OHM} ohm the peak is {limited} A, which should sit between 1 A and {ceiling} A"
        );
    }

    #[test]
    fn a_series_resistance_of_zero_is_refused() {
        let json = inrush(Some(0.0));
        let err = crate::netlist::parse(json.as_bytes()).expect_err("zero ESR is refused");
        assert!(
            err.iter().any(|d| {
                matches!(
                    d,
                    Diagnostic::NetlistValue {
                        rule: crate::diagnostic::ValueRule::Positive,
                        ..
                    }
                )
            }),
            "{err:?}"
        );
    }
}
