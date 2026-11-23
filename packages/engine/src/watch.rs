//! Watches: the levels chips read from the solution. A chip declares its watches, and the
//! loop owns them. It evaluates them after every accepted step, locates a flip to the
//! picosecond, and hands the levels to the chips at that time, so a chip never sees a step.

use std::ops::Range;

use orbe_model::Time;

use crate::mna::{Node, Solution};
use crate::newton::{Converged, Failure};

/// `Σ coefficient * V(node) - offset`. Linear in the solution.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Signal {
    pub(crate) terms: Vec<(Node, f64)>,
    pub(crate) offset: f64,
}

impl Signal {
    pub(crate) fn at(&self, x: &Solution) -> f64 {
        let sum: f64 = self
            .terms
            .iter()
            .map(|&(node, coefficient)| coefficient * x.node(node))
            .sum();
        sum - self.offset
    }
}

/// A level a chip reads. Low turns high when `rise` is above 0, and high turns low when
/// `fall` is at or below 0. Equal forms make a plain comparator.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Watch {
    pub(crate) rise: Signal,
    pub(crate) fall: Signal,
}

impl Watch {
    pub(crate) fn comparator(signal: Signal) -> Watch {
        Watch {
            rise: signal.clone(),
            fall: signal,
        }
    }

    /// The form that decides the level next, from the level `high` in force.
    fn form(&self, high: bool) -> &Signal {
        if high { &self.fall } else { &self.rise }
    }

    fn level(&self, x: &Solution, prior: Option<bool>) -> bool {
        match prior {
            None | Some(false) => self.rise.at(x) > 0.0,
            Some(true) => self.fall.at(x) > 0.0,
        }
    }
}

/// Every watch of a circuit, chips in part order and each chip's in its own order.
pub(crate) struct Watches {
    watches: Vec<Watch>,
    by_chip: Vec<Range<usize>>,
}

/// One level per watch, in `Watches` order. `true` is high.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Levels(Vec<bool>);

impl Levels {
    pub(crate) fn of(&self, range: Range<usize>) -> &[bool] {
        &self.0[range]
    }

    /// These levels, except that each watch whose level differs between `before` and `after`
    /// takes its `after` level.
    pub(crate) fn with_flips(&self, before: &Levels, after: &Levels) -> Levels {
        Levels(
            self.0
                .iter()
                .zip(before.0.iter().zip(&after.0))
                .map(|(&own, (&before, &after))| if before == after { own } else { after })
                .collect(),
        )
    }
}

impl Watches {
    /// Each chip's watches, by chip slot.
    pub(crate) fn new(chips: Vec<Vec<Watch>>) -> Watches {
        let mut watches = Vec::new();
        let mut by_chip = Vec::with_capacity(chips.len());
        for chip in chips {
            let start = watches.len();
            watches.extend(chip);
            by_chip.push(start..watches.len());
        }
        Watches { watches, by_chip }
    }

    /// The levels on `x`. With no `prior`, at t = 0, every watch takes its `rise` form.
    pub(crate) fn levels(&self, x: &Solution, prior: Option<&Levels>) -> Levels {
        Levels(
            self.watches
                .iter()
                .enumerate()
                .map(|(k, watch)| watch.level(x, prior.map(|prior| prior.0[k])))
                .collect(),
        )
    }

    /// The watches of chip `slot`.
    pub(crate) fn of(&self, slot: usize) -> Range<usize> {
        self.by_chip[slot].clone()
    }

    /// Each watch's deciding form on `x`, from the levels `before`.
    fn signals(&self, x: &Solution, before: &Levels) -> Vec<f64> {
        self.watches
            .iter()
            .zip(&before.0)
            .map(|(watch, &high)| watch.form(high).at(x))
            .collect()
    }
}

/// Which end of the bracket a probe moved.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    Lo,
    Hi,
}

