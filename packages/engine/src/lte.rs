//! The integrator's memory and its local truncation error. Pure: solutions in, a formula and an
//! error out.
//!
//! Every capacitor and inductor is a [`Store`]: a state that moves at a slope the rest of the
//! circuit sets. A step is backward Euler from a segment's start, t = 0 or a restart solve, and
//! the variable-step BDF2 formula after it. Both are L-stable, so a mode much faster than the
//! step decays within it, and a slow one keeps its amplitude to within BDF2's own damping,
//! `(w h)^4 / 4` a step.
//!
//! The error is Milne's: the step's state against a predictor of the same order through the
//! points before it, scaled by the ratio of the two formulas' error constants.

use orbe_model::Time;

use crate::netlist::{Model, Netlist};

use crate::element;
use crate::mna::{Layout, Node, Solution};
use crate::newton::{ABSTOL, Integration, RELTOL, VNTOL};

/// The share of its range a store's state may be off by on one step. On a decay BDF2's error
/// has one sign on every step, so the errors of the few hundred steps of a swing add up. At this
/// share a 10 ns RC edge stays within 1e-5 of its swing.
const SHARE: f64 = 1e-7;

/// The charge a device's capacitance may be off by on one step, as SPICE's `chgtol`, up to
/// [`DEVICE_SHARE`] of its range. A junction or a gate of a few picofarads that swings is then
/// held to its charge and not to Newton's 1 nV.
const CHGTOL: f64 = 2e-16;

/// The share of its range a device's capacitance may be off by on one step, when its charge
/// floor is more. A 1 fF gate driven through 1 Gohm swings 5 V, and its charge floor of 0.2 V a
/// step left the ngspice reference by 80 times the bound. At this share, 30 times [`SHARE`], every
/// ngspice fixture holds its bound, the 1 fF gate an NE555 drives at 0.79 of it, and a BSS138's
/// few picofarads still take steps of nanoseconds.
///
/// The share is of the store's own range, as the bound is of each reading's own span. A floor
/// in volts on a store that has barely moved left a 1 fF gate and a 100 pF one that swing
/// 1.5 mV outside the bound, and a floor that is a share of the level left a 1.5 mV swing at
/// 5 V outside it. So an idle junction that ripples a few microvolts is held to about 1 nV.
const DEVICE_SHARE: f64 = 3e-6;

/// A capacitor or an inductor. A capacitor's state is the voltage across it, and its slope
/// times its capacitance is its current. An inductor's state is its current, and its slope times
/// its inductance is the voltage across it.
struct Store {
    /// The nodes the state or the slope is the voltage across.
    ends: [Node; 2],
    /// The branch row of the current.
    branch: usize,
    /// Farad or henry.
    size: f64,
    inductor: bool,
    /// The charge floor: `CHGTOL` on a device's capacitance, and 0 on a capacitor part and an
    /// inductor. A capacitor part's voltage and current are what a user reads, and at a charge
    /// floor a 1 pF RC edge's current leaves its closed form by more than 1e-4 of its swing.
    charge: f64,
}

impl Store {
    fn volts(&self, x: &Solution) -> f64 {
        x.node(self.ends[0]) - x.node(self.ends[1])
    }

    fn state(&self, x: &Solution) -> f64 {
        match self.inductor {
            false => self.volts(x),
            true => x.get(self.branch),
        }
    }

    fn slope(&self, x: &Solution) -> f64 {
        self.flow(x) / self.size
    }

    /// The slope times the size: a capacitor's current or the voltage across an inductor.
    fn flow(&self, x: &Solution) -> f64 {
        match self.inductor {
            false => x.get(self.branch),
            true => self.volts(x),
        }
    }

    /// Newton's tolerance on the flow at `x`. A branch row is settled to `RELTOL` of its
    /// current plus `ABSTOL`, and each end on a row to `RELTOL` of its voltage plus `VNTOL`.
    fn flow_resolved(&self, x: &Solution) -> f64 {
        match self.inductor {
            false => RELTOL * x.get(self.branch).abs() + ABSTOL,
            true => self
                .ends
                .iter()
                .map(|&end| match end {
                    Node::Ground => 0.0,
                    Node::Row(_) => RELTOL * x.node(end).abs() + VNTOL,
                })
                .sum(),
        }
    }

