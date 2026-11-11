use orbe_model::Time;
use serde::Deserialize;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, StoreEnd, TWO_PIN, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::{ParseCtx, PinMap, WireValue, resolve_pins, resolve_value};
use crate::netlist::{Conduction, Kind, NetId, Value};
use crate::newton::{History, Method, Side, Store};

/// Seconds. A held inductor sits beside `HOLD / L` siemens, the backward Euler companion of a
/// step this long, the dual of a held capacitor's `HOLD / C` ohm. An exact hold would leave two
/// inductors in series with nothing but gmin to share a difference in their currents.
const HOLD: f64 = 1e-15;

/// A coil that ends at `b`, the part's only store, so its branch is 0. `stamp_coil` puts `ohm`
/// from the coil's other end to internal node 0, and the inductance from there to `b`. An
/// inductor is one, and so is a relay's coil.
pub(crate) fn coil(b: NetId, henry: f64, ohm: f64) -> OwnedInductor {
    OwnedInductor {
        k: 0,
        ends: [StoreEnd::Internal(0), StoreEnd::Net(b)],
        henry,
        ohm,
    }
}

fn owned(inductor: &Inductor) -> OwnedInductor {
    coil(
        inductor.pins[1],
        inductor.inductance.si.get(),
        inductor.series_resistance.si.get(),
    )
}

/// `coil`'s resistance from `a` to internal node 0, and its inductance.
pub(crate) fn stamp_coil(stamp: &mut Stamp<'_>, a: NetId, coil: OwnedInductor) {
    let x = stamp.internal(0);
    stamp.conductance(a, x, 1.0 / coil.ohm);
    stamp_inductor(stamp, coil.k, coil.ends, coil.henry);
}

/// Stamp inductor `k` from `ends[0]` to `ends[1]`. Its branch, the part's after its capacitors',
/// is the current entering `ends[0]`. The operating point is a short. A restart holds the current
/// in the solution before the edge, scaled with the sources. A step follows the companion. `v0`
/// and `i0` come from the last accepted solution.
pub(crate) fn stamp_inductor(stamp: &mut Stamp<'_>, k: usize, ends: [StoreEnd; 2], henry: f64) {
    let [x, b] = ends.map(|end| end.node(stamp.layout, stamp.part));
    let branch = stamp.layout.flux_branch(stamp.part, k);
    stamp.branch_current(x, b, branch);
    match stamp.conditions.history {
        History::Start => stamp.branch_row(x, b, branch, 0.0, 1.0, 0.0),
        History::Restart { previous } => {
            let i0 = stamp.probe(previous).branch(branch);
            let scale = stamp.conditions.source_scale;
            stamp.branch_row(x, b, branch, 1.0, -HOLD / henry, i0 * scale);
        }
        History::Step(step) => {
            let previous = stamp.probe(step.previous);
            let v0 = previous.v(x) - previous.v(b);
            let index = stamp.layout.flux_index(stamp.part, k);
            let method = step.method_of(Store::Flux(index));
            let (g, rhs) = companion(henry, method, step.h(), v0, previous.branch(branch));
            stamp.branch_row(x, b, branch, 1.0, -g, rhs);
        }
    }
}

