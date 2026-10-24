//! Local truncation error. Pure: solutions in, an error out.
//!
//! The estimate reads the capacitor currents the solve already has as branch
//! unknowns, so it costs no extra solve. Backward Euler is `h/2 * |i1 - i0| / C`,
//! which is `h^2/2 * v''`. The trapezoidal rule is `h^3 * |DD2(i)| / (6C)`, which is
//! `h^3/12 * v'''`, over the point before the last, the last, and the candidate.
//!
//! The tolerance is a share of each capacitor's swing in its segment, not of its level, so
//! a small swing on a large level is held to its own size, and a full swing is not held to
//! a millionth of the level it passes through.

use sim_core::netlist::{Netlist, PartId};
use sim_core::time::Time;

use crate::element;
use crate::mna::{Layout, Node, Solution};
use crate::newton::{ChargeIndex, Method, RELTOL, VNTOL, method_on};

/// Each capacitor's branch row, pins and capacitance, and the circuit's net rows. Built once
/// with the layout.
pub(crate) struct Charges {
    caps: Vec<Charge>,
    /// The net rows are the first rows of a solution. Ground has none.
    nets: usize,
}

struct Charge {
    /// In the circuit's charge list. Stiffness keys on it.
    index: ChargeIndex,
    branch: usize,
    pins: [Node; 2],
    farad: f64,
}

impl Charges {
    pub(crate) fn new(netlist: &Netlist, layout: &Layout) -> Charges {
        let mut owned = Vec::new();
        for (id, part) in netlist.parts() {
            for cap in element::charges(&part.model) {
                owned.push((id, cap));
            }
        }
        Self::from_owned(&owned, layout, netlist.nets().len() - 1)
    }

    fn from_owned(
        owned: &[(PartId, element::OwnedCapacitor)],
        layout: &Layout,
        nets: usize,
    ) -> Charges {
        let caps = owned
            .iter()
            .map(|(id, cap)| Charge {
                index: layout.charge_index(*id, cap.k),
                branch: layout.branch_row(*id, cap.k),
                pins: cap.ends.map(|end| end.node(layout, *id)),
                farad: cap.farad,
            })
            .collect();
        Charges { caps, nets }
    }

    /// The highest net voltage minus the lowest, ground's 0 V included.
    fn span(&self, solution: &Solution) -> f64 {
        let volts = solution.components()[..self.nets].iter();
        let (lo, hi) = volts.fold((0.0f64, 0.0f64), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        hi - lo
    }
}

impl Charge {
    fn volts(&self, solution: &Solution) -> f64 {
        let [a, b] = self.pins;
        solution.node(a) - solution.node(b)
    }
}

/// The share of its swing a trapezoidal step may be off by. On an exponential the error over
/// what is left of the swing grows by `x^3 / 12` a step, `x` the step over the time constant.
/// A step that rides `SHARE` of the whole swing has `x = 0.9 * (12 * SHARE * e^(t/tau))^(1/3)`,
/// so the error peaks 1.6 time constants after the edge at `0.039 * (12 * SHARE)^(2/3)` of the
/// swing, 9.5e-5 here. A step never reads more than the whole swing, so this bounds it.
const SHARE: f64 = 1e-5;

/// How far a capacitor's voltage moves in its segment. Its tolerance is `SHARE` of this,
/// plus `VNTOL`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Swing {
    /// The lowest and the highest voltage at the segment's points, the step's end included.
    lo: f64,
    hi: f64,
    /// The least distance to its asymptote that a step of the segment has read, at most the
    /// circuit's span.
    ahead: f64,
}

impl Swing {
    /// A segment's start at `v`. The first step's reading is the segment's swing before the
    /// voltage has moved, so that step is held to a share of the change in its current,
    /// `(i1 - i0)^2 / (2 * SHARE * i0^2)`, and not cut to the floor.
    fn start(v: f64, span: f64) -> Swing {
        Swing {
            lo: v,
            hi: v,
            ahead: span,
        }
    }

    /// A step of `h` s to `v`, its current going from `i0` to `i1`. The voltage is heading
    /// `v'^2 / |v''|` further, `h * i0^2 / (C * |i1 - i0|)`, which an exponential's distance to
    /// its asymptote equals at every point. From no current it is heading nowhere yet, and with
    /// a steady one as far as the circuit reaches. The least reading counts. Where two modes
    /// cancel the curvature at a segment's start, the first step reads past the swing, and a
    /// later one does not.
    fn step(self, v: f64, i: [f64; 2], h: f64, farad: f64) -> Swing {
        let [i0, i1] = i;
        let heading = match i0 {
            0.0 => 0.0,
            _ => h * i0 * i0 / (farad * (i1 - i0).abs()),
        };
        Swing {
            lo: self.lo.min(v),
            hi: self.hi.max(v),
            ahead: self.ahead.min(heading),
        }
    }

    fn volts(self) -> f64 {
        (self.hi - self.lo).max(self.ahead)
    }
}

/// The accepted points since the segment started, as far as the next step needs them.
#[derive(Clone)]
pub(crate) enum Segment {
    /// One point. The next step is backward Euler. `span` is the circuit's span at that
    /// point, and it caps the first step's look-ahead.
    Start { span: f64 },
    /// The accepted point before the last one. The next step is trapezoidal, except on the
    /// stiff capacitors.
    Smooth {
        before: (Time, Solution),
        stiff: Stiff,
        /// Each capacitor's swing up to the last point, in `Charges` order.
        swings: Vec<Swing>,
        /// Each capacitor's `max_step * 2|di| / 2C`, summed over the edges that kept the segment,
        /// in `Charges` order.
        ringing: Vec<f64>,
    },
}

/// Capacitors whose trapezoidal step failed Newton's tolerance at the 1 ps floor. Each holds a
/// mode faster than any step, on which the trapezoidal rule rings, so it takes backward
/// Euler. Backward Euler damps that mode by `1 + h/tau` a step, so once the step has grown
/// they try the trapezoidal rule again.
#[derive(Clone)]
pub(crate) enum Stiff {
    /// On backward Euler, ascending charge indices. After an accepted step of `retry` ps or
    /// more, the next step tries them on the trapezoidal rule.
    Held {
        indices: Vec<ChargeIndex>,
        retry: u64,
    },
    /// This step tries them on the trapezoidal rule. Only an accepted step of `retry` ps or
    /// more passes the try. A shorter accepted step, cut or not, is one a mode still alive
    /// passes too, so the next step tries them again.
    Trying {
        indices: Vec<ChargeIndex>,
        retry: u64,
    },
}

/// The accepted backward Euler step, in ps, after which a stiff capacitor first tries the
/// trapezoidal rule. Tried at 16 or 32 ps, a 1 ps mode is still alive, and the trapezoidal
/// rule keeps it.
const RETRY: u64 = 256;

/// A tried capacitor stays on the trapezoidal rule only under this share of Newton's tolerance.
/// What is left of the fast mode does not decay on the trapezoidal rule, and its error grows
/// with the step, so a try that passes near 1 holds the step near the length it was tried at.
const TRY: f64 = 1e-3;