    /// How far the state moved on the solve's last Newton iteration, by `moved`.
    fn moved(&self, moved: &[f64]) -> f64 {
        match self.inductor {
            false => self
                .ends
                .iter()
                .map(|&end| match end {
                    Node::Ground => 0.0,
                    Node::Row(row) => moved[row],
                })
                .sum(),
            true => moved[self.branch],
        }
    }

    /// The absolute tolerance on a state that has moved `range` since t = 0: `ABSTOL` on an
    /// inductor's current, and on a capacitor `VNTOL` for each end on a row, or its charge floor
    /// over its capacitance when that is more. The charge floor is at most `DEVICE_SHARE` of the
    /// range.
    fn floor(&self, range: f64) -> f64 {
        match self.inductor {
            false => {
                let ends = self
                    .ends
                    .iter()
                    .filter(|end| matches!(end, Node::Row(_)))
                    .count();
                let charge = (self.charge / self.size).min(DEVICE_SHARE * range);
                (ends as f64 * VNTOL).max(charge)
            }
            true => ABSTOL,
        }
    }

    /// `SHARE` of the range `lo` to `hi` plus the floor.
    fn tolerance(&self, (lo, hi): (f64, f64)) -> f64 {
        SHARE * (hi - lo) + self.floor(hi - lo)
    }
}

/// Every capacitor and inductor of a circuit, capacitors first, in part order.
pub(crate) struct Stores {
    stores: Vec<Store>,
}

impl Stores {
    pub(crate) fn new(netlist: &Netlist, layout: &Layout) -> Stores {
        let mut stores = Vec::new();
        for (id, part) in netlist.parts() {
            for cap in element::Device::charges(&part.model) {
                stores.push(Store {
                    ends: cap.ends.map(|end| end.node(layout, id)),
                    branch: layout.branch_row(id, cap.k),
                    size: cap.farad,
                    inductor: false,
                    charge: match part.model {
                        Model::Capacitor(_) => 0.0,
                        _ => CHGTOL,
                    },
                });
            }
        }
        for (id, part) in netlist.parts() {
            for coil in element::Device::inductors(&part.model) {
                stores.push(Store {
                    ends: coil.ends.map(|end| end.node(layout, id)),
                    branch: layout.branch_row(id, layout.flux_branch(id, coil.k)),
                    size: coil.henry,
                    inductor: true,
                    charge: 0.0,
                });
            }
        }
        Stores { stores }
    }

    /// Whether a restart solve `held` left every store's slope where the step `step` put it,
    /// within Newton's tolerance on a capacitor's current or an inductor's voltage. The states
    /// are held across a restart, so then the points before it still lie on one smooth curve,
    /// and the next step reads them.
    pub(crate) fn smooth(&self, step: &Solution, held: &Solution) -> bool {
        self.stores.iter().all(|store| {
            (store.flow(held) - store.flow(step)).abs()
                <= store.flow_resolved(step).max(store.flow_resolved(held))
        })
    }

    /// The shortest time, in whole ps, a store takes to move its tolerance at its slope at `x`,
    /// the solve that starts a segment. Backward Euler's error is half the step times the
    /// change of slope over it, so on a slope that decays a first step that long is within
    /// half its tolerance. `u64::MAX` when no store moves.
    pub(crate) fn restart_step(&self, x: &Solution, ranges: &Ranges) -> u64 {
        let seconds = self
            .stores
            .iter()
            .zip(&ranges.0)
            .map(|(store, &range)| store.tolerance(range) / store.slope(x).abs())
            .fold(f64::INFINITY, f64::min);
        (seconds * 1e12) as u64
    }
}

