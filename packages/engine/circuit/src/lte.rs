//! Local truncation error. Pure: solutions in, an error out.
//!
//! The estimate reads the capacitor currents the solve already has as branch
//! unknowns, so it costs no extra solve. Backward Euler is `h/2 * |i1 - i0| / C`,
//! which is `h^2/2 * v''`. The trapezoidal rule is `h^3 * |DD2(i)| / (6C)`, which is
//! `h^3/12 * v'''`, over the point before the last, the last, and the candidate.
//!
//! An inductor is the dual. Its state is its current, a branch unknown, and its derivative is
//! the voltage across it, so the same estimates read `v` and `L` in place of `i` and `C`, in
//! amperes.
//!
//! The tolerance is a share of each store's swing in its segment, not of its level, so a small
//! swing on a large level is held to its own size, and a full swing is not held to a millionth
//! of the level it passes through.

use orbe_engine_core::netlist::{Netlist, PartId};
use orbe_engine_core::time::Time;

use crate::element;
use crate::mna::{Layout, Node, Solution};
use crate::newton::{ABSTOL, ChargeIndex, FluxIndex, Method, RELTOL, Store, VNTOL, method_on};

/// Each capacitor's branch row, pins and capacitance, each inductor's, and the circuit's net
/// rows. Built once with the layout.
pub(crate) struct Charges {
    caps: Vec<Charge>,
    fluxes: Vec<Flux>,
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

struct Flux {
    /// In the circuit's flux list. Stiffness keys on it.
    index: FluxIndex,
    /// The inductor's current.
    branch: usize,
    /// The nodes its voltage is across.
    ends: [Node; 2],
    henry: f64,
    /// The resistance in series, so the most current it carries is the circuit's span over it.
    ohm: f64,
}

impl Charges {
    pub(crate) fn new(netlist: &Netlist, layout: &Layout) -> Charges {
        let mut caps = Vec::new();
        let mut fluxes = Vec::new();
        for (id, part) in netlist.parts() {
            for cap in element::charges(&part.model) {
                caps.push((id, cap));
            }
            for inductor in element::inductors(&part.model) {
                fluxes.push((id, inductor));
            }
        }
        Self::from_owned(&caps, &fluxes, layout, netlist.nets().len() - 1)
    }

    fn from_owned(
        caps: &[(PartId, element::OwnedCapacitor)],
        fluxes: &[(PartId, element::OwnedInductor)],
        layout: &Layout,
        nets: usize,
    ) -> Charges {
        let caps = caps
            .iter()
            .map(|(id, cap)| Charge {
                index: layout.charge_index(*id, cap.k),
                branch: layout.branch_row(*id, cap.k),
                pins: cap.ends.map(|end| end.node(layout, *id)),
                farad: cap.farad,
            })
            .collect();
        let fluxes = fluxes
            .iter()
            .map(|(id, inductor)| Flux {
                index: layout.flux_index(*id, inductor.k),
                branch: layout.branch_row(*id, layout.flux_branch(*id, inductor.k)),
                ends: inductor.ends.map(|end| end.node(layout, *id)),
                henry: inductor.henry,
                ohm: inductor.ohm,
            })
            .collect();
        Charges { caps, fluxes, nets }
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

impl Flux {
    fn volts(&self, solution: &Solution) -> f64 {
        let [a, b] = self.ends;
        solution.node(a) - solution.node(b)
    }
}

/// The share of its swing a trapezoidal step may be off by. On an exponential the error over
/// what is left of the swing grows by `x^3 / 12` a step, `x` the step over the time constant.
/// A step that rides `SHARE` of the whole swing has `x = 0.9 * (12 * SHARE * e^(t/tau))^(1/3)`,
/// so the error peaks 1.6 time constants after the edge at `0.039 * (12 * SHARE)^(2/3)` of the
/// swing, 9.5e-5 here. A step never reads more than the whole swing, so this bounds it.
const SHARE: f64 = 1e-5;

/// How far a store's state moves in its segment, a capacitor's voltage or an inductor's
/// current. Its tolerance is `SHARE` of this, plus `VNTOL` or `ABSTOL`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Swing {
    /// The lowest and the highest state at the segment's points, the step's end included.
    lo: f64,
    hi: f64,
    /// The least distance to its asymptote that a step of the segment has read, at most the
    /// circuit's reach.
    ahead: f64,
}

impl Swing {
    /// A segment's start at `x`, with `reach` the furthest the state can go. The first step's
    /// reading is the segment's swing before the state has moved, so that step is held to a
    /// share of the change in its slope, `(i1 - i0)^2 / (2 * SHARE * i0^2)` on a capacitor,
    /// and not cut to the floor.
    fn start(x: f64, reach: f64) -> Swing {
        Swing {
            lo: x,
            hi: x,
            ahead: reach,
        }
    }

    /// A step of `h` s to `x`, its slope times `size` going from `d[0]` to `d[1]`: a
    /// capacitor's voltage, its current and its capacitance, or an inductor's current, its
    /// voltage and its inductance. The state is heading `x'^2 / |x''|` further,
    /// `h * d0^2 / (size * |d1 - d0|)`, which an exponential's distance to its asymptote equals
    /// at every point. From no slope it is heading nowhere yet, and with a steady one as far as
    /// the circuit reaches. The least reading counts. Where two modes cancel the curvature at a
    /// segment's start, the first step reads past the swing, and a later one does not.
    fn step(self, x: f64, d: [f64; 2], h: f64, size: f64) -> Swing {
        let [d0, d1] = d;
        let heading = match d0 {
            0.0 => 0.0,
            _ => h * d0 * d0 / (size * (d1 - d0).abs()),
        };
        Swing {
            ahead: self.ahead.min(heading),
            ..self.within(x)
        }
    }

    /// The range widened to `x`.
    fn within(self, x: f64) -> Swing {
        Swing {
            lo: self.lo.min(x),
            hi: self.hi.max(x),
            ahead: self.ahead,
        }
    }

    fn size(self) -> f64 {
        (self.hi - self.lo).max(self.ahead)
    }
}

/// An inductor's two swings. The current's is a capacitor voltage's dual. The voltage's reach
/// is read once, at the segment's start. An inductor's voltage heads to 0 V at DC, so the reach
/// is its distance to 0 V there, or the circuit's span if that is larger, for a coil at rest
/// beside an edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FluxSwing {
    amps: Swing,
    volts: Swing,
}

/// Every store's swing up to a point, in `Charges` order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Swings {
    caps: Vec<Swing>,
    fluxes: Vec<FluxSwing>,
    /// Caps then fluxes. The step's change in slope reverses the previous change, and each
    /// change clears `RELTOL` of the largest sample plus the row's absolute tolerance.
    reverses: Vec<bool>,
}

/// Each store's sum of what the edges that kept the segment may have left ringing, in `Charges`
/// order. A capacitor's in volts, an inductor's in amperes.
#[derive(Clone, Debug)]
pub(crate) struct Ringing {
    caps: Vec<f64>,
    fluxes: Vec<f64>,
}

impl Ringing {
    /// None yet on any of the stores `swings` covers.
    fn none(swings: &Swings) -> Ringing {
        Ringing {
            caps: vec![0.0; swings.caps.len()],
            fluxes: vec![0.0; swings.fluxes.len()],
        }
    }

    /// Whether every capacitor's sum is at most `VNTOL` and every inductor's at most `ABSTOL`.
    fn quiet(&self) -> bool {
        self.caps.iter().all(|&sum| sum <= VNTOL) && self.fluxes.iter().all(|&sum| sum <= ABSTOL)
    }
}

/// The accepted points since the segment started, as far as the next step needs them.
/// `Smooth` holds a solution, so the variant is already far larger than `Start`.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub(crate) enum Segment {
    /// One point. The next step is backward Euler. `span` is the circuit's span at that
    /// point, and it caps the first step's look-ahead.
    Start { span: f64 },
    /// The accepted point before the last one. The next step is trapezoidal, except on the
    /// stiff stores.
    Smooth {
        before: (Time, Solution),
        stiff: Stiff,
        /// Each store's swing up to the last point.
        swings: Swings,
        ringing: Ringing,
        /// Caps then fluxes. How many accepted steps in a row have reversed that store's
        /// slope. A segment start clears it. A kept edge keeps it.
        reversals: Vec<u32>,
    },
}

