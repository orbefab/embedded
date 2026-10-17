//! The step policy. Pure, like the ladder: each function sees one plan and
//! an outcome and names the next plan.
//!
//! After t = 0 and after every contact edge, the step restarts at
//! `EDGE_STEP` with backward Euler and doubles per accepted step. Once it
//! reaches `max_step` it switches to the trapezoidal rule. One backward Euler
//! step before trapezoidal would leave a stiff mode ringing undamped.

use sim_core::netlist::Transient;
use sim_core::time::Time;

use crate::newton::Method;

/// The first step after a discontinuity, in ps. It must equal `Transient::MIN_MAX_STEP_PS`.
pub(crate) const EDGE_STEP: u64 = 1_000;

const _: () = assert!(EDGE_STEP == Transient::MIN_MAX_STEP_PS);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// The step wanted before it is cut to the next breakpoint. 1 to `max_step`.
    pub(crate) h: u64,
    pub(crate) method: Method,
}

impl Plan {
    /// After t = 0 or a contact edge.
    pub(crate) const RESTART: Plan = Plan {
        h: EDGE_STEP,
        method: Method::BackwardEuler,
    };

    /// After an accepted step: double up to `max_step`. Backward Euler holds until
    /// the step reaches `max_step`, and the trapezoidal rule holds from then on.
    pub(crate) fn grow(self, max_step: u64) -> Plan {
        let h = self.h.saturating_mul(2).min(max_step);
        let method = match self.method {
            Method::BackwardEuler if h < max_step => Method::BackwardEuler,
            _ => Method::Trapezoidal,
        };
        Plan { h, method }
    }

    /// After Newton stalled on a step of `tried` ps: half of it, same method.
    /// `None` when `tried` was 1 ps.
    pub(crate) fn halve(self, tried: u64) -> Option<Plan> {
        (tried > 1).then_some(Plan {
            h: tried / 2,
            ..self
        })
    }

    /// The step to take from `now`. `next` is the next breakpoint, after `now`.
    pub(crate) fn clip(self, now: Time, next: Time) -> u64 {
        self.h.min(next.as_ps() - now.as_ps())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u64 = 20_000_000;

    #[test]
    fn the_ramp_doubles_with_backward_euler_then_turns_trapezoidal_at_max_step() {
        let mut plan = Plan::RESTART;
        let mut steps = vec![(plan.h, plan.method)];
        while plan.method == Method::BackwardEuler {
            plan = plan.grow(MAX);
            steps.push((plan.h, plan.method));
        }
        assert_eq!(steps.len(), 16, "{steps:?}");
        assert_eq!(steps[1], (2_000, Method::BackwardEuler));
        assert_eq!(steps[14], (16_384_000, Method::BackwardEuler));
        assert_eq!(steps[15], (MAX, Method::Trapezoidal));
        assert_eq!(plan.grow(MAX), plan);
    }

    #[test]
    fn a_max_step_of_one_edge_step_is_trapezoidal_after_one_step() {
        let plan = Plan::RESTART.grow(EDGE_STEP);
        assert_eq!(
            plan,
            Plan {
                h: EDGE_STEP,
                method: Method::Trapezoidal,
            }
        );
    }

    #[test]
    fn halving_keeps_the_method_and_stops_at_one_picosecond() {
        let trapezoidal = Plan {
            h: MAX,
            method: Method::Trapezoidal,
        };
        assert_eq!(
            trapezoidal.halve(3),
            Some(Plan {
                h: 1,
                method: Method::Trapezoidal,
            })
        );
        assert_eq!(trapezoidal.halve(1), None);
        let mut tried = EDGE_STEP;
        let mut attempts = 1;
        while let Some(plan) = Plan::RESTART.halve(tried) {
            tried = plan.h;
            attempts += 1;
        }
        assert_eq!((tried, attempts), (1, 10));
    }

    #[test]
    fn a_trapezoidal_plan_regrows_without_going_back_to_backward_euler() {
        let halved = Plan {
            h: MAX / 4,
            method: Method::Trapezoidal,
        };
        assert_eq!(halved.grow(MAX).method, Method::Trapezoidal);
        assert_eq!(halved.grow(MAX).grow(MAX).h, MAX);
    }

    #[test]
    fn clip_lands_on_the_next_breakpoint() {
        let plan = Plan::RESTART;
        assert_eq!(plan.clip(Time::from_ps(10), Time::from_ps(400)), 390);
        assert_eq!(
            plan.clip(Time::from_ps(10), Time::from_ps(5_000)),
            EDGE_STEP
        );
    }
}
