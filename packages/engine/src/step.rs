//! The step policy. Pure, like the ladder: each function sees one outcome and
//! names the next plan.
//!
//! A segment starts at t = 0 and at every restart solve. Its first step is `EDGE_STEP`, or
//! after a chip's or a core's edge the time its stores take to move their tolerance at their
//! slopes when that is longer. From then on the local truncation error sets it. An accepted
//! step grows by `0.9 * ratio^(-1/(p+1))`, at most 2, and a rejected one shrinks by the same
//! rule, between 0.25 and 0.9. p is the order of the step's formula, 1 for backward Euler and
//! 2 for BDF2.

use orbe_model::Time;

use crate::netlist::Transient;

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
    /// A ratio of 0 doubles. A factor over 1 that rounds to no growth grows by 1 ps when the
    /// error predicted at that step is within the tolerance, so a step can leave the 1 ps floor
    /// without being rejected. Capped at `max_step`.
    pub(crate) fn accepted(tried: u64, ratio: f64, order: u32, max_step: u64) -> Plan {
        let factor = factor(ratio, order).min(MOST_GROWTH);
        let h = scaled(tried, factor);
        // `factor / SAFETY` is the growth that predicts the error right at the tolerance.
        let h = if factor > 1.0 && (tried + 1) as f64 <= tried as f64 * factor / SAFETY {
            h.max(tried + 1)
        } else {
            h
        };
        Plan { h: h.min(max_step) }
    }

    /// The first step of a segment a chip's or a core's edge starts, from `estimate` ps, the
    /// time its stores take to move their tolerance at their slopes. At least `EDGE_STEP`: a
    /// rejection shrinks a step by up to 4 and an accepted step grows by at most 2, so a
    /// first step too long costs fewer solves than one too short. At most `max_step`.
    pub(crate) fn driven(estimate: u64, max_step: u64) -> Plan {
        Plan {
            h: estimate.clamp(EDGE_STEP, max_step),
        }
    }

    /// After a step of `tried` ps whose error was `ratio` of its tolerance, over 1.
    pub(crate) fn rejected(tried: u64, ratio: f64, order: u32) -> Plan {
        let factor = factor(ratio, order).clamp(MOST_SHRINK, SAFETY);
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

/// `SAFETY * ratio^(-1/(p+1))`, `p` being `order`, 1 or 2. Infinite at a ratio of 0.
fn factor(ratio: f64, order: u32) -> f64 {
    let root = match order {
        1 => ratio.sqrt(),
        _ => libm::cbrt(ratio),
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
            h = Plan::accepted(h, 0.0, 2, MAX).h;
            steps.push(h);
        }
        assert_eq!(steps.len(), 16, "{steps:?}");
        assert_eq!(steps[14], 16_384_000);
        assert_eq!(steps[15], MAX);
        assert_eq!(Plan::accepted(MAX, 0.0, 1, MAX).h, MAX);
    }

    #[test]
    fn an_accepted_step_grows_by_the_root_of_its_order() {
        // 0.9 / sqrt(0.36) = 1.5, and 0.9 / cbrt(0.125) = 1.8.
        assert_eq!(Plan::accepted(1_000, 0.36, 1, MAX).h, 1_500);
        assert_eq!(Plan::accepted(1_000, 0.125, 2, MAX).h, 1_800);
        // A step right at its tolerance shrinks by the safety factor.
        assert_eq!(Plan::accepted(1_000, 1.0, 2, MAX).h, 900);
    }

    #[test]
    fn an_accepted_step_that_may_grow_grows_by_at_least_one_picosecond() {
        // 0.9 / cbrt(0.7) is about 1.01, and 11 ps predicts 0.7 * 1.1^3, about 0.93.
        assert_eq!(Plan::accepted(10, 0.7, 2, MAX).h, 11);
        // 0.9 / cbrt(0.1) is about 1.94, and 2 ps predicts 0.1 * 8 = 0.8.
        assert_eq!(Plan::accepted(1, 0.1, 2, MAX).h, 2);
        // 0.9 / sqrt(0.9) is under 1, so the step shrinks.
        assert_eq!(Plan::accepted(10, 0.9, 1, MAX).h, 9);
        assert_eq!(Plan::accepted(1, 1.0, 1, MAX).h, 1);
    }

    /// 1 ps more is 2 times a 1 ps step and 1.5 times a 2 ps one, so the error it predicts is
    /// 8 and 3.4 times the last on BDF2. Taken over the tolerance it would be rejected.
    #[test]
    fn a_picosecond_more_waits_until_its_predicted_error_fits() {
        // 0.9 / cbrt(0.5) is about 1.13, but 2 ps predicts 4 and 3 ps from 2 ps 1.7.
        assert_eq!(Plan::accepted(1, 0.5, 2, MAX).h, 1);
        assert_eq!(Plan::accepted(2, 0.5, 2, MAX).h, 2);
        // 3 ps from 2 ps predicts 0.25 * 3.4, about 0.84.
        assert_eq!(Plan::accepted(2, 0.25, 2, MAX).h, 3);
        // Backward Euler: 2 ps from 1 ps predicts 0.3 * 4 = 1.2, and 0.2 * 4 = 0.8.
        assert_eq!(Plan::accepted(1, 0.3, 1, MAX).h, 1);
        assert_eq!(Plan::accepted(1, 0.2, 1, MAX).h, 2);
    }

    #[test]
    fn a_rejected_step_shrinks_between_a_quarter_and_nine_tenths() {
        // 0.9 / sqrt(4) = 0.45.
        assert_eq!(Plan::rejected(1_000, 4.0, 1).h, 450);
        assert_eq!(Plan::rejected(1_000, 1e12, 2).h, 250);
        assert_eq!(Plan::rejected(1_000, 1.0 + 1e-12, 2).h, 899);
        assert_eq!(Plan::rejected(3, 1e12, 1).h, 1);
    }

    #[test]
    fn a_driven_edge_s_first_step_is_its_estimate_from_1_ns_to_max_step() {
        assert_eq!(Plan::driven(1, MAX).h, EDGE_STEP);
        assert_eq!(Plan::driven(250_000, MAX).h, 250_000);
        assert_eq!(Plan::driven(u64::MAX, MAX).h, MAX);
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