/// Stores whose trapezoidal step failed Newton's tolerance at the 1 ps floor. Each holds a
/// mode faster than any step, on which the trapezoidal rule rings, so it takes backward
/// Euler. Backward Euler damps that mode by `1 + h/tau` a step, so once the step has grown
/// they try the trapezoidal rule again.
#[derive(Clone)]
pub(crate) enum Stiff {
    /// On backward Euler, ascending. The next step tries the trapezoidal rule after an accepted
    /// step of `retry` ps, or once the accepted steps of this hold add up to `HOLD_FACTOR`
    /// times `retry`. `waited` is that sum. It also grows while no store is held, from the
    /// segment start or from the end of the last hold. A store that joins does not restart it.
    /// A failed try does.
    Held {
        stores: Vec<Store>,
        retry: u64,
        waited: u64,
    },
    /// This step tries them on the trapezoidal rule. Only an accepted step of `retry` ps or
    /// more passes the try. A try that started because the hold reached its time uses the
    /// length of the step that started it, when that step was shorter than the old `retry`.
    /// A shorter accepted step, cut or not, is one a mode still alive passes too, so the next
    /// step tries them again.
    Trying { stores: Vec<Store>, retry: u64 },
}

/// The accepted backward Euler step, in ps, after which a stiff store first tries the
/// trapezoidal rule. Tried at 16 or 32 ps, a 1 ps mode is still alive, and the trapezoidal
/// rule keeps it.
const RETRY: u64 = 256;

/// A hold that never accepts a step of `retry` ps still tries after this many waits.
const HOLD_FACTOR: u64 = 4;

/// Slope reversals in a row, the candidate included, after which a trapezoidal store over
/// Newton's tolerance turns stiff at any step length.
const REVERSAL_RUN: u32 = 3;

impl Stiff {
    const NONE: Stiff = Stiff::Held {
        stores: Vec::new(),
        retry: RETRY,
        waited: 0,
    };

    /// The stores on backward Euler this step.
    fn stores(&self) -> &[Store] {
        match self {
            Stiff::Held { stores, .. } => stores,
            Stiff::Trying { .. } => &[],
        }
    }

    /// After an accepted step of `h` ps. An empty hold adds `h` and does not try.
    fn advance(self, h: u64) -> Stiff {
        match self {
            Stiff::Held {
                stores,
                retry,
                waited,
            } => {
                let waited = waited.saturating_add(h);
                if !stores.is_empty() && (h >= retry || waited >= HOLD_FACTOR.saturating_mul(retry))
                {
                    Stiff::Trying {
                        stores,
                        retry: retry.min(h.max(1)),
                    }
                } else {
                    Stiff::Held {
                        stores,
                        retry,
                        waited,
                    }
                }
            }
            Stiff::Trying { retry, .. } if h >= retry => Stiff::NONE,
            stiff => stiff,
        }
    }

