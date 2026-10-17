use sim_core::netlist::Capacitor;

use crate::mna::{Probe, Stamp};
use crate::newton::Method;

/// Branch 0 is the current entering pin `a`. At the operating point it is 0.
/// In a step it follows the companion, with `v0` and `i0` read from the last
/// accepted solution.
pub(super) fn stamp(capacitor: &Capacitor, stamp: &mut Stamp<'_>) {
    let [a, b] = capacitor.pins;
    stamp.branch_current(a, b, 0);
    let history = stamp
        .conditions
        .step
        .zip(stamp.previous())
        .map(|(step, previous)| {
            let v0 = previous.v(a) - previous.v(b);
            let capacitance = capacitor.capacitance.si.get();
            companion(capacitance, step.method, step.h(), v0, previous.branch(0))
        });
    match history {
        None => stamp.branch_row(a, b, 0, 1.0, 0.0, 0.0),
        Some((g, rhs)) => stamp.branch_row(a, b, 0, 1.0, -g, rhs),
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