/// The accepted points before the latest one that the next step reads, since its segment
/// started at t = 0 or at a restart solve.
#[derive(Clone, Debug)]
pub(crate) enum Past {
    /// The latest point starts a segment. Its slopes are the circuit's own, so the next step
    /// is backward Euler, and its estimate starts from them.
    Start,
    /// The segment's start, the one point before the latest. Its slopes are exact, so the
    /// estimate reads them.
    One { start: (Time, Solution) },
    /// The two points before the latest, the newer first.
    Two {
        before: (Time, Solution),
        older: (Time, Solution),
    },
}

impl Past {
    /// After a step from `(at, last)` is accepted. `keep` is false when the next step would be
    /// more than twice the one accepted, as after a step a breakpoint or a watch cut short.
    /// BDF2 is zero-stable only while a step is under `1 + sqrt(2)` times the one before, so
    /// `(at, last)` is left out, and the next step reads the points before it.
    pub(crate) fn advance(self, at: Time, last: Solution, keep: bool) -> Past {
        if !keep {
            return self;
        }
        match self {
            Past::Start => Past::One { start: (at, last) },
            Past::One { start } => Past::Two {
                before: (at, last),
                older: start,
            },
            Past::Two { before, .. } => Past::Two {
                before: (at, last),
                older: before,
            },
        }
    }

    /// 1 for backward Euler, 2 for BDF2.
    pub(crate) fn order(&self) -> u32 {
        match self {
            Past::Start => 1,
            Past::One { .. } | Past::Two { .. } => 2,
        }
    }

    /// The formula for a step of `h` ps from `previous` at `at`. BDF2 is the slope at the step's
    /// end of the parabola through the step's end, `previous` and the point before it, with
    /// `w` the step over the one before: `((1 + 2w) / (1 + w) * x - (1 + w) * x0 +
    /// w^2 / (1 + w) * x1) / h`.
    pub(crate) fn integration<'p>(
        &'p self,
        at: Time,
        previous: &'p Solution,
        h_ps: u64,
    ) -> Integration<'p> {
        let before = match self {
            Past::Start => None,
            Past::One { start: point } | Past::Two { before: point, .. } => Some(point),
        };
        let (coefficients, before) = match before {
            None => ([1.0, -1.0, 0.0], None),
            Some((then, solution)) => {
                let w = h_ps as f64 / (at.as_ps() - then.as_ps()) as f64;
                let coefficients = [(1.0 + 2.0 * w) / (1.0 + w), -(1.0 + w), w * w / (1.0 + w)];
                (coefficients, Some(solution))
            }
        };
        Integration {
            h_ps,
            coefficients,
            previous,
            before,
        }
    }
}

/// Each store's lowest and highest state since t = 0, in `Stores` order. The tolerance is a
/// share of that range, so it does not depend on the level the store sits at.
#[derive(Clone, Debug)]
pub(crate) struct Ranges(Vec<(f64, f64)>);

impl Ranges {
    pub(crate) fn start(stores: &Stores, x: &Solution) -> Ranges {
        Ranges(
            stores
                .stores
                .iter()
                .map(|store| (store.state(x), store.state(x)))
                .collect(),
        )
    }

    /// The ranges widened to the states at `x`.
    pub(crate) fn with(&self, stores: &Stores, x: &Solution) -> Ranges {
        Ranges(
            stores
                .stores
                .iter()
                .zip(&self.0)
                .map(|(store, &(lo, hi))| {
                    let state = store.state(x);
                    (lo.min(state), hi.max(state))
                })
                .collect(),
        )
    }
}

/// A step's local truncation error.
#[derive(Debug)]
pub(crate) struct Error {
    /// The worst store's error over its tolerance. At most 1 accepts.
    pub(crate) ratio: f64,
    /// The order of the step's formula, which sets how the next step grows or shrinks.
    pub(crate) order: u32,
}