    /// For a retry of the same step of `h` ps: `more` on backward Euler too, and the tried
    /// stores back on it until the step reaches twice `h`. A failed try restarts the hold's
    /// clock. A ring on another store during a try sends the tried stores back the same
    /// way. A store already held keeps its wait and its clock.
    fn with(self, more: &[Store], h: u64) -> Stiff {
        let (mut stores, retry, waited) = match self {
            Stiff::Held {
                stores,
                retry,
                waited,
            } => (stores, retry, waited),
            Stiff::Trying { stores, .. } => (stores, h.saturating_mul(2).max(RETRY), 0),
        };
        stores.extend_from_slice(more);
        stores.sort_unstable();
        stores.dedup();
        Stiff::Held {
            stores,
            retry,
            waited,
        }
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
    pub(crate) fn advance(self, at: Time, last: Solution, h: u64, swings: Swings) -> Segment {
        let (stiff, ringing, reversals) = match self {
            Segment::Start { .. } => (
                Stiff::NONE,
                Ringing::none(&swings),
                vec![0; swings.reverses.len()],
            ),
            Segment::Smooth {
                stiff,
                ringing,
                reversals,
                ..
            } => {
                let reversals = reversals
                    .iter()
                    .zip(&swings.reverses)
                    .map(|(&count, &rev)| if rev { count + 1 } else { 0 })
                    .collect();
                (stiff.advance(h), ringing, reversals)
            }
        };
        Segment::Smooth {
            before: (at, last),
            stiff,
            swings,
            ringing,
            reversals,
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
    /// estimate of that move over the longest step, across the edges that keep the segment.
    ///
    /// An inductor's voltage jumps by `dv` in the same way, and it sums `max_step * |dv| / L`.
    /// Its point before does not move, because its voltage is a difference of node rows that
    /// other parts share. The segment goes on while every capacitor's sum is at most `VNTOL`
    /// and every inductor's at most `ABSTOL`, and starts at `held` otherwise.
    ///
    /// Newton settles each row of `step` and of `held` only to its own tolerance. The restart
    /// solve holds each capacitor at its voltage and solves the rest. On an edge that changes
    /// nothing, the jump is the step's companion current against the current the circuit draws
    /// at that voltage, plus what Newton leaves. Two settled solves can differ by about twice
    /// the row tolerance. The floor is that residual. A coil's relative
    /// term is `RELTOL` times the voltage across the coil, and that term is small when the
    /// coil's ends sit at volts. The floor stays the row tolerance. A jump inside it adds
    /// nothing to a sum. The edge started no mode. The point before still moves by the raw
    /// `di`. Every step's history already carries a residual of that size.
    ///
    /// A kept edge ends the stiff hold and its clock. The reversal count stays, because the
    /// segment continues. The stores take the trapezoidal rule on the step the segment wanted.
    /// A segment start's next step is 1 ns of backward Euler, and a kept edge does not take
    /// that step. A switch edge or a source jump that keeps the segment is still cut at its
    /// breakpoint 1 ns later. A stiff store otherwise leaves backward Euler through a try after
    /// a step of `RETRY` ps or more, or after `HOLD_FACTOR` times its wait of accepted steps.
    pub(crate) fn across(
        self,
        charges: &Charges,
        step: &Solution,
        held: &Solution,
        max_step: u64,
    ) -> Segment {
        if let Segment::Smooth {
            before: (then, mut before),
            swings,
            mut ringing,
            reversals,
            ..
        } = self
        {
            let h = seconds(max_step);
            for (cap, sum) in charges.caps.iter().zip(&mut ringing.caps) {
                let (from, to) = (step.get(cap.branch), held.get(cap.branch));
                before.add(cap.branch, to - from);
                *sum += h * resolved(from, to, ABSTOL) / cap.farad;
            }
            for (flux, sum) in charges.fluxes.iter().zip(&mut ringing.fluxes) {
                let (from, to) = (flux.volts(step), flux.volts(held));
                *sum += h * resolved(from, to, VNTOL) / flux.henry;
            }
            if ringing.quiet() {
                return Segment::Smooth {
                    before: (then, before),
                    stiff: Stiff::NONE,
                    swings,
                    ringing,
                    reversals,
                };
            }
        }
        Segment::start(charges, held)
    }

    /// Each store's swing with a step of `h` ps from `last` to `next`.
    pub(crate) fn swings(
        &self,
        charges: &Charges,
        last: &Solution,
        h: u64,
        next: &Solution,
    ) -> Swings {
        let h = seconds(h);
        let cap_step = |swing: Swing, cap: &Charge| {
            let i = [last.get(cap.branch), next.get(cap.branch)];
            swing.step(cap.volts(next), i, h, cap.farad)
        };
        let flux_step = |swing: FluxSwing, flux: &Flux| {
            let v = [flux.volts(last), flux.volts(next)];
            FluxSwing {
                amps: swing.amps.step(next.get(flux.branch), v, h, flux.henry),
                volts: swing.volts.within(v[1]),
            }
        };
        let reverses = self.reverses(charges, last, next);
        match self {
            Segment::Start { span } => Swings {
                caps: (charges.caps.iter())
                    .map(|cap| cap_step(Swing::start(cap.volts(last), *span), cap))
                    .collect(),
                fluxes: (charges.fluxes.iter())
                    .map(|flux| {
                        let v = flux.volts(last);
                        let start = FluxSwing {
                            amps: Swing::start(last.get(flux.branch), *span / flux.ohm),
                            volts: Swing::start(v, v.abs().max(*span)),
                        };
                        flux_step(start, flux)
                    })
                    .collect(),
                reverses,
            },
            Segment::Smooth { swings, .. } => Swings {
                caps: (swings.caps.iter().zip(&charges.caps))
                    .map(|(swing, cap)| cap_step(*swing, cap))
                    .collect(),
                fluxes: (swings.fluxes.iter().zip(&charges.fluxes))
                    .map(|(swing, flux)| flux_step(*swing, flux))
                    .collect(),
                reverses,
            },
        }
    }

    /// Whether each store's slope reversed on this step. The scale is the largest of the three
    /// samples, not the larger change. A segment's first step has no earlier slope.
    fn reverses(&self, charges: &Charges, last: &Solution, next: &Solution) -> Vec<bool> {
        let reversed = |a: f64, b: f64, c: f64, floor: f64| {
            let (d0, d1) = (b - a, c - b);
            let tol = RELTOL * a.abs().max(b.abs()).max(c.abs()) + floor;
            d0 * d1 < 0.0 && d0.abs() > tol && d1.abs() > tol
        };
        match self {
            Segment::Start { .. } => vec![false; charges.caps.len() + charges.fluxes.len()],
            Segment::Smooth {
                before: (_, before),
                ..
            } => charges
                .caps
                .iter()
                .map(|cap| {
                    reversed(
                        before.get(cap.branch),
                        last.get(cap.branch),
                        next.get(cap.branch),
                        ABSTOL,
                    )
                })
                .chain(charges.fluxes.iter().map(|flux| {
                    reversed(
                        flux.volts(before),
                        flux.volts(last),
                        flux.volts(next),
                        VNTOL,
                    )
                }))
                .collect(),
        }
    }

    /// The same point and step of `h` ps, with `more` stiff too.
    pub(crate) fn stiffen(self, more: &[Store], h: u64) -> Segment {
        match self {
            // Every store takes backward Euler on the first step, so none turns stiff on it.
            Segment::Start { span } => Segment::Start { span },
            Segment::Smooth {
                before,
                stiff,
                swings,
                ringing,
                reversals,
            } => Segment::Smooth {
                before,
                stiff: stiff.with(more, h),
                swings,
                ringing,
                reversals,
            },
        }
    }

    /// The method of every store that is not stiff.
    pub(crate) fn method(&self) -> Method {
        match self {
            Segment::Start { .. } => Method::BackwardEuler,
            Segment::Smooth { .. } => Method::Trapezoidal,
        }
    }

    /// The stiff stores on backward Euler this step.
    pub(crate) fn stiff(&self) -> &[Store] {
        match self {
            Segment::Start { .. } => &[],
            Segment::Smooth { stiff, .. } => stiff.stores(),
        }
    }

    /// The stiff stores this step tries on the trapezoidal rule.
    fn tried(&self) -> &[Store] {
        match self {
            Segment::Smooth {
                stiff: Stiff::Trying { stores, .. },
                ..
            } => stores,
            _ => &[],
        }
    }

    fn method_of(&self, store: Store) -> Method {
        match self {
            Segment::Smooth { stiff, .. } => method_on(stiff.stores(), self.method(), store),
            Segment::Start { .. } => Method::BackwardEuler,
        }
    }
}

/// A step's local truncation error.
#[derive(Debug)]
pub(crate) struct Error {
    /// The worst store's error over its tolerance. At most 1 accepts.
    pub(crate) ratio: f64,
    /// That store's method, which sets how the next step grows or shrinks.
    pub(crate) method: Method,
    /// Each store's swing with this step folded in. `advance` stores it on accept.
    pub(crate) swings: Swings,
    /// The trapezoidal stores over Newton's tolerance, ascending.
    over: Vec<Store>,
    /// The worst swing-ratio of the stores the step tried on the trapezoidal rule. 0 when it
    /// tried none. A try stays when this is at most 1.
    tried: f64,
    /// Trapezoidal stores over Newton's tolerance whose slope has reversed on
    /// `REVERSAL_RUN` steps running, the candidate included.
    ringing: Vec<Store>,
}

/// What a step does once its error is known.
#[derive(Debug)]
pub(crate) enum Verdict {
    Accept,
    /// Retry it shorter, from the same point.
    Reject,
    /// Retry the same step from the same point, with these stores stiff too, and the ones it
    /// tried back on backward Euler.
    Stiffen(Vec<Store>),
}

impl Error {
    /// A step of `h` ps. A step of 1 ps over its tolerance is accepted whatever its error,
    /// unless a trapezoidal store is over Newton's tolerance there, which makes it stiff.
    /// A trapezoidal store also turns stiff at any longer step when its error is over Newton's
    /// tolerance and its slope has reversed on `REVERSAL_RUN` steps running. A step that tried
    /// stiff stores keeps them on the trapezoidal rule when their error is within the swing
    /// tolerance every trapezoidal store has. A ring on another store during a try sends the
    /// tried stores back to backward Euler, as a failed try does.
    pub(crate) fn verdict(&self, h: u64) -> Verdict {
        if self.tried > 1.0 {
            return Verdict::Stiffen(Vec::new());
        }
        if !(h == 1 && !self.over.is_empty()) && !self.ringing.is_empty() {
            return Verdict::Stiffen(self.ringing.clone());
        }
        match (h, self.over.is_empty(), self.ratio <= 1.0) {
            (1, false, _) => Verdict::Stiffen(self.over.clone()),
            (1, true, _) | (_, _, true) => Verdict::Accept,
            (_, _, false) => Verdict::Reject,
        }
    }
}

/// One store's error on one step.
struct Judged {
    store: Store,
    method: Method,
    /// Over the tolerance the step is held to.
    ratio: f64,
    /// Over Newton's tolerance.
    newton: f64,
}

/// The error of a step of `h` ps from `(at, last)` to `next`, each store on its method in
/// `segment`. A capacitor's voltage is held to `SHARE` of its swing plus `VNTOL`, or plus the
/// larger of `VNTOL` and `h * ABSTOL / C` on a step whose change in current reverses the one
/// before. An inductor's current is held to `SHARE` of its swing plus the larger of `ABSTOL`
/// and `h * VNTOL / L`, and the voltage that error implies, `L * error / h`, to `SHARE` of the
/// voltage's swing plus `VNTOL`. A stiff store keeps Newton's tolerance.
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
    // The state's error from its slope times `size`, read at the point before the last, the
    // last and the candidate.
    let truncation = |method: Method, slope: &dyn Fn(&Solution) -> f64, size: f64| {
        let (d0, d1) = (slope(last), slope(next));
        match (method, segment) {
            (
                Method::Trapezoidal,
                Segment::Smooth {
                    before: (then, before),
                    ..
                },
            ) => {
                let hp = seconds(at.as_ps() - then.as_ps());
                let dd2 = ((d1 - d0) / h - (d0 - slope(before)) / hp) / (h + hp);
                h * h * h * dd2.abs() / (6.0 * size)
            }
            _ => h / 2.0 * (d1 - d0).abs() / size,
        }
    };
    // A stiff store's backward Euler error accumulates with one sign, so it keeps Newton's
    // tolerance. The swing's share holds only for a trapezoidal segment, and for the one
    // backward Euler step that starts a segment.
    let stiff = |method: Method| {
        matches!(
            (segment, method),
            (Segment::Smooth { .. }, Method::BackwardEuler)
        )
    };
    let caps = charges.caps.iter().zip(&swings.caps).map(|(cap, swing)| {
        let store = Store::Charge(cap.index);
        let method = segment.method_of(store);
        let error = truncation(method, &|x: &Solution| x.get(cap.branch), cap.farad);
        let level = cap.volts(last).abs().max(cap.volts(next).abs());
        let newton = error / (RELTOL * level + VNTOL);
        let ratio = match stiff(method) {
            true => newton,
            false => {
                // A current that flips each step is the trapezoidal rule's own mode, which moves
                // no charge. Newton resolves that current to `ABSTOL` at best, and over the step
                // it moves the voltage by `h * ABSTOL / C`.
                let current = |x: &Solution| x.get(cap.branch);
                let flips = match segment {
                    Segment::Smooth {
                        before: (_, before),
                        ..
                    } => (current(next) - current(last)) * (current(last) - current(before)) < 0.0,
                    Segment::Start { .. } => false,
                };
                let floor = match flips {
                    true => VNTOL.max(h * ABSTOL / cap.farad),
                    false => VNTOL,
                };
                error / (SHARE * swing.size() + floor)
            }
        };
        Judged {
            store,
            method,
            ratio,
            newton,
        }
    });
    let fluxes = charges
        .fluxes
        .iter()
        .zip(&swings.fluxes)
        .map(|(flux, swing)| {
            let store = Store::Flux(flux.index);
            let method = segment.method_of(store);
            let error = truncation(method, &|x: &Solution| flux.volts(x), flux.henry);
            let level = last.get(flux.branch).abs().max(next.get(flux.branch).abs());
            let newton = error / (RELTOL * level + ABSTOL);
            let ratio = match stiff(method) {
                true => newton,
                false => {
                    // Newton resolves the voltage across the inductance to `VNTOL` at best, and
                    // over the step that voltage moves the current by `h * VNTOL / L`.
                    let floor = ABSTOL.max(h * VNTOL / flux.henry);
                    let amps = error / (SHARE * swing.amps.size() + floor);
                    let volts = flux.henry * error / h / (SHARE * swing.volts.size() + VNTOL);
                    amps.max(volts)
                }
            };
            Judged {
                store,
                method,
                ratio,
                newton,
            }
        });
    let prior = match segment {
        Segment::Smooth { reversals, .. } => reversals.as_slice(),
        Segment::Start { .. } => &[],
    };
    let mut ratio_worst = 0.0;
    let mut method = segment.method();
    let mut over = Vec::new();
    let mut tried = 0.0f64;
    let mut ringing = Vec::new();
    for (k, judged) in caps.chain(fluxes).enumerate() {
        if judged.newton > 1.0 && judged.method == Method::Trapezoidal {
            over.push(judged.store);
        }
        if segment.tried().binary_search(&judged.store).is_ok() {
            tried = tried.max(judged.ratio);
        }
        let run = if swings.reverses[k] {
            prior.get(k).copied().unwrap_or(0) + 1
        } else {
            0
        };
        if judged.method == Method::Trapezoidal && run >= REVERSAL_RUN && judged.newton > 1.0 {
            ringing.push(judged.store);
        }
        if judged.ratio > ratio_worst {
            (ratio_worst, method) = (judged.ratio, judged.method);
        }
    }
    Error {
        ratio: ratio_worst,
        method,
        swings,
        over,
        tried,
        ringing,
    }
}

/// The jump from `from` to `to`, or 0 when it is within Newton's tolerance on that row.
/// `floor` is that tolerance's absolute part.
fn resolved(from: f64, to: f64, floor: f64) -> f64 {
    let jump = (to - from).abs();
    if jump <= RELTOL * from.abs().max(to.abs()) + floor {
        0.0
    } else {
        jump
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
            fluxes: Vec::new(),
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
            fluxes: Vec::new(),
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

    fn ix(index: usize) -> Store {
        Store::Charge(ChargeIndex::new(index))
    }

    /// A trapezoidal segment from 0 V and 0 A at t = 0, with `stiff` stiff.
    fn smooth(stiff: &[Store]) -> Segment {
        let last = Solution::zeros(3);
        advanced(Segment::start(&two(1e-9), &last), Time::ZERO, last, 1).stiffen(stiff, 1)
    }

    /// A trapezoidal segment on one capacitor whose voltage has covered `lo` to `hi`, with
    /// `before` the point before the last.
    fn swung(before: (Time, Solution), lo: f64, hi: f64) -> Segment {
        Segment::Smooth {
            before,
            stiff: Stiff::NONE,
            swings: Swings {
                caps: vec![Swing { lo, hi, ahead: 0.0 }],
                fluxes: Vec::new(),
                reverses: vec![false],
            },
            ringing: Ringing {
                caps: vec![0.0],
                fluxes: Vec::new(),
            },
            reversals: vec![0],
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
            fluxes: Vec::new(),
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
            fluxes: Vec::new(),
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
    fn a_try_stays_within_its_swing_tolerance_and_a_failed_one_waits_for_twice_the_step() {
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
            step(0.0, 0.5 / per_amp).verdict(RETRY),
            Verdict::Accept
        ));
        assert!(matches!(
            step(0.5 / per_amp, 0.5 / per_amp).verdict(RETRY),
            Verdict::Accept
        ));
        let failed = step(0.0, 2.0 / per_amp).verdict(RETRY);
        assert!(matches!(&failed, Verdict::Stiffen(parts) if parts.is_empty()));
        let held = trying().stiffen(&[], RETRY + 44);
        assert_eq!(held.stiff(), [ix(1)]);
        assert_eq!(held.method_of(ix(0)), Method::Trapezoidal);
        let short = advanced(held, at, Solution::zeros(3), 2 * (RETRY + 44) - 1);
        assert_eq!(short.stiff(), [ix(1)]);
        let again = advanced(short, at, Solution::zeros(3), 2 * (RETRY + 44));
        assert_eq!(again.tried(), [ix(1)]);
    }

    /// A try accepted short of its pass length has not shown that the fast mode died. The next
    /// step still tries it, and an error over the swing tolerance puts it back on backward
    /// Euler, while the slow capacitor beside it keeps the trapezoidal rule.
    #[test]
    fn a_try_cut_short_of_its_step_tries_again() {
        let last = Solution::from(vec![1.0, 0.0, 0.0]);
        let (cut, h, farad) = (40, RETRY, 1e-9);
        let segment = advanced(trying(), Time::from_ps(RETRY + 1), last.clone(), cut);
        assert_eq!(segment.tried(), [ix(1)]);
        let at = Time::from_ps(RETRY + 1 + cut);
        let per_amp = error(
            &two(farad),
            &segment,
            at,
            &last,
            h,
            &Solution::from(vec![1.0, 0.0, 1.0]),
        )
        .tried;
        let next = Solution::from(vec![1.0, 0.0, 2.0 / per_amp]);
        let got = error(&two(farad), &segment, at, &last, h, &next);
        assert!(
            matches!(got.verdict(h), Verdict::Stiffen(ref parts) if parts.is_empty()),
            "{got:?}"
        );
        let retried = segment.stiffen(&[], h);
        assert_eq!(retried.method_of(ix(0)), Method::Trapezoidal);
        assert_eq!(retried.method_of(ix(1)), Method::BackwardEuler);
    }

    /// Three slope reversals over Newton's tolerance at a 1 us step turn the store stiff.
    /// Two reversals do not, and neither do three whose error is under Newton's tolerance.
    /// The swing on these cases is 1 V, so Newton's tolerance is the tighter one.
    #[test]
    fn three_reversals_over_newton_at_1_us_turn_stiff() {
        let h = 1_000_000;
        let charges = one(1e-6);
        let judge = |before: f64, last: f64, next: f64, count: u32| {
            let mut segment = swung((Time::from_ps(h), point(1.0, before)), 0.0, 1.0);
            let Segment::Smooth { reversals, .. } = &mut segment else {
                panic!("a swung segment is smooth");
            };
            reversals[0] = count;
            error(
                &charges,
                &segment,
                Time::from_ps(2 * h),
                &point(1.0, last),
                h,
                &point(1.0, next),
            )
        };
        let over = judge(1.5e-5, -1.5e-5, 1.5e-5, 2);
        assert!(over.swings.reverses[0], "{over:?}");
        assert!(over.ratio <= 1.0, "{over:?}");
        assert!(matches!(over.verdict(h), Verdict::Stiffen(stores) if stores == [ix(0)]));
        let two = judge(1.5e-5, -1.5e-5, 1.5e-5, 1);
        assert!(matches!(two.verdict(h), Verdict::Accept), "{two:?}");
        let under = judge(1e-9, -1e-9, 1e-9, 2);
        assert!(under.swings.reverses[0], "{under:?}");
        assert!(matches!(under.verdict(h), Verdict::Accept), "{under:?}");

        let currents = [0.0, 3e-5, -3e-5, 3e-5];
        let mut segment = Segment::start(&charges, &point(1.0, currents[0]));
        let mut at = Time::ZERO;
        for pair in currents.windows(2) {
            let next = point(1.0, pair[1]);
            segment = accept(segment, &charges, at, point(1.0, pair[0]), h, &next);
            at = Time::from_ps(at.as_ps() + h);
        }
        let Segment::Smooth { reversals, .. } = &segment else {
            panic!("accepted steps make a smooth segment");
        };
        assert_eq!(reversals, &[2]);
    }

    /// Steps shorter than `RETRY` still try once the hold has lasted four times `RETRY`.
    /// A store that joins the hold does not restart that clock.
    #[test]
    fn a_hold_tries_after_four_times_its_wait_and_a_joiner_keeps_the_clock() {
        let step = RETRY / 4;
        let mut segment = smooth(&[ix(1)]);
        let mut at = Time::from_ps(1);
        for n in 0..15 {
            segment = advanced(segment, at, Solution::zeros(3), step);
            at = Time::from_ps(at.as_ps() + step);
            assert_eq!(segment.stiff(), [ix(1)], "held at step {n}");
            assert!(segment.tried().is_empty(), "held at step {n}");
        }
        let trying = advanced(segment, at, Solution::zeros(3), step);
        assert_eq!(trying.tried(), [ix(1)]);

        let mut segment = smooth(&[ix(1)]);
        let mut at = Time::from_ps(1);
        for _ in 0..12 {
            segment = advanced(segment, at, Solution::zeros(3), step);
            at = Time::from_ps(at.as_ps() + step);
        }
        segment = segment.stiffen(&[ix(0)], step);
        for n in 0..3 {
            segment = advanced(segment, at, Solution::zeros(3), step);
            at = Time::from_ps(at.as_ps() + step);
            assert_eq!(segment.stiff(), [ix(0), ix(1)], "joined at step {n}");
        }
        let trying = advanced(segment, at, Solution::zeros(3), step);
        assert_eq!(trying.tried(), [ix(0), ix(1)]);
    }

    /// An empty hold adds every accepted step. The first store keeps that time, so once the
    /// clock has reached four waits the next accepted step tries it.
    #[test]
    fn an_empty_hold_carries_its_clock_to_the_first_store() {
        let step = RETRY / 4;
        let last = Solution::zeros(3);
        let mut segment = advanced(
            Segment::start(&two(1e-9), &last),
            Time::ZERO,
            last.clone(),
            1,
        );
        let mut at = Time::from_ps(1);
        for _ in 0..15 {
            segment = advanced(segment, at, last.clone(), step);
            at = Time::from_ps(at.as_ps() + step);
            assert!(segment.stiff().is_empty());
            assert!(segment.tried().is_empty());
        }
        segment = segment.stiffen(&[ix(1)], step);
        assert_eq!(segment.stiff(), [ix(1)]);
        let trying = advanced(segment, at, last, step);
        assert_eq!(trying.tried(), [ix(1)]);
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
            fluxes: Vec::new(),
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
        assert_eq!(swings.caps[0].lo, first.get(0));
        let heading = 0.02 * 1e-5 / 22e-6;
        assert!(
            (swings.caps[0].ahead - heading).abs() < 1e-6 * heading,
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
        assert_eq!(swings.caps[0].size(), 1.0);
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
            ahead.push(swings.caps[0].ahead);
        }
        assert_eq!(ahead[0], 1.0);
        assert!((ahead[1] - 0.01).abs() < 1e-12, "{ahead:?}");
        assert_eq!(ahead[2], ahead[1]);
    }

    /// A resistor, then two capacitors. The resistor owns no charge, so each capacitor's
    /// part index differs from its charge index.
    fn split() -> (orbe_engine_core::netlist::Netlist, Layout) {
        let netlist = orbe_engine_core::netlist::parse(
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
            assert_eq!(Store::Charge(cap.index), ix(index));
            assert_ne!(ChargeIndex::new(id.index()), cap.index);
        }
    }

    /// A capacitor's charge is on its pins. A BJT's two are on its internal nodes, `CJE` from
    /// `b'` to `e'` and `CJC` from `b'` to `c'`. An `nmos` adds three on its pins, `CGS`, `CGD`
    /// and `CDS`, each on its own branch.
    #[test]
    fn charges_are_the_parts_capacitors() {
        let netlist = orbe_engine_core::netlist::parse(
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
                .part_id(&orbe_engine_core::name::Name::new(name).unwrap())
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
        let indices: Vec<_> = charges
            .caps
            .iter()
            .map(|charge| Store::Charge(charge.index))
            .collect();
        assert_eq!(indices, [ix(0), ix(1), ix(2), ix(3), ix(4), ix(5)]);
    }

    #[test]
    #[should_panic(expected = "a part's capacitors and inductors exceed its branches")]
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
            .part_id(&orbe_engine_core::name::Name::new("n1").unwrap())
            .unwrap();
        layout.branch_row(id, 1);
    }

    #[test]
    #[should_panic(expected = "charge 1 is past this part's charges")]
    fn a_charge_past_the_reservation_panics() {
        let (netlist, layout) = split();
        let id = netlist
            .part_id(&orbe_engine_core::name::Name::new("n1").unwrap())
            .unwrap();
        layout.charge_index(id, 1);
    }

    /// `c_n` owns two capacitors. The second sits on `c_n`'s internal nodes. `d_o` is the
    /// next part's capacitor, so a charge cursor that advances by one collides with it.
    fn two_internal() -> (
        orbe_engine_core::netlist::Netlist,
        element::Pair,
        PartId,
        PartId,
    ) {
        use orbe_engine_core::netlist::NetId;

        let netlist = orbe_engine_core::netlist::parse(
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
                    .part_id(&orbe_engine_core::name::Name::new(name).unwrap())
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
                    element::StoreEnd::Net(net_a),
                    element::StoreEnd::Net(NetId::GROUND),
                ],
                [
                    element::StoreEnd::Internal(0),
                    element::StoreEnd::Internal(1),
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
            .part_id(&orbe_engine_core::name::Name::new("f_s").unwrap())
            .unwrap();
        let gs = netlist
            .part_id(&orbe_engine_core::name::Name::new("g_s").unwrap())
            .unwrap();
        let layout = Layout::with_stand_in(&netlist, Vec::new(), cn, &pair);
        let owned: Vec<_> = element::charges(&pair)
            .into_iter()
            .map(|cap| (cn, cap))
            .collect();
        let charges = Charges::from_owned(&owned, &[], &layout, netlist.nets().len() - 1);
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
        assert_eq!(Store::Charge(layout.charge_index(cn, 0)), ix(0));
        assert_eq!(Store::Charge(layout.charge_index(cn, 1)), ix(1));
        assert_eq!(Store::Charge(layout.charge_index(later, 0)), ix(2));
        assert_ne!(ix(cn.index()), Store::Charge(layout.charge_index(cn, 1)));
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

        let solved = |stiff: &[Store], restart: bool| -> [f64; 2] {
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
        let judged = Charges::from_owned(&judged_owned, &[], &layout, netlist.nets().len() - 1);
        let zeros = Solution::zeros(layout.size());
        let smooth = accept(
            Segment::start(&judged, &zeros),
            &judged,
            Time::ZERO,
            zeros.clone(),
            1,
            &zeros,
        )
        .stiffen(&[Store::Charge(layout.charge_index(cn, 1))], 1);
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

    /// One inductor of `henry` from row 0 to ground behind `ohm`, its current in row 1.
    fn coil(henry: f64, ohm: f64) -> Charges {
        Charges {
            caps: Vec::new(),
            fluxes: vec![Flux {
                index: FluxIndex::new(0),
                branch: 1,
                ends: [Node::Row(0), Node::Ground],
                henry,
                ohm,
            }],
            nets: 1,
        }
    }

    fn fx(index: usize) -> Store {
        Store::Flux(FluxIndex::new(index))
    }

    /// A trapezoidal segment on one inductor whose current has covered `amps` and whose voltage
    /// has covered `volts`, with `before` the point before the last.
    fn coiled(before: (Time, Solution), amps: [f64; 2], volts: [f64; 2]) -> Segment {
        let range = |[lo, hi]: [f64; 2]| Swing { lo, hi, ahead: 0.0 };
        Segment::Smooth {
            before,
            stiff: Stiff::NONE,
            swings: Swings {
                caps: Vec::new(),
                fluxes: vec![FluxSwing {
                    amps: range(amps),
                    volts: range(volts),
                }],
                reverses: vec![false],
            },
            ringing: Ringing {
                caps: Vec::new(),
                fluxes: vec![0.0],
            },
            reversals: vec![0],
        }
    }

    #[test]
    fn backward_euler_is_exact_for_a_voltage_ramp() {
        // v = k*t across 0.5 H gives i'' = k/L, so the error of a 1 us step is h^2/2 * k/L.
        let (k, henry, h) = (2.0, 0.5, 1e-6);
        let exact = h * h / 2.0 * k / henry;
        let charges = coil(henry, 70.0);
        let last = point(0.0, 1e-3);
        let next = point(k * h, 1e-3 + exact);
        // A stiff inductor keeps Newton's tolerance.
        let zeros = Solution::zeros(2);
        let stiff = accept(
            Segment::start(&charges, &zeros),
            &charges,
            Time::ZERO,
            zeros.clone(),
            1,
            &zeros,
        )
        .stiffen(&[fx(0)], 1);
        let got = error(&charges, &stiff, Time::from_ps(1), &last, 1_000_000, &next);
        assert_eq!(got.method, Method::BackwardEuler);
        let want = exact / (RELTOL * (1e-3 + exact) + ABSTOL);
        assert!((got.ratio - want).abs() <= 1e-12 * want, "{got:?}");
        // From no voltage the current is heading nowhere yet, so the first step's current swing
        // is its own move. The voltage that error implies is half the step's move.
        let first = Segment::start(&charges, &last);
        let got = error(&charges, &first, Time::ZERO, &last, 1_000_000, &next);
        assert_eq!(got.method, Method::BackwardEuler);
        let amps = exact / (SHARE * exact + ABSTOL);
        let volts = henry * exact / h / (SHARE * k * h + VNTOL);
        let want = amps.max(volts);
        assert!((got.ratio - want).abs() <= 1e-9 * want, "{got:?}");
    }

    #[test]
    fn trapezoidal_is_exact_for_a_quadratic_voltage_on_an_uneven_grid() {
        // v = k*t^2 has DD2 = k on any grid, and i''' = 2k/L.
        let (k, henry) = (3e9, 1e-3);
        let volts = |ps: u64| k * seconds(ps) * seconds(ps);
        let segment = coiled(
            (Time::from_ps(1_000), point(volts(1_000), 0.5)),
            [0.0, 1.0],
            [0.0, 1.0],
        );
        let last = point(volts(4_000), 0.5);
        let next = point(volts(9_000), 0.5);
        let got = error(
            &coil(henry, 70.0),
            &segment,
            Time::from_ps(4_000),
            &last,
            5_000,
            &next,
        );
        let h = seconds(5_000);
        let exact = h * h * h / 12.0 * (2.0 * k / henry);
        let amps = exact / (SHARE + ABSTOL);
        let implied = henry * exact / h / (SHARE + VNTOL);
        assert_eq!(got.method, Method::Trapezoidal);
        let want = amps.max(implied);
        assert!((got.ratio - want).abs() <= 1e-9 * want, "{got:?}");
    }

    /// A coil behind an open switch rings on the trapezoidal rule, its voltage flipping sign
    /// each step. The current's error is under its tolerance, and the voltage it implies is not.
    #[test]
    fn the_voltage_ratio_rejects_ringing() {
        let (henry, swing, a, h) = (0.5, 5.0 / 70.0, 5e-3, 50_000_000);
        let segment = coiled((Time::ZERO, point(a, 0.01)), [0.0, swing], [0.0, 5.0]);
        let last = point(-a, 0.01);
        let next = point(a, 0.01);
        let got = error(
            &coil(henry, 70.0),
            &segment,
            Time::from_ps(h),
            &last,
            h,
            &next,
        );
        // DD2 is `2a / h^2`, so the error is `h * a / 3L`, and the voltage it implies `a / 3`.
        let error = seconds(h) * a / (3.0 * henry);
        let amps = error / (SHARE * swing + ABSTOL);
        assert!(amps < 1.0, "{amps}");
        let want = (a / 3.0) / (SHARE * 5.0 + VNTOL);
        assert!((got.ratio - want).abs() <= 1e-6 * want, "{got:?}");
        assert!(matches!(got.verdict(h), Verdict::Reject), "{got:?}");
    }

    /// One inductor of `henry` and 70 ohm from row 1 to ground, its current in row 2, and one
    /// net, row 0.
    fn behind_a_net(henry: f64) -> Charges {
        Charges {
            fluxes: vec![Flux {
                ends: [Node::Row(1), Node::Ground],
                branch: 2,
                ..coil(henry, 70.0).fluxes.remove(0)
            }],
            nets: 1,
            ..coil(henry, 70.0)
        }
    }

    /// A segment's first step holds the voltage its current's error implies to a share of the
    /// voltage's reach, the larger of its distance to 0 V and the circuit's span. A coil
    /// switched off from 5 V has its 5 V on the inductance and no net away from 0 V. A coil at
    /// 0 V beside a 5 V net can move as far as the span.
    #[test]
    fn a_coil_s_first_step_reads_its_voltage_reach() {
        let charges = behind_a_net(0.505);
        // 2^-20 V, so the move off 5 V is exact.
        let (amps, h, dv) = (5.0 / 70.0, 1_000, 9.5367431640625e-7);
        for (net, coil) in [(0.0, -5.0), (5.0, 0.0)] {
            let last = Solution::from(vec![net, coil, amps]);
            let next = Solution::from(vec![net, coil - dv, amps]);
            let first = Segment::start(&charges, &last);
            let got = error(&charges, &first, Time::ZERO, &last, h, &next);
            let want = (dv / 2.0) / (SHARE * 5.0 + VNTOL);
            assert!((got.ratio - want).abs() <= 1e-9 * want, "net {net} {got:?}");
        }
    }

    /// The first step caps how far the current reads it is heading at the span over the series
    /// resistance. A 1 uH coil at 0.1 V beside a 5 V net, its voltage moving by 0.1 mV over a
    /// 100 ns step, reads 10 A ahead, and 5 V can drive 71 mA through 70 ohm.
    #[test]
    fn a_coil_s_first_step_caps_its_current_s_reach_at_the_span_over_its_resistance() {
        let (henry, h, dv) = (1e-6, 100_000, 1e-4);
        let charges = behind_a_net(henry);
        let last = Solution::from(vec![5.0, 0.1, 0.0]);
        let next = Solution::from(vec![5.0, 0.1 - dv, seconds(h) * (0.1 - dv) / henry]);
        let got = error(
            &charges,
            &Segment::start(&charges, &last),
            Time::ZERO,
            &last,
            h,
            &next,
        );
        let error = seconds(h) / 2.0 * dv / henry;
        let floor = seconds(h) * VNTOL / henry;
        let want = error / (SHARE * 5.0 / 70.0 + floor);
        assert!((got.ratio - want).abs() <= 1e-9 * want, "{got:?}");
    }

    /// After a restart solve that held it, an IRF520's 34 pF gate-drain capacitor sits at
    /// 4.778 V with 0.675 pA of roundoff in its current, and the trapezoidal rule flips that
    /// current by `a` each way every step. The ring reads as `h * a / 3C`, which grows with the
    /// step, so against 1 nV it would cap the step at 151 ns. Against `h * ABSTOL / C` it
    /// reads `a / 3 pA` at any step.
    #[test]
    fn a_ring_under_a_picoampere_does_not_hold_a_capacitor_s_step() {
        let (farad, a, volts) = (34e-12, 6.75e-13, 4.778);
        for h in [1_000_000, 1_000_000_000] {
            let segment = swung((Time::ZERO, point(volts, a)), volts, volts);
            let got = error(
                &one(farad),
                &segment,
                Time::from_ps(h),
                &point(volts, -a),
                h,
                &point(volts, a),
            );
            let want = a / (3.0 * ABSTOL);
            assert!((got.ratio - want).abs() <= 1e-9 * want, "{h} ps {got:?}");
            assert!(matches!(got.verdict(h), Verdict::Accept), "{h} ps {got:?}");
        }
    }

    /// 1 pF charged by a current that grows from 0 to 0.1 pA to 0.3 pA over two 1 us steps. Its
    /// change does not reverse, so the curve is real, and its error, `h * 0.1 pA / 12C`, 8.3 nV,
    /// stays held to 1 nV.
    #[test]
    fn a_picoampere_curve_keeps_the_nanovolt_floor() {
        let (farad, a, h) = (1e-12, 1e-13, 1_000_000);
        let segment = swung((Time::ZERO, point(0.0, 0.0)), 0.0, 0.0);
        let got = error(
            &one(farad),
            &segment,
            Time::from_ps(h),
            &point(0.0, a),
            h,
            &point(0.0, 3.0 * a),
        );
        let want = seconds(h) * a / (12.0 * farad) / VNTOL;
        assert!((got.ratio - want).abs() <= 1e-9 * want, "{got:?}");
        assert!(matches!(got.verdict(h), Verdict::Reject), "{got:?}");
    }

    /// A 10 uH coil into a net that draws nothing carries the 5 pA gmin lets through, and the
    /// trapezoidal rule leaves the voltage across it flipping by `a` each way every step, 21.92 pV
    /// on an Uno's AVCC coil. That ring reads as `h * a / 3L`, which grows with the step, so
    /// against 1 pA it would cap the step at 1.37 us. Against `h * VNTOL / L` it reads `a / 3 nV`
    /// at any step.
    #[test]
    fn a_ring_under_a_nanovolt_does_not_hold_a_coil_s_step() {
        let (henry, a, amps) = (1e-5, 2.192e-11, 5e-12);
        for h in [2_000_000, 2_000_000_000] {
            let segment = coiled((Time::ZERO, point(a, amps)), [amps, amps], [-a, a]);
            let got = error(
                &coil(henry, 0.5),
                &segment,
                Time::from_ps(h),
                &point(-a, amps),
                h,
                &point(a, amps),
            );
            let want = a / (3.0 * VNTOL);
            assert!((got.ratio - want).abs() <= 1e-9 * want, "{h} ps {got:?}");
            assert!(matches!(got.verdict(h), Verdict::Accept), "{h} ps {got:?}");
        }
    }

    /// Each edge that keeps the segment adds `max_step * |dv| / L` to the coil's sum, for a
    /// jump Newton resolves on the voltage row. A sum over 1e-12 A starts a segment, whose
    /// first step is backward Euler. The row sits at 0 V, so Newton's tolerance is 1 nV. 3 H
    /// and a 1 ms cap put one 1 pA of sum at 3 nV, above that tolerance and under ten times it,
    /// so one edge over the floor restarts and two halves of it restart only on the second.
    #[test]
    fn a_coil_s_edges_start_a_segment_once_their_sum_passes_its_floor() {
        let (henry, max_step) = (3.0, 1_000_000_000);
        let charges = coil(henry, 70.0);
        let smooth = || coiled((Time::ZERO, point(0.0, 0.0)), [0.0, 0.1], [0.0, 0.0]);
        let edge = |segment: Segment, dv: f64| {
            segment.across(&charges, &point(0.0, 0.0), &point(dv, 0.0), max_step)
        };
        let floor = ABSTOL * henry / seconds(max_step);
        let over = floor * 1.01;
        let newton = RELTOL * over.abs() + VNTOL;
        assert!(over > newton && over < 10.0 * newton, "{over} {newton}");
        assert!(over / 2.0 > newton, "{}", over / 2.0);
        assert_eq!(edge(smooth(), over).method(), Method::BackwardEuler);
        assert_eq!(edge(smooth(), -over).method(), Method::BackwardEuler);
        let kept = edge(smooth(), over / 2.0);
        assert_eq!(kept.method(), Method::Trapezoidal);
        assert_eq!(edge(kept, -over / 2.0).method(), Method::BackwardEuler);
    }

    /// An edge that keeps the segment ends a stiff hold. The capacitor jump is 0.5 pA and the
    /// coil jump is 0.5 nV, both under Newton's floor and neither one bitwise zero, so a rule
    /// that clears the hold only for a zero jump fails. The stores take the trapezoidal rule.
    /// A segment start's next step is backward Euler, and this edge does not take that step.
    #[test]
    fn a_kept_edge_puts_the_stiff_capacitors_back_on_the_trapezoidal_rule() {
        let held = smooth(&[ix(1)]);
        assert_eq!(held.stiff(), [ix(1)]);
        let from = Solution::zeros(3);
        let mut to = Solution::zeros(3);
        to.add(2, 0.5 * ABSTOL);
        let kept = held.across(&two(1e-9), &from, &to, 1_000_000);
        assert_ne!(from.get(2), to.get(2));
        assert_eq!(kept.method(), Method::Trapezoidal);
        assert_eq!(kept.stiff(), []);
        assert_eq!(kept.method_of(ix(1)), Method::Trapezoidal);

        let held =
            coiled((Time::ZERO, point(0.0, 0.0)), [0.0, 0.1], [0.0, 0.0]).stiffen(&[fx(0)], 1);
        assert_eq!(held.stiff(), [fx(0)]);
        let from = point(0.0, 0.0);
        let to = point(0.5 * VNTOL, 0.0);
        let kept = held.across(&coil(1.0, 70.0), &from, &to, 1_000);
        assert_ne!(from.get(0), to.get(0));
        assert_eq!(kept.method(), Method::Trapezoidal);
        assert_eq!(kept.stiff(), []);
        assert_eq!(kept.method_of(fx(0)), Method::Trapezoidal);
    }

    /// `resolved` is 0 at or under `RELTOL` times the larger of the two levels, plus the row's
    /// floor, and it is the jump past that. A capacitor row uses `ABSTOL`, 1e-12 A. A coil's
    /// voltage uses `VNTOL`, 1e-9 V. The coil case goes through `across`, so the floor is the
    /// one that call passes.
    #[test]
    fn a_jump_inside_the_larger_level_s_floor_adds_nothing() {
        for floor in [ABSTOL, VNTOL] {
            let under = 0.75 * floor;
            assert_eq!(resolved(0.0, under, floor), 0.0, "under {floor}");
            let over = 1.5 * floor;
            assert_eq!(resolved(0.0, over, floor), over, "over {floor}");
            let from = 1.0;
            let tol_from = RELTOL * from + floor;
            let jump = tol_from * (1.0 + 0.5 * RELTOL);
            let to = from + jump;
            assert!(jump > RELTOL * from.abs() + floor, "{floor}");
            assert!(jump <= RELTOL * to.abs() + floor, "{floor}");
            assert_eq!(resolved(from, to, floor), 0.0, "larger level {floor}");
        }

        let (henry, max_step) = (0.1, 1_000_000_000);
        let charges = coil(henry, 70.0);
        let edge = |dv: f64| {
            coiled((Time::ZERO, point(0.0, 0.0)), [0.0, 0.1], [0.0, 0.0]).across(
                &charges,
                &point(0.0, 0.0),
                &point(dv, 0.0),
                max_step,
            )
        };
        assert_eq!(
            edge(0.75 * VNTOL).method(),
            Method::Trapezoidal,
            "a coil at 0 V under VNTOL keeps the segment"
        );
        assert_eq!(edge(1.5 * VNTOL).method(), Method::BackwardEuler);
    }

    /// `c_m` owns a capacitor from `a` to ground and an inductor from `b` to ground. `d_c` is
    /// the next part's capacitor, and `g_l` a later inductor across `a`. `e_s` and `f_s` hold
    /// `a` and `b`.
    fn shared() -> (
        orbe_engine_core::netlist::Netlist,
        element::Mixed,
        [PartId; 3],
    ) {
        use element::StoreEnd;
        use orbe_engine_core::netlist::NetId;

        let netlist = orbe_engine_core::netlist::parse(
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
                { "name": "c_m", "kind": "resistor", "pins": { "a": "a", "b": "b" },
                  "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
                  "ratings": { "power": null } },
                { "name": "d_c", "kind": "capacitor", "pins": { "a": "a", "b": "gnd" },
                  "capacitance": { "value": 1e-9, "unit": "F", "source": "user" },
                  "initial": null },
                { "name": "e_s", "kind": "voltage_source", "pins": { "p": "a", "n": "gnd" },
                  "voltage": { "value": 2.0, "unit": "V", "source": "user" } },
                { "name": "f_s", "kind": "voltage_source", "pins": { "p": "b", "n": "gnd" },
                  "voltage": { "value": 3.0, "unit": "V", "source": "user" } },
                { "name": "g_l", "kind": "inductor", "pins": { "a": "a", "b": "gnd" },
                  "inductance": { "value": 1.0, "unit": "H", "source": "user" },
                  "series_resistance": { "value": 10.0, "unit": "ohm", "source": "user" } }
              ]
            }"#,
        )
        .unwrap();
        let net = |name: &str| {
            netlist
                .nets()
                .find(|(_, net)| net.as_str() == name)
                .unwrap()
                .0
        };
        let id = |name: &str| {
            netlist
                .part_id(&orbe_engine_core::name::Name::new(name).unwrap())
                .unwrap()
        };
        let mixed = element::Mixed {
            capacitor: [StoreEnd::Net(net("a")), StoreEnd::Net(NetId::GROUND)],
            farad: 1e-6,
            inductor: [StoreEnd::Net(net("b")), StoreEnd::Net(NetId::GROUND)],
            henry: 1e-3,
            ohm: 1.0,
            internal_nodes: 0,
            branches: 2,
        };
        let parts = [id("c_m"), id("d_c"), id("g_l")];
        (netlist, mixed, parts)
    }

    /// The stores `Charges::new` would collect with `mixed` standing in for `part`.
    fn with_stand_in(
        netlist: &orbe_engine_core::netlist::Netlist,
        layout: &Layout,
        part: PartId,
        mixed: &element::Mixed,
    ) -> Charges {
        let (mut caps, mut fluxes) = (Vec::new(), Vec::new());
        for (id, model) in netlist.parts() {
            let (charges, inductors) = if id == part {
                (element::charges(mixed), element::inductors(mixed))
            } else {
                (
                    element::charges(&model.model),
                    element::inductors(&model.model),
                )
            };
            caps.extend(charges.into_iter().map(|cap| (id, cap)));
            fluxes.extend(inductors.into_iter().map(|inductor| (id, inductor)));
        }
        Charges::from_owned(&caps, &fluxes, layout, netlist.nets().len() - 1)
    }

    /// The shared part's inductor is flux 0 and the later one flux 1. Each current sits on a
    /// branch of its own, the shared part's after its capacitor's, apart from the next part's
    /// capacitor.
    #[test]
    fn an_inductor_takes_the_branch_after_its_part_s_capacitors() {
        let (netlist, mixed, [cm, dc, gl]) = shared();
        let layout = Layout::with_stand_in(&netlist, Vec::new(), cm, &mixed);
        let charges = with_stand_in(&netlist, &layout, cm, &mixed);
        let caps: Vec<_> = charges.caps.iter().map(|c| (c.index, c.branch)).collect();
        let fluxes: Vec<_> = charges.fluxes.iter().map(|f| (f.index, f.branch)).collect();
        assert_eq!(
            caps,
            [
                (ChargeIndex::new(0), layout.branch_row(cm, 0)),
                (ChargeIndex::new(1), layout.branch_row(dc, 0)),
            ]
        );
        assert_eq!(
            fluxes,
            [
                (FluxIndex::new(0), layout.branch_row(cm, 1)),
                (FluxIndex::new(1), layout.branch_row(gl, 0)),
            ]
        );
        assert_eq!(layout.branch_row(dc, 0), layout.branch_row(cm, 1) + 1);
        assert_eq!(charges.fluxes[0].ends, [Node::Row(1), Node::Ground]);
        assert_eq!(
            charges.fluxes[1].ends,
            [layout.internal(gl, 0), Node::Ground]
        );
    }

    #[test]
    #[should_panic(expected = "a part's capacitors and inductors exceed its branches")]
    fn a_part_with_a_capacitor_and_an_inductor_on_one_branch_panics() {
        let (netlist, mut mixed, [cm, ..]) = shared();
        mixed.branches = 1;
        Layout::with_stand_in(&netlist, Vec::new(), cm, &mixed);
    }

    #[test]
    #[should_panic(expected = "flux 1 is past this part's inductors")]
    fn a_flux_past_the_reservation_panics() {
        let (netlist, mixed, [cm, ..]) = shared();
        let layout = Layout::with_stand_in(&netlist, Vec::new(), cm, &mixed);
        layout.flux_index(cm, 1);
    }

    /// At 1 ps the shared part's inductor voltage jumps by 1 V, over Newton's tolerance on its
    /// current, while every capacitor's current and the other inductor's voltage stay put. Only
    /// the inductor turns stiff. Its step takes backward Euler and the capacitor on its part
    /// keeps the trapezoidal rule, in the error and in the stamps. A part's index keys nothing.
    #[test]
    fn an_inductor_turns_stiff_alone() {
        use crate::mna::{Stamp, System};
        use crate::newton::{Conditions, History, Integration, Iterate};

        let (netlist, mixed, [cm, ..]) = shared();
        assert_ne!(cm.index(), 0);
        let layout = Layout::with_stand_in(&netlist, Vec::new(), cm, &mixed);
        let charges = with_stand_in(&netlist, &layout, cm, &mixed);
        let zeros = Solution::zeros(layout.size());
        let smooth = accept(
            Segment::start(&charges, &zeros),
            &charges,
            Time::ZERO,
            zeros.clone(),
            1,
            &zeros,
        );
        let mut jumped = vec![0.0; layout.size()];
        jumped[1] = 1.0;
        let got = error(
            &charges,
            &smooth,
            Time::from_ps(1),
            &zeros,
            1,
            &Solution::from(jumped),
        );
        assert!(
            matches!(got.verdict(1), Verdict::Stiffen(ref stores) if stores == &[fx(0)]),
            "{got:?}"
        );
        let stiff = smooth.stiffen(&[fx(0)], 1);
        assert_eq!(stiff.method_of(fx(0)), Method::BackwardEuler);
        assert_eq!(stiff.method_of(ix(0)), Method::Trapezoidal);

        // `a` and `b` move from 1.5 V and 2.5 V to the sources' 2 V and 3 V over 1 ms.
        let mut prev = vec![0.0; layout.size()];
        prev[0] = 1.5;
        prev[1] = 2.5;
        prev[layout.branch_row(cm, 0)] = 1e-4;
        prev[layout.branch_row(cm, 1)] = 4e-4;
        let previous = Solution::from(prev);
        let solved = |stiff: &[Store]| -> [f64; 2] {
            let conditions = Conditions {
                history: History::Step(Integration {
                    method: Method::Trapezoidal,
                    stiff,
                    h_ps: 1_000_000_000,
                    previous: &previous,
                }),
                ..Conditions::NOMINAL
            };
            let iterate = Iterate {
                solution: Solution::zeros(layout.size()),
                junctions: Vec::new(),
            };
            let mut system = System::new(&layout);
            system.reset(&layout, conditions.gmin);
            for (id, part) in netlist.parts() {
                let mut stamp = Stamp::new(&mut system, &layout, id, &iterate, &conditions);
                if id == cm {
                    element::stamp(&mixed, &mut stamp);
                } else {
                    element::stamp(&part.model, &mut stamp);
                }
            }
            let solution = system.solve().expect("the held nets are nonsingular");
            [
                solution.get(layout.branch_row(cm, 0)),
                solution.get(layout.branch_row(cm, 1)),
            ]
        };
        let h = 1e-3;
        let cap = |method| match method {
            Method::BackwardEuler => 1e-6 / h * 0.5,
            Method::Trapezoidal => 2e-6 / h * 0.5 - 1e-4,
        };
        let coil = |method| match method {
            Method::BackwardEuler => 4e-4 + h / 1e-3 * 3.0,
            Method::Trapezoidal => 4e-4 + h / 2e-3 * (3.0 + 2.5),
        };
        let near = |got: [f64; 2], want: [f64; 2]| {
            (got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9
        };
        let (be, tr) = (Method::BackwardEuler, Method::Trapezoidal);
        for (stiff, want) in [
            (vec![], [cap(tr), coil(tr)]),
            (vec![fx(0)], [cap(tr), coil(be)]),
            (vec![ix(0)], [cap(be), coil(tr)]),
            (vec![fx(cm.index())], [cap(tr), coil(tr)]),
            (vec![ix(cm.index())], [cap(tr), coil(tr)]),
        ] {
            let got = solved(&stiff);
            assert!(near(got, want), "{stiff:?}: {got:?} against {want:?}");
        }
    }
}
