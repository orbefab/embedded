//! The step policy. Pure, like the ladder: each function sees one outcome and
//! names the next plan.
//!
//! A segment starts at t = 0 and at every restart solve whose jump on a capacitor or an
//! inductor exceeds Newton's tolerance on that row, and its first step is `EDGE_STEP`.
//! From then on the local truncation error sets it. An accepted step grows by
//! `0.9 * ratio^(-1/(p+1))`, at most 2, and a rejected one shrinks by the same rule,
//! between 0.25 and 0.9. p is 1 for backward Euler and 2 for trapezoidal.

use crate::netlist::Transient;
use crate::time::Time;

use crate::newton::Method;

/// The first step of a segment, in ps. It must equal `Transient::MIN_MAX_STEP_PS`.
pub(crate) const EDGE_STEP: u64 = 1_000;

const _: () = assert!(EDGE_STEP == Transient::MIN_MAX_STEP_PS);

const SAFETY: f64 = 0.9;
const MOST_GROWTH: f64 = 2.0;
const MOST_SHRINK: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// The step wanted before it is cut to the next breakpoint. 1 to `max_step`.
    pub(crate) h: u64,
}

impl Plan {
    pub(crate) const RESTART: Plan = Plan { h: EDGE_STEP };

    /// After an accepted step of `tried` ps whose error was `ratio` of its tolerance.
    /// A ratio of 0 doubles. A factor over 1 grows by at least 1 ps, so a step can leave
    /// the 1 ps floor. Capped at `max_step`.
    pub(crate) fn accepted(tried: u64, ratio: f64, method: Method, max_step: u64) -> Plan {
        let factor = factor(ratio, method).min(MOST_GROWTH);
        let h = scaled(tried, factor);
        let h = if factor > 1.0 { h.max(tried + 1) } else { h };
        Plan { h: h.min(max_step) }
    }

    /// After a step of `tried` ps whose error was `ratio` of its tolerance, over 1.
    pub(crate) fn rejected(tried: u64, ratio: f64, method: Method) -> Plan {
        let factor = factor(ratio, method).clamp(MOST_SHRINK, SAFETY);
        Plan {
            h: scaled(tried, factor),
        }
    }

    /// After Newton stalled on a step of `tried` ps: half of it.
    /// `None` when `tried` was 1 ps.
    pub(crate) fn halve(tried: u64) -> Option<Plan> {
        (tried > 1).then_some(Plan { h: tried / 2 })
    }

    /// The step to take from `now`. `next` is the next breakpoint, after `now`.
    pub(crate) fn clip(self, now: Time, next: Time) -> u64 {
        self.h.min(next.as_ps() - now.as_ps())
    }
}

/// `SAFETY * ratio^(-1/(p+1))`. Infinite at a ratio of 0.
fn factor(ratio: f64, method: Method) -> f64 {
    let root = match method {
        Method::BackwardEuler => ratio.sqrt(),
        Method::Trapezoidal => libm::cbrt(ratio),
    };
    SAFETY / root
}

/// `tried * factor`, floored to whole picoseconds, at least 1.
fn scaled(tried: u64, factor: f64) -> u64 {
    ((tried as f64 * factor) as u64).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u64 = 20_000_000;

    #[test]
    fn no_error_doubles_up_to_max_step() {
        let mut h = EDGE_STEP;
        let mut steps = vec![h];
        while h < MAX {
            h = Plan::accepted(h, 0.0, Method::Trapezoidal, MAX).h;
            steps.push(h);
        }
        assert_eq!(steps.len(), 16, "{steps:?}");
        assert_eq!(steps[14], 16_384_000);
        assert_eq!(steps[15], MAX);
        assert_eq!(Plan::accepted(MAX, 0.0, Method::BackwardEuler, MAX).h, MAX);
    }

    #[test]
    fn an_accepted_step_grows_by_the_root_of_its_order() {
        // 0.9 / sqrt(0.36) = 1.5, and 0.9 / cbrt(0.125) = 1.8.
        assert_eq!(
            Plan::accepted(1_000, 0.36, Method::BackwardEuler, MAX).h,
            1_500
        );
        assert_eq!(
            Plan::accepted(1_000, 0.125, Method::Trapezoidal, MAX).h,
            1_800
        );
        // A step right at its tolerance shrinks by the safety factor.
        assert_eq!(Plan::accepted(1_000, 1.0, Method::Trapezoidal, MAX).h, 900);
    }

    #[test]
    fn an_accepted_step_that_may_grow_grows_by_at_least_one_picosecond() {
        // 0.9 / cbrt(0.5) is about 1.13, and 0.9 / cbrt(0.7) about 1.01.
        assert_eq!(Plan::accepted(1, 0.5, Method::Trapezoidal, MAX).h, 2);
        assert_eq!(Plan::accepted(10, 0.7, Method::Trapezoidal, MAX).h, 11);
        // 0.9 / sqrt(0.9) is under 1, so the step shrinks.
        assert_eq!(Plan::accepted(10, 0.9, Method::BackwardEuler, MAX).h, 9);
        assert_eq!(Plan::accepted(1, 1.0, Method::BackwardEuler, MAX).h, 1);
    }

    #[test]
    fn a_rejected_step_shrinks_between_a_quarter_and_nine_tenths() {
        // 0.9 / sqrt(4) = 0.45.
        assert_eq!(Plan::rejected(1_000, 4.0, Method::BackwardEuler).h, 450);
        assert_eq!(Plan::rejected(1_000, 1e12, Method::Trapezoidal).h, 250);
        assert_eq!(
            Plan::rejected(1_000, 1.0 + 1e-12, Method::Trapezoidal).h,
            899
        );
        assert_eq!(Plan::rejected(3, 1e12, Method::BackwardEuler).h, 1);
    }

    #[test]
    fn halving_stops_at_one_picosecond() {
        assert_eq!(Plan::halve(3), Some(Plan { h: 1 }));
        assert_eq!(Plan::halve(1), None);
        let mut tried = EDGE_STEP;
        let mut attempts = 1;
        while let Some(plan) = Plan::halve(tried) {
            tried = plan.h;
            attempts += 1;
        }
        assert_eq!((tried, attempts), (1, 10));
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
