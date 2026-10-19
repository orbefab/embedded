//! Local truncation error. Pure: solutions in, an error out.
//!
//! The estimate reads the capacitor currents the solve already has as branch
//! unknowns, so it costs no extra solve. Backward Euler is `h/2 * |i1 - i0| / C`,
//! which is `h^2/2 * v''`. The trapezoidal rule is `h^3 * |DD2(i)| / (6C)`, which is
//! `h^3/12 * v'''`, over the point before the last, the last, and the candidate.

use sim_core::netlist::{Model, Netlist, PartId};
use sim_core::time::Time;

use crate::mna::{Layout, Node, Solution};
use crate::newton::{Method, RELTOL, VNTOL};

/// Each capacitor's branch row, pins and capacitance. Built once with the layout.
pub(crate) struct Charges {
    caps: Vec<Charge>,
}

struct Charge {
    part: PartId,
    branch: usize,
    pins: [Node; 2],
    farad: f64,
}

impl Charges {
    pub(crate) fn new(netlist: &Netlist, layout: &Layout) -> Charges {
        let caps = netlist
            .parts()
            .filter_map(|(id, part)| match &part.model {
                Model::Capacitor(capacitor) => Some(Charge {
                    part: id,
                    branch: layout.branch_row(id, 0),
                    pins: capacitor.pins.map(Node::from),
                    farad: capacitor.capacitance.si.get(),
                }),
                _ => None,
            })
            .collect();
        Charges { caps }
    }
}

/// The accepted points since t = 0 or the last restart, as far as the next step needs them.
pub(crate) enum Segment {
    /// One point. The next step is backward Euler.
    Start,
    /// The accepted point before the last one. The next step is trapezoidal, except on the
    /// stiff capacitors.
    Smooth {
        before: (Time, Solution),
        stiff: Stiff,
    },
}

/// Capacitors whose trapezoidal step failed its tolerance at the 1 ps floor. Each holds a
/// mode faster than any step, on which the trapezoidal rule rings, so it takes backward
/// Euler. Backward Euler damps that mode by `1 + h/tau` a step, so once the step has grown
/// they try the trapezoidal rule again.
pub(crate) enum Stiff {
    /// On backward Euler, ascending. After an accepted step of `retry` ps or more, the next
    /// step tries them on the trapezoidal rule.
    Held { parts: Vec<PartId>, retry: u64 },
    /// This step tries them on the trapezoidal rule.
    Trying(Vec<PartId>),
}

/// The accepted backward Euler step, in ps, after which a stiff capacitor first tries the
/// trapezoidal rule. Tried at 16 or 32 ps, a 1 ps mode is still alive, and the trapezoidal
/// rule keeps it.
const RETRY: u64 = 256;

/// A tried capacitor stays on the trapezoidal rule only under this share of its tolerance.
/// What is left of the fast mode does not decay on the trapezoidal rule, and its error grows
/// with the step, so a try that passes near 1 holds the step near the length it was tried at.
const TRY: f64 = 1e-3;

impl Stiff {
    const NONE: Stiff = Stiff::Held {
        parts: Vec::new(),
        retry: RETRY,
    };

    /// The capacitors on backward Euler this step.
    fn parts(&self) -> &[PartId] {
        match self {
            Stiff::Held { parts, .. } => parts,
            Stiff::Trying(_) => &[],
        }
    }

    /// After an accepted step of `h` ps.
    fn advance(self, h: u64) -> Stiff {
        match self {
            Stiff::Held { parts, retry } if !parts.is_empty() && h >= retry => Stiff::Trying(parts),
            Stiff::Trying(_) => Stiff::NONE,
            held => held,
        }
    }

    /// For a retry of the same step of `h` ps: `more` on backward Euler too, and the tried
    /// capacitors back on it until the step reaches twice `h`.
    fn with(self, more: &[PartId], h: u64) -> Stiff {
        let (mut parts, retry) = match self {
            Stiff::Held { parts, retry } => (parts, retry),
            Stiff::Trying(parts) => (parts, h.saturating_mul(2).max(RETRY)),
        };
        parts.extend_from_slice(more);
        parts.sort_unstable();
        parts.dedup();
        Stiff::Held { parts, retry }
    }
}

impl Segment {
    /// Once a step of `h` ps from `(at, last)` is accepted, `last` is the point before the
    /// new one.
    pub(crate) fn advance(self, at: Time, last: Solution, h: u64) -> Segment {
        let stiff = match self {
            Segment::Start => Stiff::NONE,
            Segment::Smooth { stiff, .. } => stiff.advance(h),
        };
        Segment::Smooth {
            before: (at, last),
            stiff,
        }
    }

