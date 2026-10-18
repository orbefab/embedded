use sim_core::netlist::Capacitor;

use crate::mna::{Probe, Stamp};
use crate::newton::{History, Method};

/// Seconds. A held capacitor sits behind `HOLD / C` ohm, the backward Euler companion
/// of a step this long. An exact hold would make a capacitor across a voltage source,
/// or two in parallel, singular. The series resistance shares their current by
/// capacitance, and moves the held voltage by the current times `HOLD / C`.
const HOLD: f64 = 1e-15;

/// Branch 0 is the current entering pin `a`. A held voltage scales with the sources.
/// At t = 0 a capacitor is held at `initial`, or open with 0 current. At a restart it
/// is held at its voltage in the solution before the edge. In a step it follows the
/// companion, with `v0` and `i0` read from the last accepted solution.
pub(super) fn stamp(capacitor: &Capacitor, stamp: &mut Stamp<'_>) {
    let [a, b] = capacitor.pins;
    let capacitance = capacitor.capacitance.si.get();
    let scale = stamp.conditions.source_scale;
    stamp.branch_current(a, b, 0);
    let held = |stamp: &mut Stamp<'_>, volts: f64| {
        stamp.branch_row(a, b, 0, -HOLD / capacitance, 1.0, volts * scale);
    };
    match stamp.conditions.history {
        History::Start => match &capacitor.initial {
            None => stamp.branch_row(a, b, 0, 1.0, 0.0, 0.0),
            Some(initial) => held(stamp, initial.si.get()),
        },
        History::Restart { previous } => held(stamp, previous.voltage(a) - previous.voltage(b)),
        History::Step(step) => {
            let previous = stamp.probe(step.previous);
            let v0 = previous.v(a) - previous.v(b);
            let method = step.method_of(stamp.part);
            let (g, rhs) = companion(capacitance, method, step.h(), v0, previous.branch(0));
            stamp.branch_row(a, b, 0, 1.0, -g, rhs);
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

pub(super) fn current(probe: &Probe<'_>) -> f64 {
    probe.branch(0)
}

#[cfg(test)]
mod tests {
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
}