impl Stiff {
    const NONE: Stiff = Stiff::Held {
        indices: Vec::new(),
        retry: RETRY,
    };

    /// The capacitors on backward Euler this step.
    fn indices(&self) -> &[ChargeIndex] {
        match self {
            Stiff::Held { indices, .. } => indices,
            Stiff::Trying { .. } => &[],
        }
    }

    /// After an accepted step of `h` ps.
    fn advance(self, h: u64) -> Stiff {
        match self {
            Stiff::Held { indices, retry } if !indices.is_empty() && h >= retry => {
                Stiff::Trying { indices, retry }
            }
            Stiff::Trying { retry, .. } if h >= retry => Stiff::NONE,
            stiff => stiff,
        }
    }

    /// For a retry of the same step of `h` ps: `more` on backward Euler too, and the tried
    /// capacitors back on it until the step reaches twice `h`.
    fn with(self, more: &[ChargeIndex], h: u64) -> Stiff {
        let (mut indices, retry) = match self {
            Stiff::Held { indices, retry } => (indices, retry),
            Stiff::Trying { indices, .. } => (indices, h.saturating_mul(2).max(RETRY)),
        };
        indices.extend_from_slice(more);
        indices.sort_unstable();
        indices.dedup();
        Stiff::Held { indices, retry }
    }
}

impl Segment {
    /// The span cap is this solution's span, read once here.
    pub(crate) fn start(charges: &Charges, solution: &Solution) -> Segment {
        Segment::Start {
            span: charges.span(solution),
        }
    }

    /// Once a step of `h` ps from `(at, last)` is accepted, `last` is the point before the
    /// new one. `swings` is that step's, from `error` when the accepted step is the one it
    /// judged.
    pub(crate) fn advance(self, at: Time, last: Solution, h: u64, swings: Vec<Swing>) -> Segment {
        let (stiff, ringing) = match self {
            Segment::Start { .. } => (Stiff::NONE, vec![0.0; swings.len()]),
            Segment::Smooth { stiff, ringing, .. } => (stiff.advance(h), ringing),
        };
        Segment::Smooth {
            before: (at, last),
            stiff,
            swings,
            ringing,
        }
    }

    /// The segment across a restart solve `held` at `step`, the solution this segment ends at.
    /// Each capacitor's current differs between them by `di`, the start of a mode the new drive
    /// excites. The point before moves by `di`, so the next trapezoidal estimate reads the slope
    /// before the edge and not a kink of `di` over the step that landed on it. The estimate
    /// still accepts or refuses every step after the edge.
    ///
    /// Backward Euler damps the mode, so it moves the current by at most `|di|` a step. The
    /// trapezoidal rule's factor on a mode much faster than the step is
    /// `(1 - h/2tau) / (1 + h/2tau)`, near -1, so the computed current swings from `di` to
    /// about `-di`, a move of `2|di|`, and it rings until the next segment start. A later
    /// edge's mode adds to it. So each capacitor sums `max_step * 2|di| / 2C`, backward Euler's
    /// estimate of that move over the longest step, across the edges that keep the segment. The
    /// segment goes on while every sum is at most `VNTOL`, and starts at `held` otherwise.
    pub(crate) fn across(
        self,
        charges: &Charges,
        step: &Solution,
        held: &Solution,
        max_step: u64,
    ) -> Segment {
        if let Segment::Smooth {
            before: (then, mut before),
            stiff,
            swings,
            mut ringing,
        } = self
        {
            let h = seconds(max_step);
            for (cap, sum) in charges.caps.iter().zip(&mut ringing) {
                let di = held.get(cap.branch) - step.get(cap.branch);
                before.add(cap.branch, di);
                *sum += h * di.abs() / cap.farad;
            }
            if ringing.iter().all(|&sum| sum <= VNTOL) {
                return Segment::Smooth {
                    before: (then, before),
                    stiff,
                    swings,
                    ringing,
                };
            }
        }
        Segment::start(charges, held)
    }

    /// Each capacitor's swing with a step of `h` ps from `last` to `next`.
    pub(crate) fn swings(
        &self,
        charges: &Charges,
        last: &Solution,
        h: u64,
        next: &Solution,
    ) -> Vec<Swing> {
        let h = seconds(h);
        match self {
            Segment::Start { span } => charges
                .caps
                .iter()
                .map(|cap| {
                    let i = [last.get(cap.branch), next.get(cap.branch)];
                    Swing::start(cap.volts(last), *span).step(cap.volts(next), i, h, cap.farad)
                })
                .collect(),
            Segment::Smooth { swings, .. } => swings
                .iter()
                .zip(&charges.caps)
                .map(|(swing, cap)| {
                    let i = [last.get(cap.branch), next.get(cap.branch)];
                    swing.step(cap.volts(next), i, h, cap.farad)
                })
                .collect(),
        }
    }

    /// The same point and step of `h` ps, with `more` stiff too.
    pub(crate) fn stiffen(self, more: &[ChargeIndex], h: u64) -> Segment {
        match self {
            // Every capacitor takes backward Euler on the first step, so none turns stiff on it.
            Segment::Start { span } => Segment::Start { span },
            Segment::Smooth {
                before,
                stiff,
                swings,
                ringing,
            } => Segment::Smooth {
                before,
                stiff: stiff.with(more, h),
                swings,
                ringing,
            },
        }
    }

    /// The method of every capacitor that is not stiff.
    pub(crate) fn method(&self) -> Method {
        match self {
            Segment::Start { .. } => Method::BackwardEuler,
            Segment::Smooth { .. } => Method::Trapezoidal,
        }
    }

    /// The stiff capacitors on backward Euler this step.
    pub(crate) fn stiff(&self) -> &[ChargeIndex] {
        match self {
            Segment::Start { .. } => &[],
            Segment::Smooth { stiff, .. } => stiff.indices(),
        }
    }

    /// The stiff capacitors this step tries on the trapezoidal rule.
    fn tried(&self) -> &[ChargeIndex] {
        match self {
            Segment::Smooth {
                stiff: Stiff::Trying { indices, .. },
                ..
            } => indices,
            _ => &[],
        }
    }

    fn method_of(&self, charge: ChargeIndex) -> Method {
        match self {
            Segment::Smooth { stiff, .. } => method_on(stiff.indices(), self.method(), charge),
            Segment::Start { .. } => Method::BackwardEuler,
        }
    }
}

/// A step's local truncation error.
#[derive(Debug)]
pub(crate) struct Error {
    /// The worst capacitor's error over its tolerance. At most 1 accepts.
    pub(crate) ratio: f64,
    /// That capacitor's method, which sets how the next step grows or shrinks.
    pub(crate) method: Method,
    /// Each capacitor's swing with this step folded in. `advance` stores it on accept.
    pub(crate) swings: Vec<Swing>,
    /// The trapezoidal capacitors over Newton's tolerance, ascending charge indices.
    over: Vec<ChargeIndex>,
    /// The worst error over Newton's tolerance of the capacitors the step tried on the
    /// trapezoidal rule. 0 when it tried none.
    tried: f64,
}

