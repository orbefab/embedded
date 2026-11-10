use crate::diagnostic::Diagnostic;
use crate::name::{Name, Ref};
use crate::netlist::Model;
use crate::netlist::NetId;
use crate::netlist::Value;
use crate::netlist::json::PinMap;
use crate::netlist::json::WireValue;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_value;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::element::StoreEnd;
use crate::mna::{Probe, Stamp};
use crate::newton::{History, Method, Store};

/// Seconds. A held capacitor sits behind `HOLD / C` ohm, the backward Euler companion
/// of a step this long. An exact hold would make a capacitor across a voltage source,
/// or two in parallel, singular. The series resistance shares their current by
/// capacitance, and moves the held voltage by the current times `HOLD / C`.
const HOLD: f64 = 1e-15;

/// The element's only capacitor is branch 0.
pub(crate) fn stamp(capacitor: &Capacitor, stamp: &mut Stamp<'_>) {
    let initial = capacitor.initial.as_ref().map(|initial| initial.si.get());
    stamp_capacitor(
        stamp,
        0,
        capacitor.pins.map(StoreEnd::Net),
        capacitor.capacitance.si.get(),
        initial,
    );
}

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

pub(crate) fn current(probe: &Probe<'_>) -> f64 {
    probe.branch(0)
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
              "version": 0,
              "seed": 0,
              "analysis": { "op": {} },
              "ground": "gnd",
              "nets": ["gnd", "n"],
              "parts": [
                { "name": "a_r", "kind": "resistor", "pins": { "a": "n", "b": "gnd" },
                  "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
                  "ratings": { "power": null } },
                { "name": "b_v", "kind": "voltage_source", "pins": { "p": "n", "n": "gnd" },
                  "voltage": { "value": 2.0, "unit": "V", "source": "user" } },
                { "name": "c_c", "kind": "capacitor", "pins": { "a": "n", "b": "gnd" },
                  "capacitance": { "value": 1e-6, "unit": "F", "source": "user" },
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
                crate::element::stamp(
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

/// An ideal capacitor.
#[derive(Clone, Debug, PartialEq)]
pub struct Capacitor {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Farad, greater than 0.
    pub capacitance: Value,
    /// Volt, any finite value. The operating point holds `V(a) - V(b)` here and solves the
    /// current through it. `None` is an explicit `"initial": null`, open at the operating point.
    pub initial: Option<Value>,
}

impl Capacitor {
    pub const PINS: [&'static str; 2] = ["a", "b"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireCapacitor {
    pub(crate) name: String,
    pins: PinMap,
    capacitance: WireValue,
    // Required but nullable, like the ratings below.
    #[serde(deserialize_with = "Option::deserialize")]
    initial: Option<WireValue>,
}

pub(crate) fn resolve_capacitor(
    name: &Name,
    part_ref: &Ref,
    wire: WireCapacitor,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Model> {
    let WireCapacitor {
        pins,
        capacitance,
        initial,
        ..
    } = wire;

    let pins = resolve_pins(name, Capacitor::PINS, &pins.0, nets, out);
    let capacitance = resolve_value(part_ref, "capacitance", "F", true, capacitance, out);
    let initial = match initial {
        None => Some(None),
        Some(initial) => resolve_value(part_ref, "initial", "V", false, initial, out).map(Some),
    };
    match (pins, capacitance, initial) {
        (Some(pins), Some(capacitance), Some(initial)) => Some(Model::Capacitor(Capacitor {
            pins,
            capacitance,
            initial,
        })),
        _ => None,
    }
}