/// The first picosecond in `(from, to]` at which some level differs from `before`, and the
/// solve there. `to`'s levels differ. `solve(h)` re-solves one step of `h` ps from the
/// accepted point at `from` with the same method, so every probe is a step of the same
/// integrator.
///
/// Each probe is regula falsi on the flipped watches' forms, the earliest estimate, rounded
/// to the picosecond and kept inside the bracket. After two moves of one end the next probe
/// bisects. It stops at a 1 ps bracket and returns the later end, the first solved time past
/// the threshold. A probe whose Newton stalls is replaced by the bisection point, and a stall
/// there is that probe's failure, at its time.
pub(crate) fn locate(
    watches: &Watches,
    before: &Levels,
    from: (Time, &Solution),
    to: (Time, Converged),
    mut solve: impl FnMut(u64) -> Result<Converged, Failure>,
) -> Result<(Time, Converged), (Time, Failure)> {
    let start = from.0.as_ps();
    let (mut lo, mut hi) = (start, to.0.as_ps());
    let mut at_lo = watches.signals(from.1, before);
    let mut at_hi = watches.signals(&to.1.iterate.solution, before);
    let mut solved = to.1;
    let mut flipped = watches.levels(&solved.iterate.solution, Some(before));
    let mut moves: Option<(End, u32)> = None;
    let mut stalled = false;
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        let probe = match moves {
            _ if stalled => mid,
            Some((_, count)) if count >= 2 => mid,
            _ => falsi(lo, hi, &at_lo, &at_hi, before, &flipped).unwrap_or(mid),
        };
        let converged = match solve(probe - start) {
            Ok(converged) => converged,
            Err(Failure::Stalled(_)) if !stalled && probe != mid => {
                stalled = true;
                continue;
            }
            Err(failure) => return Err((Time::from_ps(probe), failure)),
        };
        stalled = false;
        let levels = watches.levels(&converged.iterate.solution, Some(before));
        let end = if levels == *before {
            lo = probe;
            at_lo = watches.signals(&converged.iterate.solution, before);
            End::Lo
        } else {
            hi = probe;
            at_hi = watches.signals(&converged.iterate.solution, before);
            solved = converged;
            flipped = levels;
            End::Hi
        };
        moves = Some(match moves {
            Some((last, count)) if last == end => (end, count + 1),
            _ => (end, 1),
        });
    }
    Ok((Time::from_ps(hi), solved))
}