/// The error of a step of `h` ps from `last` at `at` to `next`, the step's end with how far its
/// last Newton iteration moved each unknown. Each store's state is held to
/// `SHARE` of its range, the step's end included, plus its floor, so a store whose range is
/// still 0 is held to a few nanovolts or its charge tolerance. On a nonlinear circuit the step
/// is also allowed how far the solve's last iteration moved it: Newton stops within its own
/// tolerance of the level, and an error under that is the solve's, not the step's.
///
/// The predictor is the polynomial of the formula's order through the points the formula
/// reads: the state and the slope at a segment's start for backward Euler, and for BDF2 the
/// last two points with the slope at the start, or the last three. With `h` the step, `h1`
/// the one before and `h2` the one before that, 0 where the start's slope stands in for a
/// point, the error is the step's state less the predictor's, times `1/2` for backward Euler and
/// `h (h + h1) / (h (h + h1) + (2h + h1)(h + h1 + h2))` for BDF2, `2/11` at a constant step.
pub(crate) fn error(
    stores: &Stores,
    past: &Past,
    at: Time,
    last: &Solution,
    h_ps: u64,
    (next, moved): (&Solution, &[f64]),
    ranges: &Ranges,
) -> Error {
    let h = seconds(h_ps);
    let since = |then: Time| seconds(at.as_ps() - then.as_ps());
    let mut ratio = 0.0f64;
    for (store, &(lo, hi)) in stores.stores.iter().zip(&ranges.0) {
        let x0 = store.state(last);
        let x = store.state(next);
        let predicted = |h1: f64, h2: f64, d1: f64, d2: f64| {
            let curve = (d1 - d2) / (h1 + h2);
            x0 + h * d1 + h * (h + h1) * curve
        };
        let error = match past {
            Past::Start => (x - (x0 + h * store.slope(last))) / 2.0,
            Past::One {
                start: (then, start),
            } => {
                let h1 = since(*then);
                let d1 = (x0 - store.state(start)) / h1;
                let p = predicted(h1, 0.0, d1, store.slope(start));
                (x - p) * h / (3.0 * h + h1)
            }
            Past::Two {
                before: (then, before),
                older: (old, older),
            } => {
                let h1 = since(*then);
                let h2 = seconds(then.as_ps() - old.as_ps());
                let d1 = (x0 - store.state(before)) / h1;
                let d2 = (store.state(before) - store.state(older)) / h2;
                let p = predicted(h1, h2, d1, d2);
                let near = h * (h + h1);
                (x - p) * near / (near + (2.0 * h + h1) * (h + h1 + h2))
            }
        };
        let tolerance = store.tolerance((lo.min(x), hi.max(x))) + store.moved(moved);
        ratio = ratio.max(error.abs() / tolerance);
    }
    Error {
        ratio,
        order: past.order(),
    }
}