/// What a step does once its error is known.
#[derive(Debug)]
pub(crate) enum Verdict {
    Accept,
    /// Retry it shorter, from the same point.
    Reject,
    /// Retry the same step from the same point, with these capacitors stiff too, and the
    /// ones it tried back on backward Euler.
    Stiffen(Vec<ChargeIndex>),
}

impl Error {
    /// A step of `h` ps. A step of 1 ps over its tolerance is accepted whatever its error,
    /// unless a trapezoidal capacitor is over Newton's tolerance there, which makes it stiff.
    /// Stiffness is judged on Newton's tolerance. A 20 to 43 ps mode that the swing's share
    /// would let through at 1 ps would be left ringing once the step grew past it. A step that
    /// tried stiff capacitors keeps them on the trapezoidal rule only if their error is under
    /// `TRY` of Newton's tolerance.
    pub(crate) fn verdict(&self, h: u64) -> Verdict {
        match (
            self.tried <= TRY,
            h,
            self.over.is_empty(),
            self.ratio <= 1.0,
        ) {
            (false, ..) => Verdict::Stiffen(Vec::new()),
            (true, 1, false, _) => Verdict::Stiffen(self.over.clone()),
            (true, 1, true, _) | (true, _, _, true) => Verdict::Accept,
            (true, _, _, false) => Verdict::Reject,
        }
    }
}

/// The error of a step of `h` ps from `(at, last)` to `next`, each capacitor on its method
/// in `segment`. The tolerance is `SHARE` of the capacitor's swing plus `VNTOL`, or Newton's on
/// a stiff capacitor.
pub(crate) fn error(
    charges: &Charges,
    segment: &Segment,
    at: Time,
    last: &Solution,
    h: u64,
    next: &Solution,
) -> Error {
    let swings = segment.swings(charges, last, h, next);
    let h = seconds(h);
    let mut ratio_worst = 0.0;
    let mut method = segment.method();
    let mut over = Vec::new();
    let mut tried = 0.0f64;
    for (cap, swing) in charges.caps.iter().zip(&swings) {
        let (i0, i1) = (last.get(cap.branch), next.get(cap.branch));
        let cap_method = segment.method_of(cap.index);
        let error = match (cap_method, segment) {
            (
                Method::Trapezoidal,
                Segment::Smooth {
                    before: (then, before),
                    ..
                },
            ) => {
                let hp = seconds(at.as_ps() - then.as_ps());
                let im = before.get(cap.branch);
                let dd2 = ((i1 - i0) / h - (i0 - im) / hp) / (h + hp);
                h * h * h * dd2.abs() / (6.0 * cap.farad)
            }
            _ => h / 2.0 * (i1 - i0).abs() / cap.farad,
        };
        let level = cap.volts(last).abs().max(cap.volts(next).abs());
        let newton = error / (RELTOL * level + VNTOL);
        // A stiff capacitor's backward Euler error accumulates with one sign, so it keeps
        // Newton's tolerance. The swing's share holds only for a trapezoidal segment, and for
        // the one backward Euler step that starts a segment.
        let ratio = match (segment, cap_method) {
            (Segment::Smooth { .. }, Method::BackwardEuler) => newton,
            _ => error / (SHARE * swing.volts() + VNTOL),
        };
        if newton > 1.0 && cap_method == Method::Trapezoidal {
            over.push(cap.index);
        }
        if segment.tried().binary_search(&cap.index).is_ok() {
            tried = tried.max(newton);
        }
        if ratio > ratio_worst {
            (ratio_worst, method) = (ratio, cap_method);
        }
    }
    Error {
        ratio: ratio_worst,
        method,
        swings,
        over,
        tried,
    }
}