/// The earliest linear estimate of a flipped watch's crossing, rounded to the picosecond and
/// strictly inside `(lo, hi)`. `None` when no flipped form moved.
fn falsi(
    lo: u64,
    hi: u64,
    at_lo: &[f64],
    at_hi: &[f64],
    before: &Levels,
    flipped: &Levels,
) -> Option<u64> {
    let width = (hi - lo) as f64;
    let estimate = (0..at_lo.len())
        .filter(|&k| before.0[k] != flipped.0[k] && at_lo[k] != at_hi[k])
        .map(|k| width * at_lo[k] / (at_lo[k] - at_hi[k]))
        .min_by(f64::total_cmp)?;
    let offset = (estimate.round() as u64).clamp(1, hi - lo - 1);
    Some(lo + offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::newton::{Iterate, Offender, Stall};

    fn above(threshold: f64) -> Watch {
        Watch::comparator(Signal {
            terms: vec![(Node::Row(0), 1.0)],
            offset: threshold,
        })
    }

    fn x(volts: f64) -> Solution {
        Solution::from(vec![volts])
    }

    #[test]
    fn a_comparator_is_high_only_strictly_above_its_threshold() {
        let watches = Watches::new(vec![vec![above(1.0)]]);
        assert_eq!(watches.levels(&x(1.0), None), Levels(vec![false]));
        assert_eq!(watches.levels(&x(1.0 + 1e-12), None), Levels(vec![true]));
        let high = Levels(vec![true]);
        assert_eq!(watches.levels(&x(1.0), Some(&high)), Levels(vec![false]));
    }

    #[test]
    fn hysteresis_keeps_the_level_between_the_two_forms() {
        let watch = Watch {
            rise: above(2.0).rise,
            fall: above(1.0).fall,
        };
        let watches = Watches::new(vec![vec![watch]]);
        let (low, high) = (Levels(vec![false]), Levels(vec![true]));
        assert_eq!(watches.levels(&x(1.5), Some(&low)), low);
        assert_eq!(watches.levels(&x(1.5), Some(&high)), high);
        assert_eq!(watches.levels(&x(1.5), None), low);
        assert_eq!(watches.levels(&x(2.5), Some(&low)), high);
        assert_eq!(watches.levels(&x(0.5), Some(&high)), low);
    }

    #[test]
    fn each_chip_owns_a_contiguous_range() {
        let watches = Watches::new(vec![vec![above(0.0), above(1.0)], vec![above(2.0)]]);
        assert_eq!(watches.of(0), 0..2);
        assert_eq!(watches.of(1), 2..3);
        let levels = watches.levels(&x(1.5), None);
        assert_eq!(levels.of(watches.of(0)), [true, true]);
        assert_eq!(levels.of(watches.of(1)), [false]);
    }

    fn converged(volts: f64) -> Converged {
        Converged {
            iterate: Iterate {
                solution: x(volts),
                junctions: Vec::new(),
            },
            iterations: 1,
            moved: Vec::new(),
        }
    }

    /// `locate` over `(0, to]` on the waveform `v`, with every probe's time recorded. The
    /// probes counted in `stalls` stall.
    fn run(
        watches: &Watches,
        v: impl Fn(u64) -> f64,
        to: u64,
        stalls: &[usize],
    ) -> (Result<u64, u64>, Vec<u64>) {
        let before = watches.levels(&x(v(0)), None);
        let mut probes = Vec::new();
        let found = locate(
            watches,
            &before,
            (Time::ZERO, &x(v(0))),
            (Time::from_ps(to), converged(v(to))),
            |h| {
                probes.push(h);
                if stalls.contains(&(probes.len() - 1)) {
                    return Err(Failure::Stalled(Stall {
                        iterations: 50,
                        offender: Offender::Row(0),
                    }));
                }
                Ok(converged(v(h)))
            },
        );
        let found = match found {
            Ok((at, solved)) => {
                assert_eq!(solved.iterate.solution, x(v(at.as_ps())));
                Ok(at.as_ps())
            }
            Err((at, _)) => Err(at.as_ps()),
        };
        (found, probes)
    }

    fn first_past(v: impl Fn(u64) -> f64, past: impl Fn(f64) -> bool) -> u64 {
        (0..).find(|&t| past(v(t))).unwrap()
    }

    #[test]
    fn a_rising_crossing_is_the_first_picosecond_past_its_threshold() {
        let v = |t: u64| {
            let x = t as f64 / 1000.0;
            x * x
        };
        let watches = Watches::new(vec![vec![above(2.0)]]);
        let (found, probes) = run(&watches, v, 10_000, &[]);
        assert_eq!(found, Ok(first_past(v, |v| v > 2.0)));
        assert_eq!(found, Ok(1415));
        // The chord from 0 is far off this curve, and it costs no more than bisection would.
        assert!(probes.len() <= 14, "{probes:?}");
    }

    #[test]
    fn a_nearly_straight_crossing_takes_few_probes() {
        // Bent by 1e-3 of its rise over the step, about as much as a 555 step bends.
        let v = |t: u64| {
            let x = t as f64 * 1e-6;
            1.0 + x + 1e-3 * (x * x)
        };
        let watches = Watches::new(vec![vec![above(1.0123456)]]);
        let (found, probes) = run(&watches, v, 1_000_000, &[]);
        assert_eq!(found, Ok(first_past(v, |v| v > 1.0123456)));
        assert!(probes.len() <= 4, "{probes:?}");
    }

    #[test]
    fn a_falling_crossing_is_the_first_picosecond_at_or_under_its_threshold() {
        let v = |t: u64| 5.0 * libm::exp(-(t as f64) / 3000.0);
        let watches = Watches::new(vec![vec![above(1.0)]]);
        let (found, _) = run(&watches, v, 20_000, &[]);
        assert_eq!(found, Ok(first_past(v, |v| v <= 1.0)));
    }

    #[test]
    fn the_earliest_of_two_flips_wins() {
        let v = |t: u64| t as f64 / 100.0;
        let watches = Watches::new(vec![vec![above(70.0)], vec![above(30.5)]]);
        let (found, _) = run(&watches, v, 10_000, &[]);
        assert_eq!(found, Ok(3051));
    }

    #[test]
    fn a_stuck_end_turns_to_bisection() {
        // Regula falsi on this curve keeps moving the early end by a picosecond or two.
        let v = |t: u64| libm::pow(t as f64 / 1e6, 40.0);
        let watches = Watches::new(vec![vec![above(0.5)]]);
        let (found, probes) = run(&watches, v, 1_000_000, &[]);
        assert_eq!(found, Ok(first_past(v, |v| v > 0.5)));
        // Bisection alone needs 20. A creeping regula falsi would need thousands.
        assert!(probes.len() <= 45, "{} probes", probes.len());
    }

    #[test]
    fn a_stalled_probe_is_replaced_by_the_bisection_point() {
        let v = |t: u64| t as f64 / 1000.0;
        let watches = Watches::new(vec![vec![above(2.5)]]);
        let (clean, probes) = run(&watches, v, 10_000, &[]);
        assert_eq!(clean, Ok(2501));
        let (found, retried) = run(&watches, v, 10_000, &[0]);
        assert_eq!(found, Ok(2501));
        assert_eq!(retried[..2], [probes[0], 5_000]);
        let (failed, _) = run(&watches, v, 10_000, &[0, 1]);
        assert_eq!(failed, Err(5_000));
    }

    #[test]
    fn the_located_point_does_not_flip_again() {
        // Rise above 2, fall at or under 1. Once located, the level holds between the two.
        let watch = Watch {
            rise: above(2.0).rise,
            fall: above(1.0).fall,
        };
        let watches = Watches::new(vec![vec![watch]]);
        let v = |t: u64| if t < 5_000 { t as f64 / 1000.0 } else { 1.5 };
        let (found, _) = run(&watches, v, 4_000, &[]);
        let at = found.unwrap();
        let after = watches.levels(&x(v(at)), None);
        assert_eq!(after, Levels(vec![true]));
        for t in [at, at + 1, 4_000, 9_000] {
            assert_eq!(watches.levels(&x(v(t)), Some(&after)), after, "{t}");
        }
    }
}