fn seconds(ps: u64) -> f64 {
    ps as f64 / 1e12
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One grounded capacitor of 1 F on net row 0, whose current is row 1.
    fn one_cap() -> Stores {
        Stores {
            stores: vec![Store {
                ends: [Node::Row(0), Node::Ground],
                branch: 1,
                size: 1.0,
                inductor: false,
                charge: CHGTOL,
            }],
        }
    }

    /// The state `v(t)` and its slope as row 0 and row 1 at `t` ps.
    fn point(v: impl Fn(f64) -> f64, dv: impl Fn(f64) -> f64, t: u64) -> (Time, Solution) {
        let s = seconds(t);
        (Time::from_ps(t), Solution::from(vec![v(s), dv(s)]))
    }

    /// The estimate against the true local error of each formula on `v = t^3`, whose third
    /// derivative is constant, so each estimate is exact up to rounding.
    #[test]
    fn the_estimate_is_the_local_error_of_bdf2_on_a_cubic() {
        let v = |t: f64| t * t * t;
        let dv = |t: f64| 3.0 * t * t;
        let stores = one_cap();
        let wide = Ranges(vec![(-1e9, 1e9)]);
        // Steps of 2, 3 and 5 s, in ps.
        let t = [
            1_000_000_000_000u64,
            3_000_000_000_000,
            6_000_000_000_000,
            11_000_000_000_000,
        ];
        let [p0, p1, p2, _] = t.map(|t| point(v, dv, t));
        let past = Past::Two {
            before: p1.clone(),
            older: p0.clone(),
        };
        // BDF2's own step to t3 from the exact past, solved for v'(t3) = 3 t3^2.
        let step = past.integration(p2.0, &p2.1, t[3] - t[2]);
        let h = step.h();
        let [a0, a1, a2] = step.coefficients;
        let x = (h * dv(seconds(t[3])) - a1 * v(seconds(t[2])) - a2 * v(seconds(t[1]))) / a0;
        let next = Solution::from(vec![x, dv(seconds(t[3]))]);
        let wanted = (x - v(seconds(t[3]))).abs();
        let got = error(
            &stores,
            &past,
            p2.0,
            &p2.1,
            t[3] - t[2],
            (&next, &[0.0; 2]),
            &wide,
        );
        let estimated = got.ratio * SHARE * 2e9;
        assert!(
            (estimated - wanted).abs() <= 1e-9 * wanted,
            "{estimated} vs {wanted}"
        );
        assert_eq!(got.order, 2);
    }

    /// From a segment's start, backward Euler's estimate is half its step's departure from the
    /// start's tangent, the true error on a parabola.
    #[test]
    fn backward_euler_from_a_start_reads_the_start_s_slope() {
        let v = |t: f64| t * t;
        let dv = |t: f64| 2.0 * t;
        let stores = one_cap();
        let wide = Ranges(vec![(-1e9, 1e9)]);
        let (at, last) = point(v, dv, 1_000_000_000_000);
        let h = 1_000_000_000_000;
        // v(2) = v(1) + h v'(2) with v' exact: 1 + 4 = 5 against the true 4.
        let next = Solution::from(vec![5.0, 4.0]);
        let got = error(
            &stores,
            &Past::Start,
            at,
            &last,
            h,
            (&next, &[0.0; 2]),
            &wide,
        );
        assert!(
            (got.ratio * SHARE * 2e9 - 1.0).abs() < 1e-9,
            "{}",
            got.ratio
        );
        assert_eq!(got.order, 1);
    }

    /// A store is also allowed how far its ends moved on the solve's last Newton iteration,
    /// and its current's own move does not count.
    #[test]
    fn the_tolerance_takes_the_last_newton_move_at_the_ends() {
        let stores = one_cap();
        let ranges = Ranges(vec![(0.0, 0.0)]);
        let (at, last) = point(|t| t * t, |t| 2.0 * t, 1_000_000_000_000);
        let next = Solution::from(vec![5.0, 4.0]);
        let ratio = |moved: [f64; 2]| {
            let h = 1_000_000_000_000;
            error(
                &stores,
                &Past::Start,
                at,
                &last,
                h,
                (&next, &moved),
                &ranges,
            )
            .ratio
        };
        // Backward Euler's error is half of 5 against the tangent's 3, and the range is 0 to 5.
        let tolerance = SHARE * 5.0 + VNTOL;
        assert!((ratio([0.0, 0.0]) * tolerance - 1.0).abs() < 1e-12);
        assert!((ratio([1e-6, 0.0]) * (tolerance + 1e-6) - 1.0).abs() < 1e-12);
        assert_eq!(ratio([0.0, 1.0]), ratio([0.0, 0.0]));
    }

    /// A device's capacitance is held to its charge, 2e-16 C, up to 3e-6 of its range. One that
    /// has not moved, a capacitor part and a large capacitance keep Newton's 1 nV an end.
    #[test]
    fn a_device_s_capacitance_is_held_to_its_charge_up_to_a_share_of_its_swing() {
        let cap = |farad: f64, charge: f64| Store {
            ends: [Node::Row(0), Node::Row(1)],
            branch: 2,
            size: farad,
            inductor: false,
            charge,
        };
        let near = |got: f64, want: f64| (got - want).abs() <= 1e-9 * want;
        // 1 pF that has not moved, or has rippled 0.1 mV: 3e-6 of it is under 1 nV an end.
        assert_eq!(cap(1e-12, CHGTOL).floor(0.0), 2.0 * VNTOL);
        assert_eq!(cap(1e-12, CHGTOL).floor(1e-4), 2.0 * VNTOL);
        // 1 fF over 1.5 mV: 3e-6 of it, 4.5 nV, not 0.2 V.
        assert!(near(cap(1e-15, CHGTOL).floor(1.5e-3), 4.5e-9));
        // 1 fF over 5 V: 3e-6 of 5 V, not 0.2 V.
        assert!(near(cap(1e-15, CHGTOL).floor(5.0), 1.5e-5));
        // 21 pF over 3.3 V: its charge, 9.5 uV, is under 3e-6 of 3.3 V.
        assert!(near(cap(21e-12, CHGTOL).floor(3.3), CHGTOL / 21e-12));
        assert_eq!(cap(1e-6, CHGTOL).floor(0.0), 2.0 * VNTOL);
        assert_eq!(cap(1e-12, 0.0).floor(0.0), 2.0 * VNTOL);
        assert_eq!(cap(1e-12, 0.0).floor(5.0), 2.0 * VNTOL);
    }

    /// The time the fastest store takes to move its tolerance at its slope.
    #[test]
    fn a_store_moves_its_tolerance_at_its_slope_in_the_restart_step() {
        let stores = one_cap();
        let ranges = Ranges(vec![(0.0, 1.0)]);
        let tolerance = SHARE + VNTOL;
        // A slope of an eighth of the tolerance a second: 8 s.
        let x = Solution::from(vec![0.5, tolerance / 8.0]);
        assert_eq!(stores.restart_step(&x, &ranges), 8_000_000_000_000);
        let fast = Solution::from(vec![0.5, 1e9]);
        assert_eq!(stores.restart_step(&fast, &ranges), 0);
        let still = Solution::from(vec![0.5, 0.0]);
        assert_eq!(stores.restart_step(&still, &ranges), u64::MAX);
    }

    /// The tolerance is a share of the range, so the same motion on another level gives the
    /// same ratio.
    #[test]
    fn the_tolerance_is_the_range_s_share_and_not_the_level_s() {
        let stores = one_cap();
        let ratio = |level: f64| {
            let last = Solution::from(vec![level, 1e-3]);
            let next = Solution::from(vec![level + 1e-3 + 1e-6, 1e-3]);
            let ranges = Ranges(vec![(level - 1.0, level + 1.0)]);
            error(
                &stores,
                &Past::Start,
                Time::from_ps(1),
                &last,
                1_000_000_000_000,
                (&next, &[0.0; 2]),
                &ranges,
            )
            .ratio
        };
        for level in [5.0, 1e3] {
            assert!(
                (ratio(0.0) - ratio(level)).abs() <= 1e-6 * ratio(0.0),
                "{level}"
            );
        }
    }

    #[test]
    fn a_step_more_than_twice_the_last_leaves_the_last_point_out() {
        let a = (Time::from_ps(0), Solution::from(vec![0.0]));
        let b = (Time::from_ps(10), Solution::from(vec![1.0]));
        let one = Past::Start.advance(a.0, a.1.clone(), true);
        assert!(matches!(&one, Past::One { start } if start.0 == a.0));
        let same = one.clone().advance(b.0, b.1.clone(), false);
        assert!(matches!(&same, Past::One { start } if start.0 == a.0));
        let two = one.advance(b.0, b.1, true);
        assert!(matches!(&two, Past::Two { before, older } if before.0 == b.0 && older.0 == a.0));
        assert!(matches!(Past::Start.advance(a.0, a.1, false), Past::Start));
    }

    /// At a constant step BDF2 is `(3 x - 4 x0 + x1) / 2h`.
    #[test]
    fn bdf2_at_a_constant_step_has_its_textbook_coefficients() {
        let x = Solution::from(vec![0.0]);
        let past = Past::One {
            start: (Time::from_ps(0), x.clone()),
        };
        let step = past.integration(Time::from_ps(10), &x, 10);
        assert_eq!(step.coefficients, [1.5, -2.0, 0.5]);
        assert_eq!(
            Past::Start
                .integration(Time::from_ps(10), &x, 10)
                .coefficients,
            [1.0, -1.0, 0.0]
        );
    }
}