/// The step's companion `i = g * v + rhs`, as `(g, rhs)`.
fn companion(henry: f64, method: Method, h: f64, v0: f64, i0: f64) -> (f64, f64) {
    match method {
        Method::BackwardEuler => (h / henry, i0),
        Method::Trapezoidal => {
            let g = h / (2.0 * henry);
            (g, i0 + g * v0)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::name::Name;
    use crate::netlist;

    use super::*;
    use crate::mna::{Solution, System};
    use crate::newton::{Circuit, Conditions, Iterate};

    #[test]
    fn backward_euler_is_the_voltage_over_the_step() {
        let (g, rhs) = companion(0.5, Method::BackwardEuler, 1e-3, 2.0, 5e-3);
        assert_eq!(g, 2e-3);
        assert_eq!(g * 3.0 + rhs, 5e-3 + 1e-3 * 3.0 / 0.5);
    }

    #[test]
    fn trapezoidal_averages_the_two_voltages() {
        let (g, rhs) = companion(0.5, Method::Trapezoidal, 1e-3, 2.0, 5e-3);
        let i1 = g * 3.0 + rhs;
        assert!(
            (i1 - (5e-3 + 1e-3 * (2.0 + 3.0) / 2.0 / 0.5)).abs() <= 1e-18,
            "{i1}"
        );
    }

    /// 5 V onto 1 nH behind 70 ohm. `history` sees `previous`, with 50 mA in the coil.
    /// Returns the coil's current and the voltage across its inductance.
    fn solved(history: impl Fn(&Solution) -> History<'_>) -> (f64, f64) {
        let netlist = netlist::parse(
            br#"{
              "version": 0,
              "seed": 0,
              "analysis": { "op": {} },
              "ground": "gnd",
              "nets": ["gnd", "vin"],
              "parts": [
                { "name": "v1", "kind": "voltage_source", "pins": { "p": "vin", "n": "gnd" },
                  "voltage": { "value": 5.0, "unit": "V", "source": "user" } },
                { "name": "l1", "kind": "inductor", "pins": { "a": "vin", "b": "gnd" },
                  "inductance": { "value": 1e-9, "unit": "H", "source": "user" },
                  "series_resistance": { "value": 70.0, "unit": "ohm", "source": "user" } }
              ]
            }"#,
        )
        .unwrap();
        let circuit = Circuit::new(netlist).unwrap();
        let layout = &circuit.layout;
        let l1 = circuit.netlist.part_id(&Name::new("l1").unwrap()).unwrap();
        let mut previous = vec![0.0; layout.size()];
        previous[layout.branch_row(l1, 0)] = 0.05;
        let previous = Solution::from(previous);
        let conditions = Conditions {
            history: history(&previous),
            ..Conditions::NOMINAL
        };
        let iterate = Iterate::initial(&circuit);
        let mut system = System::new(layout);
        system.reset(layout, conditions.gmin);
        for (id, part) in circuit.netlist.parts() {
            crate::element::Device::stamp(
                &part.model,
                &mut Stamp::new(&mut system, layout, id, &iterate, &conditions),
            );
        }
        let solution = system.solve().expect("the coil is nonsingular");
        (
            solution.get(layout.branch_row(l1, 0)),
            solution.node(layout.internal(l1, 0)),
        )
    }

    /// The restart keeps the current behind `HOLD / L` siemens, so it moves by `HOLD * v / L`,
    /// toward where the voltage drives it. The new source does not set it.
    #[test]
    fn held_current_across_a_restart() {
        let (i, v) = solved(|previous| History::Restart { previous });
        assert!((v - (5.0 - 70.0 * 0.05)).abs() < 1e-3, "{v}");
        let moved = HOLD * v / 1e-9;
        assert!(moved > 1e-6, "{moved}");
        assert!(((i - 0.05) - moved).abs() <= 1e-6 * moved, "{i} {v}");
    }

    #[test]
    fn operating_point_is_a_short_through_r() {
        let (i, v) = solved(|_| History::Start);
        assert!((i - 5.0 / 70.0).abs() <= 1e-12, "{i}");
        assert!(v.abs() <= 1e-12, "{v}");
    }
}

/// An ideal inductance in series with a resistance. `a` reaches the inductance through the
/// resistance. A short behind the resistance at the operating point.
#[derive(Clone, Debug, PartialEq)]
pub struct Inductor {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Henry, greater than 0.
    pub inductance: Value,
    /// Ohm, greater than 0.
    pub series_resistance: Value,
}

impl Inductor {
    pub const PINS: [&'static str; 2] = ["a", "b"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireInductor {
    pub(crate) name: String,
    pins: PinMap,
    inductance: WireValue,
    series_resistance: WireValue,
}

pub(crate) fn resolve_inductor(ctx: ParseCtx<'_>, wire: WireInductor) -> Option<Inductor> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let WireInductor {
        pins,
        inductance,
        series_resistance,
        ..
    } = wire;

    let pins = resolve_pins(name, Inductor::PINS, &pins.0, nets, out);
    let inductance = resolve_value(part_ref, "inductance", "H", true, inductance, out);
    let series_resistance = resolve_value(
        part_ref,
        "series_resistance",
        "ohm",
        true,
        series_resistance,
        out,
    );
    match (pins, inductance, series_resistance) {
        (Some(pins), Some(inductance), Some(series_resistance)) => Some(Inductor {
            pins,
            inductance,
            series_resistance,
        }),
        _ => None,
    }
}

impl Kind for Inductor {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 1,
            branches: 1,
            ..Unknowns::default()
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        vec![owned(self)]
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        stamp_coil(stamp, self.pins[0], owned(self));
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    /// A short behind its resistance.
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
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