    /// The same point and step of `h` ps, with `more` stiff too.
    pub(crate) fn stiffen(self, more: &[PartId], h: u64) -> Segment {
        match self {
            // Every capacitor takes backward Euler on the first step, so none turns stiff on it.
            Segment::Start => Segment::Start,
            Segment::Smooth { before, stiff } => Segment::Smooth {
                before,
                stiff: stiff.with(more, h),
            },
        }
    }

    /// The method of every capacitor that is not stiff.
    pub(crate) fn method(&self) -> Method {
        match self {
            Segment::Start => Method::BackwardEuler,
            Segment::Smooth { .. } => Method::Trapezoidal,
        }
    }

    /// The stiff capacitors on backward Euler this step.
    pub(crate) fn stiff(&self) -> &[PartId] {
        match self {
            Segment::Start => &[],
            Segment::Smooth { stiff, .. } => stiff.parts(),
        }
    }

    /// The stiff capacitors this step tries on the trapezoidal rule.
    fn tried(&self) -> &[PartId] {
        match self {
            Segment::Smooth {
                stiff: Stiff::Trying(parts),
                ..
            } => parts,
            _ => &[],
        }
    }

    fn method_of(&self, part: PartId) -> Method {
        match self {
            Segment::Smooth { stiff, .. } if stiff.parts().binary_search(&part).is_err() => {
                Method::Trapezoidal
            }
            _ => Method::BackwardEuler,
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
    /// The trapezoidal capacitors over their tolerance, ascending.
    over: Vec<PartId>,
    /// The worst error over its tolerance of the capacitors the step tried on the trapezoidal
    /// rule. 0 when it tried none.
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
    Stiffen(Vec<PartId>),
}

impl Error {
    /// A step of `h` ps. Every capacitor over its tolerance at 1 ps is on backward Euler,
    /// which is accepted whatever its error, or on the trapezoidal rule, which makes it stiff.
    /// A step that tried stiff capacitors keeps them on the trapezoidal rule only if their
    /// error is under `TRY` of its tolerance.
    pub(crate) fn verdict(self, h: u64) -> Verdict {
        match (
            self.tried <= TRY,
            self.ratio <= 1.0,
            h,
            self.over.is_empty(),
        ) {
            (false, ..) => Verdict::Stiffen(Vec::new()),
            (true, true, ..) | (true, false, 1, true) => Verdict::Accept,
            (true, false, 1, false) => Verdict::Stiffen(self.over),
            (true, false, _, _) => Verdict::Reject,
        }
    }
}

/// The error of a step of `h` ps from `(at, last)` to `next`, each capacitor on its method
/// in `segment`. The tolerance is `RELTOL * max(|v0|, |v1|) + VNTOL`, Newton's constants.
pub(crate) fn error(
    charges: &Charges,
    segment: &Segment,
    at: Time,
    last: &Solution,
    h: u64,
    next: &Solution,
) -> Error {
    let h = seconds(h);
    let mut worst = Error {
        ratio: 0.0,
        method: segment.method(),
        over: Vec::new(),
        tried: 0.0,
    };
    for cap in &charges.caps {
        let (i0, i1) = (last.get(cap.branch), next.get(cap.branch));
        let method = segment.method_of(cap.part);
        let error = match (method, segment) {
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
        let [a, b] = cap.pins;
        let v0 = last.node(a) - last.node(b);
        let v1 = next.node(a) - next.node(b);
        let ratio = error / (RELTOL * v0.abs().max(v1.abs()) + VNTOL);
        if ratio > 1.0 && method == Method::Trapezoidal {
            worst.over.push(cap.part);
        }
        if segment.tried().binary_search(&cap.part).is_ok() {
            worst.tried = worst.tried.max(ratio);
        }
        if ratio > worst.ratio {
            (worst.ratio, worst.method) = (ratio, method);
        }
    }
    worst
}

fn seconds(ps: u64) -> f64 {
    ps as f64 / 1e12
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids of two capacitors, `c1` and `c2`, in that order.
    fn ids() -> [PartId; 2] {
        let netlist = sim_core::netlist::parse(
            br#"{
              "version": 0,
              "seed": 0,
              "analysis": { "transient": { "stop_ps": 1000, "max_step_ps": 1000, "grid_ps": 1000 } },
              "ground": "gnd",
              "nets": ["gnd", "a"],
              "parts": [
                { "name": "c1", "kind": "capacitor", "pins": { "a": "a", "b": "gnd" },
                  "capacitance": { "value": 1e-6, "unit": "F", "source": "user" }, "initial": null },
                { "name": "c2", "kind": "capacitor", "pins": { "a": "a", "b": "gnd" },
                  "capacitance": { "value": 1e-6, "unit": "F", "source": "user" }, "initial": null }
              ]
            }"#,
        )
        .unwrap();
        let ids: Vec<PartId> = netlist.parts().map(|(id, _)| id).collect();
        [ids[0], ids[1]]
    }

    /// One capacitor of `farad` from row 0 to ground, its current in row 1.
    fn one(farad: f64) -> Charges {
        Charges {
            caps: vec![Charge {
                part: ids()[0],
                branch: 1,
                pins: [Node::Row(0), Node::Ground],
                farad,
            }],
        }
    }

    /// Two capacitors of `farad` from row 0 to ground, their currents in rows 1 and 2.
    fn two(farad: f64) -> Charges {
        let [first, second] = ids();
        Charges {
            caps: vec![
                Charge {
                    part: first,
                    branch: 1,
                    pins: [Node::Row(0), Node::Ground],
                    farad,
                },
                Charge {
                    part: second,
                    branch: 2,
                    pins: [Node::Row(0), Node::Ground],
                    farad,
                },
            ],
        }
    }

    fn point(v: f64, i: f64) -> Solution {
        Solution::from(vec![v, i])
    }

    /// A trapezoidal segment from 0 V and 0 A at t = 0, with `stiff` stiff.
    fn smooth(stiff: &[PartId]) -> Segment {
        Segment::Start
            .advance(Time::ZERO, Solution::zeros(3), 1)
            .stiffen(stiff, 1)
    }

    #[test]
    fn backward_euler_is_exact_for_a_current_ramp() {
        // i = k*t into 1 uF gives v'' = k/C, so the error of a 1 us step is h^2/2 * k/C.
        let (k, farad, h) = (2.0, 1e-6, 1e-6);
        let exact = h * h / 2.0 * k / farad;
        let last = point(1.0, 0.0);
        let next = point(1.0 + exact, k * h);
        let tolerance = RELTOL * (1.0 + exact) + VNTOL;
        for segment in [Segment::Start, smooth(&[ids()[0]])] {
            let got = error(
                &one(farad),
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
        let segment =
            Segment::Start.advance(Time::from_ps(1_000), point(2.0, current(1_000)), 1_000);
        let at = Time::from_ps(4_000);
        let last = point(2.0, current(4_000));
        let next = point(2.0, current(9_000));
        let got = error(&one(farad), &segment, at, &last, 5_000, &next);
        let h = seconds(5_000);
        let exact = h * h * h / 12.0 * (2.0 * k / farad);
        let tolerance = RELTOL * 2.0 + VNTOL;
        assert_eq!(got.method, Method::Trapezoidal);
        let want = exact / tolerance;
        assert!((got.ratio - want).abs() <= 1e-9 * want, "{got:?}");
    }

    #[test]
    fn a_steady_current_has_no_error_on_either_rule() {
        let last = point(1.0, 5e-3);
        let next = point(1.5, 5e-3);
        let trapezoidal = Segment::Start.advance(Time::ZERO, point(0.5, 5e-3), 100);
        let stiff = Segment::Start
            .advance(Time::ZERO, point(0.5, 5e-3), 100)
            .stiffen(&[ids()[0]], 100);
        for segment in [Segment::Start, trapezoidal, stiff] {
            let got = error(&one(1e-6), &segment, Time::from_ps(100), &last, 100, &next);
            assert_eq!(got.ratio, 0.0);
        }
    }

    #[test]
    fn the_ratio_is_the_worst_capacitor_against_the_larger_end_voltage() {
        let [first, second] = ids();
        let charges = Charges {
            caps: vec![
                Charge {
                    part: first,
                    branch: 2,
                    pins: [Node::Row(0), Node::Ground],
                    farad: 1e-6,
                },
                Charge {
                    part: second,
                    branch: 3,
                    pins: [Node::Row(0), Node::Row(1)],
                    farad: 1e-6,
                },
            ],
        };
        // Over 1 ns the first sees 1 mA more, 0.5 uV against 1 V. The second sees 10 mA more,
        // 5 uV against the larger of 4 V and 6 V.
        let last = Solution::from(vec![0.0, -4.0, 0.0, 0.0]);
        let next = Solution::from(vec![1.0, -5.0, 1e-3, 1e-2]);
        let got = error(&charges, &Segment::Start, Time::ZERO, &last, 1_000, &next);
        let worst = 1e-9 / 2.0 * 1e-2 / 1e-6;
        assert_eq!(got.ratio, worst / (RELTOL * 6.0 + VNTOL));
        let none = Charges { caps: Vec::new() };
        let got = error(&none, &Segment::Start, Time::ZERO, &last, 1_000, &next);
        assert_eq!(got.ratio, 0.0);
    }

    #[test]
    fn a_segment_takes_backward_euler_for_its_first_step_only() {
        assert_eq!(Segment::Start.method(), Method::BackwardEuler);
        let next = Segment::Start.advance(Time::ZERO, Solution::zeros(2), 1_000);
        assert_eq!(next.method(), Method::Trapezoidal);
        let after = next.advance(Time::from_ps(1), Solution::zeros(2), 1);
        assert_eq!(after.method(), Method::Trapezoidal);
    }

    #[test]
    fn a_stiff_capacitor_stays_on_backward_euler_and_the_others_do_not() {
        let [first, second] = ids();
        let segment = smooth(&[second]).advance(Time::from_ps(1), Solution::zeros(3), 1);
        assert_eq!(segment.stiff(), [second]);
        assert_eq!(segment.method_of(first), Method::Trapezoidal);
        assert_eq!(segment.method_of(second), Method::BackwardEuler);
        let both = segment.stiffen(&[second, first], 1);
        assert_eq!(both.stiff(), [first, second]);
    }

    #[test]
    fn only_a_trapezoidal_capacitor_over_its_tolerance_at_1_ps_turns_stiff() {
        let [first, second] = ids();
        // Both currents jump by 1 mA into 1 fF: far over 1 nV on either rule.
        let last = Solution::from(vec![1.0, 0.0, 0.0]);
        let next = Solution::from(vec![1.0, 1e-3, 1e-3]);
        let at = Time::from_ps(1);
        let one_stiff = error(&two(1e-15), &smooth(&[second]), at, &last, 1, &next);
        assert!(matches!(one_stiff.verdict(1), Verdict::Stiffen(parts) if parts == [first]));
        let longer = error(&two(1e-15), &smooth(&[second]), at, &last, 2, &next);
        assert!(matches!(longer.verdict(2), Verdict::Reject));
        let both_stiff = error(&two(1e-15), &smooth(&[first, second]), at, &last, 1, &next);
        assert_eq!(both_stiff.method, Method::BackwardEuler);
        assert!(matches!(both_stiff.verdict(1), Verdict::Accept));
        let calm = error(&two(1e-15), &smooth(&[second]), at, &last, 1, &last);
        assert!(matches!(calm.verdict(1), Verdict::Accept));
    }

    /// `second` stiff, after an accepted step of `RETRY` ps at 1 V and no current.
    fn trying() -> Segment {
        smooth(&[ids()[1]]).advance(Time::from_ps(1), Solution::from(vec![1.0, 0.0, 0.0]), RETRY)
    }

    #[test]
    fn a_stiff_capacitor_tries_the_trapezoidal_rule_once_its_step_reaches_256_ps() {
        let [_, second] = ids();
        let short = smooth(&[second]).advance(Time::from_ps(1), Solution::zeros(3), RETRY - 1);
        assert_eq!(short.stiff(), [second]);
        assert_eq!(short.tried(), []);
        let trying = trying();
        assert_eq!(trying.stiff(), []);
        assert_eq!(trying.tried(), [second]);
        assert_eq!(trying.method_of(second), Method::Trapezoidal);
        let kept = trying.advance(Time::from_ps(RETRY + 1), Solution::zeros(3), RETRY);
        assert_eq!(kept.stiff(), []);
        assert_eq!(kept.tried(), []);
    }

    #[test]
    fn a_try_stays_only_well_under_the_tolerance_and_a_failed_one_waits_for_twice_the_step() {
        let [first, second] = ids();
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
        assert_eq!(held.stiff(), [second]);
        assert_eq!(held.method_of(first), Method::Trapezoidal);
        let short = held.advance(at, Solution::zeros(3), 2 * (RETRY + 44) - 1);
        assert_eq!(short.stiff(), [second]);
        let again = short.advance(at, Solution::zeros(3), 2 * (RETRY + 44));
        assert_eq!(again.tried(), [second]);
    }
}