fn seconds(ps: u64) -> f64 {
    ps as f64 / 1e12
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids of two capacitors, `c1` and `c2`, in that order.
    /// One capacitor of `farad` from row 0 to ground, its current in row 1.
    fn one(farad: f64) -> Charges {
        Charges {
            caps: vec![Charge {
                index: ChargeIndex::new(0),
                branch: 1,
                pins: [Node::Row(0), Node::Ground],
                farad,
            }],
            nets: 1,
        }
    }

    /// Two capacitors of `farad` from row 0 to ground, their currents in rows 1 and 2.
    fn two(farad: f64) -> Charges {
        Charges {
            caps: vec![
                Charge {
                    index: ChargeIndex::new(0),
                    branch: 1,
                    pins: [Node::Row(0), Node::Ground],
                    farad,
                },
                Charge {
                    index: ChargeIndex::new(1),
                    branch: 2,
                    pins: [Node::Row(0), Node::Ground],
                    farad,
                },
            ],
            nets: 1,
        }
    }

    fn point(v: f64, i: f64) -> Solution {
        Solution::from(vec![v, i])
    }

    /// `segment` after an accepted step of `h` ps from `(at, last)` to `next`.
    fn accept(
        segment: Segment,
        charges: &Charges,
        at: Time,
        last: Solution,
        h: u64,
        next: &Solution,
    ) -> Segment {
        let swings = segment.swings(charges, &last, h, next);
        segment.advance(at, last, h, swings)
    }

    /// `segment` after an accepted step of `h` ps from `(at, last)` to 0 V and no current,
    /// on two capacitors.
    fn advanced(segment: Segment, at: Time, last: Solution, h: u64) -> Segment {
        let charges = two(1e-9);
        let next = Solution::zeros(3);
        accept(segment, &charges, at, last, h, &next)
    }

    fn ix(index: usize) -> ChargeIndex {
        ChargeIndex::new(index)
    }

    /// A trapezoidal segment from 0 V and 0 A at t = 0, with `stiff` stiff.
    fn smooth(stiff: &[ChargeIndex]) -> Segment {
        let last = Solution::zeros(3);
        advanced(Segment::start(&two(1e-9), &last), Time::ZERO, last, 1).stiffen(stiff, 1)
    }

    /// A trapezoidal segment on one capacitor whose voltage has covered `lo` to `hi`, with
    /// `before` the point before the last.
    fn swung(before: (Time, Solution), lo: f64, hi: f64) -> Segment {
        Segment::Smooth {
            before,
            stiff: Stiff::NONE,
            swings: vec![Swing { lo, hi, ahead: 0.0 }],
            ringing: vec![0.0],
        }
    }

    #[test]
    fn backward_euler_is_exact_for_a_current_ramp() {
        // i = k*t into 1 uF gives v'' = k/C, so the error of a 1 us step is h^2/2 * k/C.
        let (k, farad, h) = (2.0, 1e-6, 1e-6);
        let exact = h * h / 2.0 * k / farad;
        let last = point(1.0, 0.0);
        let next = point(1.0 + exact, k * h);
        // From no current the first step is heading nowhere, so its swing is its own move.
        // A stiff capacitor keeps Newton's tolerance.
        let charges = one(farad);
        let first = (Segment::start(&charges, &last), SHARE * exact + VNTOL);
        let stiff = (smooth(&[ix(0)]), RELTOL * (1.0 + exact) + VNTOL);
        for (segment, tolerance) in [first, stiff] {
            let got = error(
                &charges,
                &segment,
                Time::from_ps(1),
                &last,
                1_000_000,
                &next,
            );
            assert_eq!(got.method, Method::BackwardEuler);
            let want = exact / tolerance;
            assert!((got.ratio - want).abs() <= 1e-12 * want, "{got:?}");
        }
    }

    #[test]
    fn trapezoidal_is_exact_for_a_quadratic_current_on_an_uneven_grid() {
        // i = k*t^2 has DD2 = k on any grid, and v''' = 2k/C.
        let (k, farad) = (3e9, 1e-9);
        let current = |ps: u64| k * seconds(ps) * seconds(ps);
        let segment = swung((Time::from_ps(1_000), point(2.0, current(1_000))), 0.0, 2.0);
        let at = Time::from_ps(4_000);
        let last = point(2.0, current(4_000));
        let next = point(2.0, current(9_000));
        let got = error(&one(farad), &segment, at, &last, 5_000, &next);
        let h = seconds(5_000);
        let exact = h * h * h / 12.0 * (2.0 * k / farad);
        let tolerance = SHARE * 2.0 + VNTOL;
        assert_eq!(got.method, Method::Trapezoidal);
        let want = exact / tolerance;
        assert!((got.ratio - want).abs() <= 1e-9 * want, "{got:?}");
    }

    #[test]
    fn a_steady_current_has_no_error_on_either_rule() {
        let charges = one(1e-6);
        let last = point(1.0, 5e-3);
        let next = point(1.5, 5e-3);
        let after =
            |segment: Segment| accept(segment, &charges, Time::ZERO, point(0.5, 5e-3), 100, &last);
        let trapezoidal = after(Segment::start(&charges, &point(0.5, 5e-3)));
        let stiff = after(Segment::start(&charges, &point(0.5, 5e-3))).stiffen(&[ix(0)], 100);
        for segment in [Segment::start(&charges, &last), trapezoidal, stiff] {
            let got = error(&charges, &segment, Time::from_ps(100), &last, 100, &next);
            assert_eq!(got.ratio, 0.0);
        }
    }

    #[test]
    fn the_ratio_is_the_worst_capacitor_against_its_swing() {
        let charges = Charges {
            caps: vec![
                Charge {
                    index: ChargeIndex::new(0),
                    branch: 2,
                    pins: [Node::Row(0), Node::Ground],
                    farad: 1e-6,
                },
                Charge {
                    index: ChargeIndex::new(1),
                    branch: 3,
                    pins: [Node::Row(0), Node::Row(1)],
                    farad: 1e-6,
                },
            ],
            nets: 2,
        };
        // Over 1 ns the first sees 1 mA more from none, 0.5 uV against its move of 1 V. The
        // second sees 10 mA more, 5 uV against its move from 4 V to 6 V.
        let last = Solution::from(vec![0.0, -4.0, 0.0, 0.0]);
        let next = Solution::from(vec![1.0, -5.0, 1e-3, 1e-2]);
        let got = error(
            &charges,
            &Segment::start(&charges, &last),
            Time::ZERO,
            &last,
            1_000,
            &next,
        );
        let worst = 1e-9 / 2.0 * 1e-2 / 1e-6;
        assert_eq!(got.ratio, worst / (SHARE * 2.0 + VNTOL));
        let none = Charges {
            caps: Vec::new(),
            nets: 2,
        };
        let got = error(
            &none,
            &Segment::start(&none, &last),
            Time::ZERO,
            &last,
            1_000,
            &next,
        );
        assert_eq!(got.ratio, 0.0);
    }

    #[test]
    fn a_segment_takes_backward_euler_for_its_first_step_only() {
        assert_eq!(Segment::Start { span: 0.0 }.method(), Method::BackwardEuler);
        let last = Solution::zeros(3);
        let next = advanced(Segment::start(&two(1e-9), &last), Time::ZERO, last, 1_000);
        assert_eq!(next.method(), Method::Trapezoidal);
        let after = advanced(next, Time::from_ps(1), Solution::zeros(3), 1);
        assert_eq!(after.method(), Method::Trapezoidal);
    }

    #[test]
    fn a_stiff_capacitor_stays_on_backward_euler_and_the_others_do_not() {
        let segment = advanced(smooth(&[ix(1)]), Time::from_ps(1), Solution::zeros(3), 1);
        assert_eq!(segment.stiff(), [ix(1)]);
        assert_eq!(segment.method_of(ix(0)), Method::Trapezoidal);
        assert_eq!(segment.method_of(ix(1)), Method::BackwardEuler);
        let both = segment.stiffen(&[ix(1), ix(0)], 1);
        assert_eq!(both.stiff(), [ix(0), ix(1)]);
    }

    #[test]
    fn only_a_trapezoidal_capacitor_over_its_tolerance_at_1_ps_turns_stiff() {
        // Both currents jump by 1 mA into 1 fF: far over 1 nV on either rule.
        let last = Solution::from(vec![1.0, 0.0, 0.0]);
        let next = Solution::from(vec![1.0, 1e-3, 1e-3]);
        let at = Time::from_ps(1);
        let one_stiff = error(&two(1e-15), &smooth(&[ix(1)]), at, &last, 1, &next);
        assert!(matches!(one_stiff.verdict(1), Verdict::Stiffen(indices) if indices == [ix(0)]));
        let longer = error(&two(1e-15), &smooth(&[ix(1)]), at, &last, 2, &next);
        assert!(matches!(longer.verdict(2), Verdict::Reject));
        let both_stiff = error(&two(1e-15), &smooth(&[ix(0), ix(1)]), at, &last, 1, &next);
        assert_eq!(both_stiff.method, Method::BackwardEuler);
        assert!(matches!(both_stiff.verdict(1), Verdict::Accept));
        let calm = error(&two(1e-15), &smooth(&[ix(1)]), at, &last, 1, &last);
        assert!(matches!(calm.verdict(1), Verdict::Accept));
    }

    #[test]
    fn stiffness_is_judged_on_newton_s_tolerance() {
        // 1 V across 1 nF that has swung 1 V, and a current whose trapezoidal error at 1 ps
        // is 2 uV: under the swing's 10 uV, over Newton's 1 uV.
        // From no current 1 ps before, DD2 is `amps / (2 h^2)`, so the error is `h * amps / 12C`.
        let segment = swung((Time::ZERO, point(1.0, 0.0)), 0.0, 1.0);
        let last = point(1.0, 0.0);
        let amps = 2e-6 * 12.0 * 1e-9 / seconds(1);
        let next = point(1.0, amps);
        let got = error(&one(1e-9), &segment, Time::from_ps(1), &last, 1, &next);
        assert!((got.ratio - 2e-6 / (SHARE + VNTOL)).abs() < 1e-9, "{got:?}");
        assert!(matches!(got.verdict(1), Verdict::Stiffen(indices) if indices == [ix(0)]));
        // The same error on a longer step accepts: only the floor decides stiffness.
        let got = error(&one(1e-9), &segment, Time::from_ps(1), &last, 1, &next);
        assert!(matches!(got.verdict(2), Verdict::Accept));
    }

    /// `second` stiff, after an accepted step of `RETRY` ps at 1 V and no current.
    fn trying() -> Segment {
        let last = Solution::from(vec![1.0, 0.0, 0.0]);
        advanced(smooth(&[ix(1)]), Time::from_ps(1), last, RETRY)
    }

    #[test]
    fn a_stiff_capacitor_tries_the_trapezoidal_rule_once_its_step_reaches_256_ps() {
        let short = advanced(
            smooth(&[ix(1)]),
            Time::from_ps(1),
            Solution::zeros(3),
            RETRY - 1,
        );
        assert_eq!(short.stiff(), [ix(1)]);
        assert_eq!(short.tried(), []);
        let trying = trying();
        assert_eq!(trying.stiff(), []);
        assert_eq!(trying.tried(), [ix(1)]);
        assert_eq!(trying.method_of(ix(1)), Method::Trapezoidal);
        let kept = advanced(trying, Time::from_ps(RETRY + 1), Solution::zeros(3), RETRY);
        assert_eq!(kept.stiff(), []);
        assert_eq!(kept.tried(), []);
    }

    #[test]
    fn a_try_stays_only_well_under_the_tolerance_and_a_failed_one_waits_for_twice_the_step() {
        let at = Time::from_ps(RETRY + 1);
        let last = Solution::from(vec![1.0, 0.0, 0.0]);
        // The error of the tried capacitor grows with its current, the other's with its own.
        let step = |other: f64, tried: f64| {
            error(
                &two(1e-9),
                &trying(),
                at,
                &last,
                RETRY,
                &Solution::from(vec![1.0, other, tried]),
            )
        };
        let per_amp = step(0.0, 1.0).tried;
        assert!(matches!(
            step(0.0, 1e-4 / per_amp).verdict(RETRY),
            Verdict::Accept
        ));
        assert!(matches!(
            step(0.5 / per_amp, 1e-4 / per_amp).verdict(RETRY),
            Verdict::Accept
        ));
        let failed = step(0.0, 1e-2 / per_amp).verdict(RETRY);
        assert!(matches!(&failed, Verdict::Stiffen(parts) if parts.is_empty()));
        let held = trying().stiffen(&[], RETRY + 44);
        assert_eq!(held.stiff(), [ix(1)]);
        assert_eq!(held.method_of(ix(0)), Method::Trapezoidal);
        let short = advanced(held, at, Solution::zeros(3), 2 * (RETRY + 44) - 1);
        assert_eq!(short.stiff(), [ix(1)]);
        let again = advanced(short, at, Solution::zeros(3), 2 * (RETRY + 44));
        assert_eq!(again.tried(), [ix(1)]);
    }

    /// A try accepted short of `RETRY` has not shown that the fast mode died. The next step
    /// still tries it, and an error over `TRY` of Newton's tolerance puts it back on backward
    /// Euler, while the slow capacitor beside it keeps the trapezoidal rule.
    #[test]
    fn a_try_cut_short_of_its_step_tries_again() {
        let last = Solution::from(vec![1.0, 0.0, 0.0]);
        let (cut, h, farad) = (40, RETRY, 1e-9);
        let segment = advanced(trying(), Time::from_ps(RETRY + 1), last.clone(), cut);
        assert_eq!(segment.tried(), [ix(1)]);
        // From no current, the trapezoidal error is `h^2 * amps / (6C * (h + cut))`.
        let (hs, cs) = (seconds(h), seconds(cut));
        let amps = 1e-2 * (RELTOL + VNTOL) * 6.0 * farad * (hs + cs) / (hs * hs);
        let next = Solution::from(vec![1.0, 0.0, amps]);
        let at = Time::from_ps(RETRY + 1 + cut);
        let got = error(&two(farad), &segment, at, &last, h, &next);
        assert!(
            matches!(got.verdict(h), Verdict::Stiffen(ref parts) if parts.is_empty()),
            "{got:?}"
        );
        let retried = segment.stiffen(&[], h);
        assert_eq!(retried.method_of(ix(0)), Method::Trapezoidal);
        assert_eq!(retried.method_of(ix(1)), Method::BackwardEuler);
    }

    #[test]
    fn a_first_step_is_held_to_the_change_in_its_current() {
        // 3.3 V through 100 ohm into 100 pF, 40 ps into the edge: the current falls by h/tau.
        // Row 0 is the capacitor's net, row 1 the source's, row 2 the capacitor's current.
        let (farad, amps, h) = (1e-10, 0.033, 40);
        let drop = seconds(h) / 1e-8;
        let charges = Charges {
            caps: vec![Charge {
                branch: 2,
                ..one(farad).caps.remove(0)
            }],
            nets: 2,
        };
        let last = Solution::from(vec![0.0, 3.3, amps]);
        let moved = seconds(h) * amps / farad;
        let next = Solution::from(vec![moved, 3.3, amps * (1.0 - drop)]);
        let got = error(
            &charges,
            &Segment::start(&charges, &last),
            Time::ZERO,
            &last,
            h,
            &next,
        );
        // Heading `amps * tau / C`, 3.3 V, the ratio is the current's change squared over
        // twice the share.
        let want = drop * drop / (2.0 * SHARE);
        assert!((got.ratio - want).abs() <= 1e-3 * want, "{got:?} {want}");
        // The restart's nets are all at 0 V, so the span cap is 0 and the swing is the
        // step's own move. The candidate's source at 3.3 V would not cap it.
        let low_last = Solution::from(vec![0.0, 0.0, amps]);
        let low_next = Solution::from(vec![moved, 3.3, amps * (1.0 - drop)]);
        let capped = error(
            &charges,
            &Segment::start(&charges, &low_last),
            Time::ZERO,
            &low_last,
            h,
            &low_next,
        );
        let want = got.ratio * (SHARE * 3.3 + VNTOL) / (SHARE * moved + VNTOL);
        assert!((capped.ratio - want).abs() <= 1e-9 * want, "{capped:?}");
    }

    #[test]
    fn a_small_swing_on_a_large_level_is_held_to_the_swing() {
        // Held at 5 V, a 22 uF rail starts to sag by 20 mA on a 10 us time constant, which
        // heads 9.1 mV down. Its tolerance is a share of that and of the 10 mV it has moved,
        // not of the 5 V it sits at.
        let charges = one(22e-6);
        let (amps, h) = (-0.02, 1_000);
        let first = point(
            5.0 + seconds(h) * amps / 22e-6,
            amps * (1.0 - seconds(h) / 1e-5),
        );
        let from = point(5.0, amps);
        let held = accept(
            Segment::start(&charges, &from),
            &charges,
            Time::ZERO,
            from,
            h,
            &first,
        );
        let last = point(4.995, -0.015);
        let next = point(4.99, -0.012);
        let got = error(
            &charges,
            &held,
            Time::from_ps(1_000),
            &last,
            1_000_000,
            &next,
        );
        let segment = swung((Time::ZERO, point(5.0, -0.02)), 4.995, 5.0);
        let want = error(
            &charges,
            &segment,
            Time::from_ps(1_000),
            &last,
            1_000_000,
            &next,
        );
        assert_eq!(got.ratio, want.ratio);
        let Segment::Smooth { swings, .. } = &held else {
            panic!("the first step starts a smooth segment");
        };
        assert_eq!(swings[0].lo, first.get(0));
        let heading = 0.02 * 1e-5 / 22e-6;
        assert!(
            (swings[0].ahead - heading).abs() < 1e-6 * heading,
            "{swings:?}"
        );
    }

    #[test]
    fn a_swing_is_the_range_of_the_segment_s_points() {
        let charges = one(1e-6);
        let mut last = point(1.0, 0.0);
        let mut segment = Segment::start(&charges, &last);
        let mut at = Time::ZERO;
        for v in [1.5, 0.5, 0.75] {
            let next = point(v, 0.0);
            segment = accept(segment, &charges, at, last, 1_000, &next);
            at = Time::from_ps(at.as_ps() + 1_000);
            last = next;
        }
        let Segment::Smooth { swings, .. } = &segment else {
            panic!("an accepted step makes a smooth segment");
        };
        assert_eq!(swings[0].volts(), 1.0);
    }

    #[test]
    fn the_least_heading_a_step_reads_holds_for_the_rest_of_the_segment() {
        // A steady 1 mA into 1 nF heads past the circuit, so the first step reads the span at
        // the restart, 1 V. The step itself reaches 1.001 V. A step whose current falls by a
        // tenth in 1 ns reads 10 mV, and a steady one after it does not widen that again.
        let charges = one(1e-9);
        let points = [(1.0, 1e-3), (1.001, 1e-3), (1.002, 0.9e-3), (1.003, 0.9e-3)];
        let mut segment = Segment::start(&charges, &point(1.0, 1e-3));
        let mut ahead = Vec::new();
        for (k, pair) in points.windows(2).enumerate() {
            let [(v0, i0), (v1, i1)] = [pair[0], pair[1]];
            let at = Time::from_ps(k as u64 * 1_000);
            segment = accept(segment, &charges, at, point(v0, i0), 1_000, &point(v1, i1));
            let Segment::Smooth { swings, .. } = &segment else {
                panic!("an accepted step makes a smooth segment");
            };
            ahead.push(swings[0].ahead);
        }
        assert_eq!(ahead[0], 1.0);
        assert!((ahead[1] - 0.01).abs() < 1e-12, "{ahead:?}");
        assert_eq!(ahead[2], ahead[1]);
    }

    /// A resistor, then two capacitors. The resistor owns no charge, so each capacitor's
    /// part index differs from its charge index.
    fn split() -> (sim_core::netlist::Netlist, Layout) {
        let netlist = sim_core::netlist::parse(
            br#"{
              "version": 0,
              "seed": 0,
              "analysis": { "transient": { "stop_ps": 1000, "max_step_ps": 1000, "grid_ps": 1000 } },
              "ground": "gnd",
              "nets": ["gnd", "a", "b"],
              "parts": [
                { "name": "m1", "kind": "resistor", "pins": { "a": "a", "b": "gnd" },
                  "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
                  "ratings": { "power": null } },
                { "name": "n1", "kind": "capacitor", "pins": { "a": "a", "b": "gnd" },
                  "capacitance": { "value": 1e-9, "unit": "F", "source": "user" }, "initial": null },
                { "name": "o1", "kind": "capacitor", "pins": { "a": "b", "b": "gnd" },
                  "capacitance": { "value": 2e-9, "unit": "F", "source": "user" }, "initial": null }
              ]
            }"#,
        )
        .unwrap();
        let layout = Layout::new(&netlist, Vec::new());
        (netlist, layout)
    }

    #[test]
    fn every_capacitor_joins_the_charge_list_on_its_own_branch() {
        let (netlist, layout) = split();
        let charges = Charges::new(&netlist, &layout);
        let mut owned = Vec::new();
        for (id, part) in netlist.parts() {
            for cap in element::charges(&part.model) {
                owned.push((id, cap.k, part.name.as_str()));
            }
        }
        assert_eq!(charges.caps.len(), owned.len());
        let names: Vec<_> = owned.iter().map(|(_, _, name)| *name).collect();
        assert_eq!(names, ["n1", "o1"]);
        for (index, ((id, k, _), cap)) in owned.iter().zip(&charges.caps).enumerate() {
            assert_eq!(*k, 0);
            assert_eq!(cap.branch, layout.branch_row(*id, *k));
            assert_eq!(cap.index, layout.charge_index(*id, *k));
            assert_eq!(cap.index, ix(index));
            assert_ne!(ChargeIndex::new(id.index()), cap.index);
        }
    }

    /// A capacitor's charge is on its pins. A BJT's two are on its internal nodes, `CJE` from
    /// `b'` to `e'` and `CJC` from `b'` to `c'`. An `nmos` adds three on its pins, `CGS`, `CGD`
    /// and `CDS`, each on its own branch.
    #[test]
    fn charges_are_the_parts_capacitors() {
        let netlist = sim_core::netlist::parse(
            br#"{
              "version": 0,
              "seed": 0,
              "analysis": { "transient": { "stop_ps": 1000, "max_step_ps": 1000, "grid_ps": 1000 } },
              "ground": "gnd",
              "nets": ["gnd", "b", "c"],
              "parts": [
                { "name": "c1", "kind": "capacitor", "pins": { "a": "c", "b": "gnd" },
                  "capacitance": { "value": 1e-9, "unit": "F", "source": "user" }, "initial": null },
                { "name": "q1", "kind": "npn",
                  "pins": { "collector": "c", "base": "b", "emitter": "gnd" },
                  "saturation_current": { "value": 1e-14, "unit": "A", "source": "user" },
                  "forward_beta": { "value": 100.0, "unit": "1", "source": "user" },
                  "reverse_beta": { "value": 1.0, "unit": "1", "source": "user" },
                  "forward_emission": { "value": 1.0, "unit": "1", "source": "user" },
                  "reverse_emission": { "value": 1.0, "unit": "1", "source": "user" },
                  "early_voltage": { "value": 100.0, "unit": "V", "source": "user" },
                  "base_resistance": { "value": 10.0, "unit": "ohm", "source": "user" },
                  "collector_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
                  "emitter_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
                  "base_emitter_capacitance": { "value": 8e-12, "unit": "F", "source": "user" },
                  "base_collector_capacitance": { "value": 4e-12, "unit": "F", "source": "user" },
                  "ratings": { "current": null, "power": null } },
                { "name": "m1", "kind": "nmos",
                  "pins": { "drain": "c", "gate": "b", "source": "gnd" },
                  "transconductance_parameter": { "value": 0.0468, "unit": "A/V^2", "source": "user" },
                  "threshold_voltage": { "value": 1.08, "unit": "V", "source": "user" },
                  "channel_length_modulation": { "value": 0.0, "unit": "1/V", "source": "user" },
                  "gate_source_capacitance": { "value": 55e-12, "unit": "F", "source": "user" },
                  "gate_drain_capacitance": { "value": 5e-12, "unit": "F", "source": "user" },
                  "drain_source_capacitance": { "value": 20e-12, "unit": "F", "source": "user" },
                  "ratings": { "current": null, "power": null } }
              ]
            }"#,
        )
        .unwrap();
        let layout = Layout::new(&netlist, Vec::new());
        let charges = Charges::new(&netlist, &layout);
        let id = |name| {
            netlist
                .part_id(&sim_core::name::Name::new(name).unwrap())
                .unwrap()
        };
        let (cap, q, m) = (id("c1"), id("q1"), id("m1"));
        let [c, b, e] = [0, 1, 2].map(|k| layout.internal(q, k));
        let want = [
            (
                layout.branch_row(cap, 0),
                [Node::Row(1), Node::Ground],
                1e-9,
            ),
            (
                layout.branch_row(m, 0),
                [Node::Row(0), Node::Ground],
                55e-12,
            ),
            (layout.branch_row(m, 1), [Node::Row(0), Node::Row(1)], 5e-12),
            (
                layout.branch_row(m, 2),
                [Node::Row(1), Node::Ground],
                20e-12,
            ),
            (layout.branch_row(q, 0), [b, e], 8e-12),
            (layout.branch_row(q, 1), [b, c], 4e-12),
        ];
        let got: Vec<_> = charges
            .caps
            .iter()
            .map(|charge| (charge.branch, charge.pins, charge.farad))
            .collect();
        assert_eq!(got, want);
        let indices: Vec<_> = charges.caps.iter().map(|charge| charge.index).collect();
        assert_eq!(indices, [ix(0), ix(1), ix(2), ix(3), ix(4), ix(5)]);
    }

    #[test]
    #[should_panic(expected = "a part's charge count exceeds its branches")]
    fn a_part_with_a_charge_and_no_branch_panics() {
        let (netlist, pair, cn, _) = two_internal();
        let mut open = pair;
        open.branches = 0;
        Layout::with_stand_in(&netlist, Vec::new(), cn, &open);
    }

    #[test]
    #[should_panic(expected = "branch 1 is past this part's branches")]
    fn a_branch_past_the_reservation_panics() {
        let (netlist, layout) = split();
        let id = netlist
            .part_id(&sim_core::name::Name::new("n1").unwrap())
            .unwrap();
        layout.branch_row(id, 1);
    }

    #[test]
    #[should_panic(expected = "charge 1 is past this part's charges")]
    fn a_charge_past_the_reservation_panics() {
        let (netlist, layout) = split();
        let id = netlist
            .part_id(&sim_core::name::Name::new("n1").unwrap())
            .unwrap();
        layout.charge_index(id, 1);
    }

    /// `c_n` owns two capacitors. The second sits on `c_n`'s internal nodes. `d_o` is the
    /// next part's capacitor, so a charge cursor that advances by one collides with it.
    fn two_internal() -> (sim_core::netlist::Netlist, element::Pair, PartId, PartId) {
        use sim_core::netlist::NetId;

        let netlist = sim_core::netlist::parse(
            br#"{
              "version": 0,
              "seed": 0,
              "analysis": { "op": {} },
              "ground": "gnd",
              "nets": ["gnd", "a", "b"],
              "parts": [
                { "name": "a_r", "kind": "resistor", "pins": { "a": "a", "b": "gnd" },
                  "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
                  "ratings": { "power": null } },
                { "name": "b_r", "kind": "resistor", "pins": { "a": "b", "b": "gnd" },
                  "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
                  "ratings": { "power": null } },
                { "name": "c_n", "kind": "capacitor", "pins": { "a": "a", "b": "gnd" },
                  "capacitance": { "value": 1e-6, "unit": "F", "source": "user" },
                  "initial": null },
                { "name": "d_o", "kind": "capacitor", "pins": { "a": "b", "b": "gnd" },
                  "capacitance": { "value": 1e-9, "unit": "F", "source": "user" },
                  "initial": null },
                { "name": "e_s", "kind": "voltage_source", "pins": { "p": "a", "n": "gnd" },
                  "voltage": { "value": 2.0, "unit": "V", "source": "user" } },
                { "name": "f_s", "kind": "voltage_source", "pins": { "p": "b", "n": "gnd" },
                  "voltage": { "value": 3.0, "unit": "V", "source": "user" } },
                { "name": "g_s", "kind": "voltage_source", "pins": { "p": "b", "n": "gnd" },
                  "voltage": { "value": 2.5, "unit": "V", "source": "user" } }
              ]
            }"#,
        )
        .unwrap();
        let (cn, later) = {
            let id = |name: &str| {
                netlist
                    .part_id(&sim_core::name::Name::new(name).unwrap())
                    .unwrap()
            };
            (id("c_n"), id("d_o"))
        };
        let net_a = netlist
            .nets()
            .find(|(_, name)| name.as_str() == "a")
            .unwrap()
            .0;
        let pair = element::Pair {
            ends: [
                [
                    element::CapacitorEnd::Net(net_a),
                    element::CapacitorEnd::Net(NetId::GROUND),
                ],
                [
                    element::CapacitorEnd::Internal(0),
                    element::CapacitorEnd::Internal(1),
                ],
            ],
            farad: [1e-6, 2e-6],
            internal_nodes: 2,
            branches: 2,
        };
        (netlist, pair, cn, later)
    }

    /// Capacitor 1 is between two internal nodes. Stiffening only its charge uses backward
    /// Euler there, and capacitor 0 stays trapezoidal. The truncation error judges each
    /// capacitor by that same method. An unstiffened step uses each capacitor's own previous
    /// current.
    #[test]
    fn a_second_capacitor_between_internal_nodes_follows_its_own_charge() {
        use crate::mna::{Stamp, System};
        use crate::newton::{Conditions, History, Integration, Iterate};

        let (netlist, pair, cn, later) = two_internal();
        let reserved = element::unknowns(&pair);
        assert_eq!(reserved.internal_nodes, 2);
        assert_eq!(reserved.branches, 2);
        let fs = netlist
            .part_id(&sim_core::name::Name::new("f_s").unwrap())
            .unwrap();
        let gs = netlist
            .part_id(&sim_core::name::Name::new("g_s").unwrap())
            .unwrap();
        let layout = Layout::with_stand_in(&netlist, Vec::new(), cn, &pair);
        let owned: Vec<_> = element::charges(&pair)
            .into_iter()
            .map(|cap| (cn, cap))
            .collect();
        let charges = Charges::from_owned(&owned, &layout, netlist.nets().len() - 1);
        assert_eq!(charges.caps.len(), 2);
        // `Charges::new` reads the netlist models. `c_n` and `d_o` each own one capacitor.
        assert_eq!(Charges::new(&netlist, &layout).caps.len(), 2);
        assert_eq!(charges.caps[0].branch, layout.branch_row(cn, 0));
        assert_eq!(charges.caps[1].branch, layout.branch_row(cn, 1));
        assert_ne!(charges.caps[0].branch, charges.caps[1].branch);
        assert_eq!(
            charges.caps[1].pins,
            [layout.internal(cn, 0), layout.internal(cn, 1)]
        );
        assert_eq!(layout.charge_index(cn, 0), ix(0));
        assert_eq!(layout.charge_index(cn, 1), ix(1));
        assert_eq!(layout.charge_index(later, 0), ix(2));
        assert_ne!(ix(cn.index()), layout.charge_index(cn, 1));
        assert_eq!(layout.branch_row(cn, 1), layout.branch_row(cn, 0) + 1);
        assert_ne!(layout.branch_row(cn, 1), layout.branch_row(later, 0));

        let mut prev = vec![0.0; layout.size()];
        let place = |node: Node| match node {
            Node::Row(row) => row,
            Node::Ground => panic!("this end is ground"),
        };
        prev[place(charges.caps[0].pins[0])] = 2.0;
        prev[place(layout.internal(cn, 0))] = 3.0;
        prev[place(layout.internal(cn, 1))] = 2.5;
        prev[layout.branch_row(cn, 0)] = 1e-4;
        prev[layout.branch_row(cn, 1)] = 4e-4;
        let previous = Solution::from(prev);
        let i0 = [1e-4, 4e-4];
        let farad = [1e-6, 2e-6];
        let volts = [2.0, 0.5];

        let solved = |stiff: &[ChargeIndex], restart: bool| -> [f64; 2] {
            let step = Integration {
                method: Method::Trapezoidal,
                stiff,
                h_ps: 1_000_000_000,
                previous: &previous,
            };
            let history = if restart {
                History::Restart {
                    previous: &previous,
                }
            } else {
                History::Step(step)
            };
            let conditions = Conditions {
                history,
                ..Conditions::NOMINAL
            };
            let scale = conditions.source_scale;
            let iterate = Iterate {
                solution: Solution::zeros(layout.size()),
                junctions: vec![0.0; layout.junctions()],
            };
            let mut system = System::new(&layout);
            system.reset(&layout, conditions.gmin);
            for (id, part) in netlist.parts() {
                let mut stamp = Stamp::new(&mut system, &layout, id, &iterate, &conditions);
                if id == cn {
                    element::stamp(&pair, &mut stamp);
                } else if id == fs {
                    let node = layout.internal(cn, 0);
                    stamp.branch_current(node, Node::Ground, 0);
                    stamp.branch_row(node, Node::Ground, 0, 0.0, 1.0, 3.0 * scale);
                } else if id == gs {
                    let node = layout.internal(cn, 1);
                    stamp.branch_current(node, Node::Ground, 0);
                    stamp.branch_row(node, Node::Ground, 0, 0.0, 1.0, 2.5 * scale);
                } else {
                    element::stamp(&part.model, &mut stamp);
                }
            }
            let solution = system.solve().expect("the held nodes are nonsingular");
            assert!((solution.node(layout.internal(cn, 0)) - 3.0).abs() < 1e-9);
            assert!((solution.node(layout.internal(cn, 1)) - 2.5).abs() < 1e-9);
            [
                solution.get(layout.branch_row(cn, 0)),
                solution.get(layout.branch_row(cn, 1)),
            ]
        };
        let at_v0 = |k: usize, method: Method| -> f64 {
            let (c, h, v, v0, i) = (farad[k], 1e-3, volts[k], volts[k], i0[k]);
            match method {
                Method::BackwardEuler => c / h * (v - v0),
                Method::Trapezoidal => 2.0 * c / h * (v - v0) - i,
            }
        };
        let near = |got: f64, expect: f64| (got - expect).abs() < 1e-9;

        let restart = solved(&[], true);
        assert!(near(restart[0], 0.0), "{restart:?}");
        assert!(near(restart[1], 0.0), "{restart:?}");

        let both = solved(&[], false);
        assert!(near(both[0], at_v0(0, Method::Trapezoidal)), "{both:?}");
        assert!(near(both[1], at_v0(1, Method::Trapezoidal)), "{both:?}");

        let stiff = solved(&[ix(1)], false);
        assert!(near(stiff[0], at_v0(0, Method::Trapezoidal)), "{stiff:?}");
        assert!(near(stiff[1], at_v0(1, Method::BackwardEuler)), "{stiff:?}");
        assert!((stiff[1] - both[1]).abs() > 1e-4, "{stiff:?} {both:?}");

        let by_part = solved(&[ix(cn.index())], false);
        assert!(near(by_part[0], both[0]), "{by_part:?}");
        assert!(near(by_part[1], both[1]), "{by_part:?}");

        // Capacitor 1 carries 4 A, capacitor 0 carries 1 A, and the next capacitor carries
        // 1.2 mA. Backward Euler on capacitor 1 is the largest error. The trapezoidal
        // errors of the other two clear Newton's tolerance and stay smaller.
        let later_model = &netlist
            .parts()
            .find(|(id, _)| *id == later)
            .expect("d_o")
            .1
            .model;
        let mut judged_owned = owned;
        judged_owned.extend(
            element::charges(later_model)
                .into_iter()
                .map(|cap| (later, cap)),
        );
        let judged = Charges::from_owned(&judged_owned, &layout, netlist.nets().len() - 1);
        let zeros = Solution::zeros(layout.size());
        let smooth = accept(
            Segment::start(&judged, &zeros),
            &judged,
            Time::ZERO,
            zeros.clone(),
            1,
            &zeros,
        )
        .stiffen(&[layout.charge_index(cn, 1)], 1);
        let mut stepped = vec![0.0; layout.size()];
        stepped[layout.branch_row(cn, 0)] = 1.0;
        stepped[layout.branch_row(cn, 1)] = 4.0;
        stepped[layout.branch_row(later, 0)] = 1.2e-3;
        let got = error(
            &judged,
            &smooth,
            Time::from_ps(1),
            &zeros,
            1,
            &Solution::from(stepped),
        );
        let want = seconds(1) * 4.0 / (2.0 * 2e-6) / VNTOL;
        assert_eq!(got.method, Method::BackwardEuler);
        assert!((got.ratio - want).abs() <= 1e-9 * want, "{got:?} {want}");
        assert!(
            matches!(got.verdict(1), Verdict::Stiffen(indices) if indices == [ix(0), ix(2)]),
            "{got:?}"
        );
    }
}
